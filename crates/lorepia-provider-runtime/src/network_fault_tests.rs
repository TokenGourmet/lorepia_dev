use std::{
    io::{Read, Write},
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, TcpListener, TcpStream},
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
        mpsc as std_mpsc,
    },
    thread,
    time::Duration as StdDuration,
};

use reqwest::{
    Response, StatusCode,
    header::{AUTHORIZATION, HeaderValue},
};
use rustls::{
    ServerConfig, ServerConnection, StreamOwned,
    pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer},
};
use tokio::{sync::mpsc, time::Instant};
use tokio_util::sync::CancellationToken;

use super::*;
use crate::TokenUsage;
use crate::client::build_loopback_test_client;

const SECRET: &str = "NET-010-SENTINEL-DO-NOT-LOG";
const OPENAI_COMPLETE: &str = concat!(
    "event: response.created\n",
    "data: {\"type\":\"response.created\",\"response\":{\"id\":\"resp_1\"}}\n\n",
    "event: response.output_text.delta\n",
    "data: {\"type\":\"response.output_text.delta\",\"delta\":\"hello\"}\n\n",
    "event: response.completed\n",
    "data: {\"type\":\"response.completed\",\"response\":{\"usage\":{\"input_tokens\":2,\"output_tokens\":1,\"total_tokens\":3}}}\n\n",
);

#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct RequestReceipt {
    method: String,
    path: String,
    header_names: Vec<String>,
    request_bytes: usize,
    saw_secret: bool,
}

struct FaultServer {
    address: SocketAddr,
    hits: Arc<AtomicUsize>,
    receipt: Arc<Mutex<Option<RequestReceipt>>>,
    task: Option<thread::JoinHandle<()>>,
}

impl FaultServer {
    fn spawn<F>(ip: IpAddr, accept_for: StdDuration, handler: F) -> std::io::Result<Self>
    where
        F: FnOnce(TcpStream, Arc<Mutex<Option<RequestReceipt>>>) + Send + 'static,
    {
        let listener = TcpListener::bind(SocketAddr::new(ip, 0))?;
        Self::from_listener(listener, accept_for, handler)
    }

    fn spawn_at<F>(
        address: SocketAddr,
        accept_for: StdDuration,
        handler: F,
    ) -> std::io::Result<Self>
    where
        F: FnOnce(TcpStream, Arc<Mutex<Option<RequestReceipt>>>) + Send + 'static,
    {
        let listener = TcpListener::bind(address)?;
        Self::from_listener(listener, accept_for, handler)
    }

    fn from_listener<F>(
        listener: TcpListener,
        accept_for: StdDuration,
        handler: F,
    ) -> std::io::Result<Self>
    where
        F: FnOnce(TcpStream, Arc<Mutex<Option<RequestReceipt>>>) + Send + 'static,
    {
        listener.set_nonblocking(true)?;
        let address = listener.local_addr()?;
        let hits = Arc::new(AtomicUsize::new(0));
        let receipt = Arc::new(Mutex::new(None));
        let thread_hits = Arc::clone(&hits);
        let thread_receipt = Arc::clone(&receipt);
        let task = thread::spawn(move || {
            let deadline = std::time::Instant::now() + accept_for;
            loop {
                match listener.accept() {
                    Ok((stream, _)) => {
                        stream
                            .set_nonblocking(false)
                            .expect("accepted fault socket must be blocking");
                        stream
                            .set_write_timeout(Some(StdDuration::from_millis(500)))
                            .expect("fault socket write timeout");
                        thread_hits.fetch_add(1, Ordering::SeqCst);
                        handler(stream, thread_receipt);
                        return;
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        if std::time::Instant::now() >= deadline {
                            return;
                        }
                        thread::sleep(StdDuration::from_millis(1));
                    }
                    Err(_) => return,
                }
            }
        });
        Ok(Self {
            address,
            hits,
            receipt,
            task: Some(task),
        })
    }

