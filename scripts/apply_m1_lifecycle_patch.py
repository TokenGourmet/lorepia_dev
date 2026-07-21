from pathlib import Path
import xml.etree.ElementTree as ET


rust_path = Path("spikes/channel-stream/src-tauri/src/lib.rs")
rust = rust_path.read_text(encoding="utf-8")

old_capacity_call = '''        if !wait_for_data_capacity(&request, config.ack_timeout_ms).await {
            emit_cancelled(&request, &channel).await;
            return;
        }
'''
new_capacity_call = '''        match wait_for_data_capacity(&request, config.ack_timeout_ms).await {
            CapacityWait::Ready => {}
            CapacityWait::Cancelled => {
                emit_cancelled(&request, &channel).await;
                return;
            }
            CapacityWait::TimedOut => {
                emit_failed(
                    &request,
                    &channel,
                    StreamFailure::new(
                        "ACK_TIMEOUT",
                        format!(
                            "frontend did not free stream capacity within {} ms",
                            config.ack_timeout_ms
                        ),
                    ),
                )
                .await;
                return;
            }
        }
'''
if rust.count(old_capacity_call) != 1:
    raise SystemExit("expected exactly one run_stream capacity call")
rust = rust.replace(old_capacity_call, new_capacity_call)

old_wait = '''async fn wait_for_data_capacity(request: &StreamRequest, poll_ms: u64) -> bool {
    loop {
        {
            let mut machine = request.machine.lock().await;
            if machine.status.is_terminal() {
                return false;
            }
            if machine.cancel_requested {
                return false;
            }
            if machine.has_data_capacity() {
                return true;
            }
            machine.apply_pressure();
        }
        let _ =
            tokio::time::timeout(Duration::from_millis(poll_ms), request.notify.notified()).await;
    }
}
'''
new_wait = '''enum CapacityWait {
    Ready,
    Cancelled,
    TimedOut,
}

async fn wait_for_data_capacity(request: &StreamRequest, timeout_ms: u64) -> CapacityWait {
    let deadline = tokio::time::Instant::now() + Duration::from_millis(timeout_ms);
    loop {
        {
            let mut machine = request.machine.lock().await;
            if machine.status.is_terminal() || machine.cancel_requested {
                return CapacityWait::Cancelled;
            }
            if machine.has_data_capacity() {
                return CapacityWait::Ready;
            }
            machine.apply_pressure();
        }

        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            return CapacityWait::TimedOut;
        }
        if tokio::time::timeout(remaining, request.notify.notified())
            .await
            .is_err()
        {
            return CapacityWait::TimedOut;
        }
    }
}
'''
if rust.count(old_wait) != 1:
    raise SystemExit("expected exactly one wait_for_data_capacity implementation")
rust = rust.replace(old_wait, new_wait)

timeout_test = r'''

    #[tokio::test(flavor = "current_thread")]
    async fn missing_ack_times_out_with_failed_terminal() {
        use tauri::ipc::InvokeResponseBody;

        let mut config = config(24, 2);
        config.ack_timeout_ms = 10;
        let mut machine = StreamMachine::new("request-ack-timeout".into(), &config);
        let started = machine.start().unwrap();
        let request = Arc::new(StreamRequest::new(machine));
        let captured = Arc::new(Mutex::new(Vec::<serde_json::Value>::new()));
        let callback_captured = Arc::clone(&captured);
        let channel = Channel::new(move |body| {
            let InvokeResponseBody::Json(json) = body else {
                return Err(std::io::Error::other("unexpected raw channel body").into());
            };
            callback_captured
                .lock()
                .unwrap()
                .push(serde_json::from_str(&json)?);
            Ok(())
        });

        channel.send(started).unwrap();
        tokio::time::timeout(
            Duration::from_secs(1),
            run_stream(Arc::clone(&request), config, channel),
        )
        .await
        .expect("missing ACK should terminate the stream");

        let events = captured.lock().unwrap().clone();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0]["type"], "started");
        assert_eq!(events[1]["type"], "failed");
        assert_eq!(events[1]["error"]["code"], "ACK_TIMEOUT");
        assert!(events[1]["error"]["message"]
            .as_str()
            .unwrap()
            .contains("10 ms"));

        let snapshot = request.machine.lock().await.snapshot();
        assert_eq!(snapshot.status, StreamStatus::Failed);
        assert_eq!(snapshot.last_seq, 1);
        assert_eq!(snapshot.last_acked_seq, -1);
        assert_eq!(snapshot.in_flight, 2);
        assert_eq!(snapshot.text, "");
        assert_eq!(
            snapshot.error.as_ref().map(|error| error.code.as_str()),
            Some("ACK_TIMEOUT")
        );
        assert_eq!(request.terminal_seq.load(Ordering::Acquire), 1);
    }
'''
if "missing_ack_times_out_with_failed_terminal" in rust:
    raise SystemExit("timeout regression test already exists")
final_close = rust.rfind("\n}")
if final_close == -1:
    raise SystemExit("could not find test module closing brace")
rust = rust[:final_close] + timeout_test + rust[final_close:]
rust_path.write_text(rust, encoding="utf-8")

readme_path = Path("spikes/channel-stream/README.md")
readme = readme_path.read_text(encoding="utf-8")
old_readme = "- The frontend acknowledges consumed sequence numbers. A bounded in-flight window prevents an unbounded producer queue, and consumer delay expands the effective batching window without dropping text."
new_readme = old_readme + " If no ACK frees capacity within `ackTimeoutMs`, the producer emits one structured `ACK_TIMEOUT` failure instead of polling forever."
if readme.count(old_readme) != 1:
    raise SystemExit("expected exactly one README ACK paragraph")
readme_path.write_text(readme.replace(old_readme, new_readme), encoding="utf-8")

file_paths = Path(
    "spikes/channel-stream/src-tauri/gen/android/app/src/main/res/xml/file_paths.xml"
)
file_paths.write_text(
    '''<?xml version="1.0" encoding="utf-8"?>
<paths xmlns:android="http://schemas.android.com/apk/res/android">
  <files-path name="exports" path="exports/" />
  <cache-path name="share_cache" path="share/" />
  <external-files-path name="media_exports" path="LorePia/Exports/" />
</paths>
''',
    encoding="utf-8",
)
ET.parse(file_paths)