    fn ipv4<F>(handler: F) -> Self
    where
        F: FnOnce(TcpStream, Arc<Mutex<Option<RequestReceipt>>>) + Send + 'static,
    {
        Self::spawn(
            IpAddr::V4(Ipv4Addr::LOCALHOST),
            StdDuration::from_secs(2),
            handler,
        )
        .expect("IPv4 loopback fault server")
    }

    fn url(&self, scheme: &str) -> String {
        format!("{scheme}://{}/v1/responses", self.address)
    }

    fn hits(&self) -> usize {
        self.hits.load(Ordering::SeqCst)
    }

    fn receipt(&self) -> Option<RequestReceipt> {
        self.receipt.lock().expect("receipt mutex").clone()
    }

    fn wait(mut self) -> (usize, Option<RequestReceipt>) {
        if let Some(task) = self.task.take() {
            let _ = task.join();
        }
        (self.hits(), self.receipt())
    }
}

impl Drop for FaultServer {
    fn drop(&mut self) {
        if let Some(task) = self.task.take() {
            let _ = task.join();
        }
    }
}

fn capture_request(stream: &mut TcpStream, receipt: &Arc<Mutex<Option<RequestReceipt>>>) {
    stream
        .set_read_timeout(Some(StdDuration::from_secs(1)))
        .expect("request read timeout");
    let mut bytes = Vec::with_capacity(4096);
    let mut chunk = [0u8; 1024];
    let mut expected_total = None;
    while bytes.len() <= 64 * 1024 {
        match stream.read(&mut chunk) {
            Ok(0) => break,
            Ok(read) => {
                bytes.extend_from_slice(&chunk[..read]);
                if expected_total.is_none()
                    && let Some(head_end) = bytes
                        .windows(4)
                        .position(|window| window == b"\r\n\r\n")
                        .map(|position| position + 4)
                {
                    let head = String::from_utf8_lossy(&bytes[..head_end]);
                    let body_bytes = head
                        .lines()
                        .filter_map(|line| line.split_once(':'))
                        .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
                        .and_then(|(_, value)| value.trim().parse::<usize>().ok())
                        .unwrap_or(0);
                    expected_total = Some(head_end.saturating_add(body_bytes));
                }
                if expected_total.is_some_and(|expected| bytes.len() >= expected) {
                    break;
                }
            }
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                break;
            }
            Err(_) => break,
        }
    }
    let head = String::from_utf8_lossy(&bytes);
    let mut lines = head.lines();
    let mut request_line = lines.next().unwrap_or_default().split_ascii_whitespace();
    let method = request_line.next().unwrap_or_default().to_owned();
    let path = request_line.next().unwrap_or_default().to_owned();
    let mut header_names = lines
        .take_while(|line| !line.is_empty())
        .filter_map(|line| {
            line.split_once(':')
                .map(|(name, _)| name.to_ascii_lowercase())
        })
        .collect::<Vec<_>>();
    header_names.sort();
    *receipt.lock().expect("receipt mutex") = Some(RequestReceipt {
        method,
        path,
        header_names,
        request_bytes: bytes.len(),
        saw_secret: bytes
            .windows(SECRET.len())
            .any(|window| window == SECRET.as_bytes()),
    });
}

fn write_response(stream: &mut TcpStream, status: &str, headers: &[(&str, &str)], body: &[u8]) {
    let mut head = format!(
        "HTTP/1.1 {status}\r\nConnection: close\r\nContent-Length: {}\r\n",
        body.len()
    );
    for (name, value) in headers {
        head.push_str(name);
        head.push_str(": ");
        head.push_str(value);
        head.push_str("\r\n");
    }
    head.push_str("\r\n");
    stream.write_all(head.as_bytes()).expect("response head");
    stream.write_all(body).expect("response body");
}

fn serve_response(
    status: &'static str,
    headers: Vec<(&'static str, &'static str)>,
    body: Vec<u8>,
) -> FaultServer {
    FaultServer::ipv4(move |mut stream, receipt| {
        capture_request(&mut stream, &receipt);
        write_response(&mut stream, status, &headers, &body);
    })
}

fn client(read_timeout: StdDuration) -> reqwest::Client {
    build_loopback_test_client(StdDuration::from_millis(200), read_timeout)
}

async fn request(server: &FaultServer) -> Response {
    client(StdDuration::from_secs(2))
        .post(server.url("http"))
        .header(AUTHORIZATION, sensitive_bearer())
        .send()
        .await
        .expect("loopback response")
}

fn sensitive_bearer() -> HeaderValue {
    let mut value = HeaderValue::from_str(&format!("Bearer {SECRET}")).expect("sentinel header");
    value.set_sensitive(true);
    value
}

fn decode_hex(fixture: &str) -> Vec<u8> {
    let bytes = fixture.trim().as_bytes();
    assert!(bytes.len().is_multiple_of(2), "DER hex fixture length");
    bytes
        .chunks_exact(2)
        .map(|pair| {
            let high = (pair[0] as char).to_digit(16).expect("DER high nibble");
            let low = (pair[1] as char).to_digit(16).expect("DER low nibble");
            ((high << 4) | low) as u8
        })
        .collect()
}

fn self_signed_tls_server() -> FaultServer {
    FaultServer::ipv4(|stream, _| {
        let certificate = CertificateDer::from(decode_hex(include_str!(
            "testdata/self-signed-cert.der.hex"
        )));
        let key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(decode_hex(include_str!(
            "testdata/self-signed-key.der.hex"
        ))));
        let config = ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(vec![certificate], key)
            .expect("self-signed TLS fixture");
        let connection = ServerConnection::new(Arc::new(config)).expect("TLS server connection");
        let mut tls = StreamOwned::new(connection, stream);
        let _ = tls.write_all(b"HTTP/1.1 200 OK\r\nConnection: close\r\nContent-Length: 0\r\n\r\n");
        // Give the client time to retain its certificate-verification error
        // before the fixture socket is dropped.
        thread::sleep(StdDuration::from_millis(100));
    })
}

async fn consume_openai(
    runtime: &ProviderRuntime,
    response: Response,
    idle: StdDuration,
    overall: StdDuration,
) -> (Result<ProviderRunOutcome>, Vec<ProviderStreamEvent>) {
    let (sender, mut receiver) = mpsc::channel(32);
    let result = runtime
        .consume_sse(
            response,
            ProviderId::OpenAi,
            CancellationToken::new(),
            sender,
            Instant::now() + overall,
        )
        .await;
    let mut events = Vec::new();
    while let Ok(event) = receiver.try_recv() {
        events.push(event);
    }
    // Keep the signature explicit: the runtime's configured idle deadline is
    // what consume_sse uses. Test callers set it through with_limits.
    let _ = idle;
    (result, events)
}

#[tokio::test]
async fn localhost_sse_handles_lf_crlf_comments_multiline_and_byte_chunks() {
    let body = concat!(
        ": heartbeat\r\n\r\n",
        "event: response.created\n",
        "data: {\"type\":\"response.created\",\"response\":{\"id\":\"resp_1\"}}\n\n",
        "event: response.output_text.delta\r\n",
        "data: {\"type\":\"response.output_text.delta\"\r\n",
        "data: ,\"delta\":\"hello\"}\r\n\r\n",
        "\r\n",
        "event: response.completed\r\n",
        "data: {\"type\":\"response.completed\",\"response\":{}}\r\n\r\n",
    )
    .as_bytes()
    .to_vec();
    let server = FaultServer::ipv4(move |mut stream, receipt| {
        capture_request(&mut stream, &receipt);
        let head = format!(
            "HTTP/1.1 200 OK\r\nConnection: close\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\n\r\n",
            body.len()
        );
        stream.write_all(head.as_bytes()).expect("response head");
        for byte in body {
            if stream.write_all(&[byte]).is_err() {
                return;
            }
            let _ = stream.flush();
            thread::yield_now();
        }
    });
    let response = request(&server).await;
    validate_response_metadata(&response, 64, 16 * 1024).unwrap();
    validate_content_type(&response, StreamProtocol::Sse).unwrap();
    let (outcome, events) = consume_openai(
        &ProviderRuntime::new(),
        response,
        StdDuration::from_secs(1),
        StdDuration::from_secs(2),
    )
    .await;
    assert!(matches!(outcome, Ok(ProviderRunOutcome::Completed { .. })));
    assert!(events.contains(&ProviderStreamEvent::TextDelta {
        text: "hello".into()
    }));
    let receipt = server.receipt().expect("sanitized receipt");
    assert_eq!(receipt.method, "POST");
    assert_eq!(receipt.path, "/v1/responses");
    assert!(receipt.header_names.contains(&"authorization".to_owned()));
    assert!(
        receipt.saw_secret,
        "fixture must prove the credential was sent only to its origin"
    );
}

#[tokio::test]
async fn localhost_ndjson_handles_crlf_and_terminal_line_without_newline() {
    let body = b"{\"message\":{\"thinking\":\"why\",\"content\":\"hello\"},\"done\":false}\r\n{\"done\":true,\"done_reason\":\"stop\",\"prompt_eval_count\":2,\"eval_count\":1}".to_vec();
    let server = serve_response(
        "200 OK",
        vec![("Content-Type", "application/x-ndjson")],
        body,
    );
    let response = request(&server).await;
    validate_content_type(&response, StreamProtocol::Ndjson).unwrap();
    let (sender, mut receiver) = mpsc::channel(16);
    let outcome = ProviderRuntime::new()
        .consume_ndjson(
            response,
            ProviderId::OllamaCloud,
            CancellationToken::new(),
            sender,
            Instant::now() + StdDuration::from_secs(2),
        )
        .await
        .unwrap();
    assert!(matches!(outcome, ProviderRunOutcome::Completed { .. }));
    let mut events = Vec::new();
    while let Ok(event) = receiver.try_recv() {
        events.push(event);
    }
    assert!(events.iter().any(
        |event| matches!(event, ProviderStreamEvent::ReasoningDelta { text } if text == "why")
    ));
    assert!(
        events.iter().any(
            |event| matches!(event, ProviderStreamEvent::TextDelta { text } if text == "hello")
        )
    );
}

#[tokio::test]
async fn malformed_json_fails_immediately_and_eof_without_terminal_fails_closed() {
    for (body, expected) in [
        (
            b"data: {not-json}\n\ndata: {\"type\":\"response.completed\",\"response\":{}}\n\n"
                .as_slice(),
            "INVALID_PROVIDER_JSON",
        ),
        (
            b"data: {\"type\":\"response.output_text.delta\",\"delta\":\"partial\"}\n\n".as_slice(),
            "MISSING_TERMINAL_EVENT",
        ),
    ] {
        let server = serve_response(
            "200 OK",
            vec![("Content-Type", "text/event-stream")],
            body.to_vec(),
        );
        let response = request(&server).await;
        let (result, _) = consume_openai(
            &ProviderRuntime::new(),
            response,
            StdDuration::from_secs(1),
            StdDuration::from_secs(2),
        )
        .await;
        assert_eq!(result.expect_err("fault must fail closed").code(), expected);
    }
}

#[tokio::test]
async fn usage_only_and_mixed_channels_preserve_typed_order() {
    let usage_only = serve_response(
        "200 OK",
        vec![("Content-Type", "text/event-stream")],
        b"data: {\"type\":\"response.completed\",\"response\":{\"usage\":{\"input_tokens\":9,\"output_tokens\":0,\"total_tokens\":9}}}\n\n".to_vec(),
    );
    let (outcome, events) = consume_openai(
        &ProviderRuntime::new(),
        request(&usage_only).await,
        StdDuration::from_secs(1),
        StdDuration::from_secs(2),
    )
    .await;
    assert!(events.is_empty());
    assert!(matches!(
        outcome.unwrap(),
        ProviderRunOutcome::Completed {
            usage: Some(TokenUsage {
                total_tokens: Some(9),
                ..
            }),
            ..
        }
    ));

    let mixed = serve_response(
        "200 OK",
        vec![("Content-Type", "text/event-stream")],
        concat!(
            "data: {\"type\":\"response.reasoning_summary_text.delta\",\"delta\":\"think\"}\n\n",
            "data: {\"type\":\"response.output_text.delta\",\"delta\":\"say\"}\n\n",
            "data: {\"type\":\"response.refusal.delta\",\"delta\":\"no\"}\n\n",
            "data: {\"type\":\"response.completed\",\"response\":{}}\n\n",
        )
        .as_bytes()
        .to_vec(),
    );
    let (_, events) = consume_openai(
        &ProviderRuntime::new(),
        request(&mixed).await,
        StdDuration::from_secs(1),
        StdDuration::from_secs(2),
    )
    .await;
    assert!(matches!(&events[..], [
        ProviderStreamEvent::ReasoningDelta { text: a },
        ProviderStreamEvent::TextDelta { text: b },
        ProviderStreamEvent::RefusalDelta { text: c },
    ] if a == "think" && b == "say" && c == "no"));
}

#[tokio::test]
async fn content_type_and_content_encoding_are_strict() {
    for headers in [vec![], vec![("Content-Type", "application/json")]] {
        let server = serve_response("200 OK", headers, OPENAI_COMPLETE.as_bytes().to_vec());
        let response = request(&server).await;
        assert_eq!(
            validate_content_type(&response, StreamProtocol::Sse)
                .expect_err("wrong or missing type")
                .code(),
            "UNEXPECTED_CONTENT_TYPE"
        );
    }

    for encoding in ["gzip", "deflate", "br", "zstd"] {
        let server = serve_response(
            "200 OK",
            vec![
                ("Content-Type", "text/event-stream"),
                ("Content-Encoding", encoding),
            ],
            OPENAI_COMPLETE.as_bytes().to_vec(),
        );
        let response = request(&server).await;
        assert_eq!(
            validate_response_metadata(&response, 64, 16 * 1024)
                .expect_err("compressed stream must fail before body consumption")
                .code(),
            "UNSUPPORTED_CONTENT_ENCODING"
        );
    }
}

#[tokio::test]
async fn oversized_declared_stream_is_rejected_before_the_server_sends_one_megabyte() {
    let sent = Arc::new(AtomicUsize::new(0));
    let thread_sent = Arc::clone(&sent);
    let (permit_sender, permit_receiver) = std_mpsc::channel();
    let server = FaultServer::ipv4(move |mut stream, receipt| {
        capture_request(&mut stream, &receipt);
        stream
            .write_all(b"HTTP/1.1 200 OK\r\nConnection: close\r\nContent-Type: text/event-stream\r\n\r\ndata: ")
            .expect("stream head");
        let block = [b'x'; 4096];
        while permit_receiver.recv().is_ok() {
            match stream.write_all(&block) {
                Ok(()) => {
                    thread_sent.fetch_add(block.len(), Ordering::SeqCst);
                    let _ = stream.flush();
                }
                Err(_) => break,
            }
        }
    });
    let response = request(&server).await;
    let consume = tokio::spawn(async move {
        consume_openai(
            &ProviderRuntime::new(),
            response,
            StdDuration::from_secs(1),
            StdDuration::from_secs(3),
        )
        .await
    });
    // The 256 KiB frame limit must trip within 65 fixed-size blocks. Permits
    // make the bound independent of host scheduling and TCP send-buffer size.
    for _ in 0..65 {
        if permit_sender.send(()).is_err() {
            break;
        }
    }
    let outcome = tokio::time::timeout(StdDuration::from_secs(3), consume).await;
    drop(permit_sender);
    let _ = server.wait();
    let (result, _) = outcome
        .expect("oversized frame must be rejected before more data is permitted")
        .expect("consumer task");
    assert_eq!(result.unwrap_err().code(), "SSE_FRAME_TOO_LARGE");
    assert!(sent.load(Ordering::SeqCst) < 1024 * 1024);
}

#[tokio::test]
async fn response_header_count_and_bytes_have_explicit_post_parse_bounds() {
    let count_server = FaultServer::ipv4(move |mut stream, receipt| {
        capture_request(&mut stream, &receipt);
        let mut response = String::from(
            "HTTP/1.1 200 OK\r\nConnection: close\r\nContent-Type: text/event-stream\r\nContent-Length: 0\r\n",
        );
        for index in 0..65 {
            response.push_str(&format!("X-Fault-{index}: x\r\n"));
        }
        response.push_str("\r\n");
        stream
            .write_all(response.as_bytes())
            .expect("header count response");
    });
    let response = request(&count_server).await;
    assert_eq!(
        validate_response_metadata(&response, 64, 64 * 1024)
            .unwrap_err()
            .code(),
        "RESPONSE_HEADERS_TOO_LARGE"
    );

    let byte_server = FaultServer::ipv4(move |mut stream, receipt| {
        capture_request(&mut stream, &receipt);
        let value = "x".repeat(17 * 1024);
        let response = format!(
            "HTTP/1.1 200 OK\r\nConnection: close\r\nContent-Type: text/event-stream\r\nContent-Length: 0\r\nX-Fault: {value}\r\n\r\n"
        );
        stream
            .write_all(response.as_bytes())
            .expect("header byte response");
    });
    let response = request(&byte_server).await;
    assert_eq!(
        validate_response_metadata(&response, 128, 16 * 1024)
            .unwrap_err()
            .code(),
        "RESPONSE_HEADERS_TOO_LARGE"
    );
}

#[tokio::test]
async fn every_redirect_is_observed_but_never_followed_or_credentialed() {
    for status in [301, 302, 307, 308] {
        let destination = FaultServer::spawn(
            IpAddr::V4(Ipv4Addr::LOCALHOST),
            StdDuration::from_millis(250),
            |mut stream, receipt| {
                capture_request(&mut stream, &receipt);
                write_response(&mut stream, "200 OK", &[], b"");
            },
        )
        .unwrap();
        let location = destination.url("http");
        let status_line: &'static str = match status {
            301 => "301 Moved Permanently",
            302 => "302 Found",
            307 => "307 Temporary Redirect",
            308 => "308 Permanent Redirect",
            _ => unreachable!(),
        };
        let origin = FaultServer::ipv4(move |mut stream, receipt| {
            capture_request(&mut stream, &receipt);
            write_response(&mut stream, status_line, &[("Location", &location)], b"");
        });
        let response = request(&origin).await;
        assert_eq!(response.status().as_u16(), status);
        assert_eq!(response.url().as_str(), origin.url("http"));
        assert!(!response.url().as_str().contains(SECRET));
        let _ = origin.wait();
        let (destination_hits, destination_receipt) = destination.wait();
        assert_eq!(destination_hits, 0, "redirect {status} leaked a request");
        assert!(destination_receipt.is_none());
    }
}

#[tokio::test]
async fn tls_failure_ipv6_and_no_implicit_replay_are_exercised_on_loopback() {
    let tls = self_signed_tls_server();
    let error = client(StdDuration::from_millis(250))
        .post(tls.url("https"))
        .header(AUTHORIZATION, sensitive_bearer())
        .send()
        .await
        .expect_err("plaintext endpoint must fail TLS");
    let diagnostic = format!("{error:?} {error}");
    assert!(!diagnostic.contains(SECRET));
    assert!(
        diagnostic.to_ascii_lowercase().contains("certificate")
            || diagnostic.to_ascii_lowercase().contains("unknownissuer"),
        "expected certificate validation failure: {diagnostic}"
    );
    assert_eq!(tls.hits(), 1, "transport must not replay a streaming POST");

    let Ok(ipv6) = FaultServer::spawn(
        IpAddr::V6(Ipv6Addr::LOCALHOST),
        StdDuration::from_secs(2),
        |mut stream, receipt| {
            capture_request(&mut stream, &receipt);
            write_response(
                &mut stream,
                "200 OK",
                &[("Content-Type", "text/event-stream")],
                OPENAI_COMPLETE.as_bytes(),
            );
        },
    ) else {
        return;
    };
    let response = client(StdDuration::from_secs(1))
        .post(ipv6.url("http"))
        .send()
        .await
        .expect("IPv6-only loopback response");
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn connector_deadline_and_explicit_offline_to_online_recovery_are_bounded() {
    let stalled_handshake = FaultServer::ipv4(|_stream, _| {
        thread::sleep(StdDuration::from_millis(100));
    });
    let error = build_loopback_test_client(StdDuration::from_millis(10), StdDuration::from_secs(1))
        .post(stalled_handshake.url("https"))
        .send()
        .await
        .expect_err("stalled connector must time out");
    assert!(error.is_timeout(), "expected connector timeout: {error:?}");
    assert_eq!(stalled_handshake.hits(), 1);

    let reservation = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    let address = reservation.local_addr().unwrap();
    drop(reservation);
    let offline_error = client(StdDuration::from_millis(100))
        .post(format!("http://{address}/v1/responses"))
        .send()
        .await
        .expect_err("closed loopback port represents offline");
    assert!(offline_error.is_connect());

    let online =
        FaultServer::spawn_at(address, StdDuration::from_secs(2), |mut stream, receipt| {
            capture_request(&mut stream, &receipt);
            write_response(
                &mut stream,
                "200 OK",
                &[("Content-Type", "text/event-stream")],
                OPENAI_COMPLETE.as_bytes(),
            );
        })
        .unwrap();
    let response = client(StdDuration::from_secs(1))
        .post(online.url("http"))
        .send()
        .await
        .expect("explicit new request may recover after network returns");
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(online.hits(), 1);
}

#[tokio::test]
async fn header_idle_overall_timeouts_and_connection_reset_are_distinct() {
    let header_stall = FaultServer::ipv4(|mut stream, receipt| {
        capture_request(&mut stream, &receipt);
        thread::sleep(StdDuration::from_millis(100));
    });
    let result = tokio::time::timeout(
        StdDuration::from_millis(10),
        client(StdDuration::from_secs(1))
            .post(header_stall.url("http"))
            .send(),
    )
    .await;
    assert!(result.is_err(), "response-header deadline must fire");

    let idle_server = FaultServer::ipv4(|mut stream, receipt| {
        capture_request(&mut stream, &receipt);
        stream
            .write_all(
                b"HTTP/1.1 200 OK\r\nConnection: close\r\nContent-Type: text/event-stream\r\n\r\n",
            )
            .unwrap();
        thread::sleep(StdDuration::from_millis(100));
    });
    let limits = RuntimeLimits {
        stream_idle_timeout: StdDuration::from_millis(10),
        ..RuntimeLimits::default()
    };
    let runtime = ProviderRuntime::with_limits(limits).unwrap();
    let (result, _) = consume_openai(
        &runtime,
        request(&idle_server).await,
        StdDuration::from_millis(10),
        StdDuration::from_secs(1),
    )
    .await;
    assert_eq!(result.unwrap_err().code(), "STREAM_IDLE_TIMEOUT");

    let overall_server = FaultServer::ipv4(|mut stream, receipt| {
        capture_request(&mut stream, &receipt);
        stream
            .write_all(
                b"HTTP/1.1 200 OK\r\nConnection: close\r\nContent-Type: text/event-stream\r\n\r\n",
            )
            .unwrap();
        for byte in b"data: {\"type\":\"response.completed\",\"response\":{}}\n\n" {
            if stream.write_all(&[*byte]).is_err() {
                return;
            }
            let _ = stream.flush();
            thread::sleep(StdDuration::from_millis(3));
        }
    });
    let limits = RuntimeLimits {
        stream_idle_timeout: StdDuration::from_millis(50),
        ..RuntimeLimits::default()
    };
    let runtime = ProviderRuntime::with_limits(limits).unwrap();
    let (result, _) = consume_openai(
        &runtime,
        request(&overall_server).await,
        StdDuration::from_millis(50),
        StdDuration::from_millis(15),
    )
    .await;
    assert_eq!(result.unwrap_err().code(), "OVERALL_TIMEOUT");

    let reset = FaultServer::ipv4(|mut stream, receipt| {
        capture_request(&mut stream, &receipt);
        stream
            .write_all(b"HTTP/1.1 200 OK\r\nConnection: close\r\nContent-Type: text/event-stream\r\nContent-Length: 9999\r\n\r\ndata: {\"type\":")
            .unwrap();
    });
    let (result, _) = consume_openai(
        &ProviderRuntime::new(),
        request(&reset).await,
        StdDuration::from_secs(1),
        StdDuration::from_secs(2),
    )
    .await;
    assert_eq!(result.unwrap_err().code(), "HTTP_TRANSPORT_FAILED");
    assert_eq!(reset.hits(), 1, "reset stream must not be replayed");
}

#[tokio::test]
async fn retry_after_and_transient_statuses_return_typed_decisions_without_replay() {
    let server = serve_response(
        "429 Too Many Requests",
        vec![("Retry-After", "7")],
        b"sensitive provider body".to_vec(),
    );
    let response = request(&server).await;
    let error = read_http_error(
        response,
        ProviderId::OpenAi,
        64 * 1024,
        &CancellationToken::new(),
        StdDuration::from_secs(1),
        Instant::now() + StdDuration::from_secs(2),
    )
    .await;
    assert_eq!(
        error.retry_decision(),
        RetryDecision::RetryAfter {
            delay: StdDuration::from_secs(7)
        }
    );
    assert!(!error.to_string().contains("sensitive provider body"));
    assert_eq!(server.hits(), 1);

    for status in [500, 502, 503] {
        let status_line = match status {
            500 => "500 Internal Server Error",
            502 => "502 Bad Gateway",
            503 => "503 Service Unavailable",
            _ => unreachable!(),
        };
        let server = serve_response(status_line, vec![], Vec::new());
        let response = request(&server).await;
        let error = read_http_error(
            response,
            ProviderId::OpenAi,
            64 * 1024,
            &CancellationToken::new(),
            StdDuration::from_secs(1),
            Instant::now() + StdDuration::from_secs(2),
        )
        .await;
        assert!(matches!(
            error.retry_decision(),
            RetryDecision::ExponentialBackoff { .. }
        ));
        assert_eq!(server.hits(), 1, "HTTP {status} was implicitly replayed");
    }
}

#[tokio::test]
async fn malicious_provider_strings_never_enter_errors_and_events_remain_json_safe() {
    let malicious = "\\u0000\\u001b[31m\\\"}\\nAuthorization: Bearer NET-010-SENTINEL-DO-NOT-LOG";
    let body = format!(
        "data: {{\"type\":\"response.failed\",\"response\":{{\"error\":{{\"code\":\"x\",\"message\":\"{malicious}\"}}}}}}\n\n"
    );
    let server = serve_response(
        "200 OK",
        vec![("Content-Type", "text/event-stream")],
        body.into_bytes(),
    );
    let (result, _) = consume_openai(
        &ProviderRuntime::new(),
        request(&server).await,
        StdDuration::from_secs(1),
        StdDuration::from_secs(2),
    )
    .await;
    let error = result.unwrap_err();
    let rendered = format!("{error:?} {error}");
    assert_eq!(error.code(), "PROVIDER_STREAM_ERROR");
    assert!(!rendered.contains(SECRET));
    assert!(!rendered.contains("Authorization"));

    let event = ProviderStreamEvent::TextDelta {
        text: "\0\u{1b}\n\"}".to_owned(),
    };
    let wire = serde_json::to_string(&event).unwrap();
    assert!(!wire.as_bytes().contains(&0));
    assert_eq!(
        serde_json::from_str::<ProviderStreamEvent>(&wire).unwrap(),
        event
    );
}

#[test]
fn credential_sentinel_is_absent_from_debug_url_and_stable_errors() {
    let credential = ProviderCredential::for_official(ProviderId::OpenAi, SECRET).unwrap();
    assert!(!format!("{credential:?}").contains(SECRET));
    let url = url::Url::parse("https://api.openai.com/v1/responses").unwrap();
    assert!(!url.as_str().contains(SECRET));
    let error = RuntimeError::new(
        RuntimeErrorKind::Http,
        "HTTP_TRANSPORT_FAILED",
        "provider HTTP transport failed",
    );
    assert!(!format!("{error:?} {error}").contains(SECRET));
}
