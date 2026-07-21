use std::{
    fmt,
    io::{BufWriter, Write},
    path::PathBuf,
    str::FromStr,
};

use serde::Serialize;
use serde_json::json;

use crate::util::{
    DeterministicRng, Result, deterministic_id, emit_receipt, invalid, prepare_new_file,
};

const REQUEST_NAMESPACE: u64 = 0x5354_524d_0000_0000;
const MODEL_MAX_ACTIVE_REQUESTS: u64 = 128;
const MODEL_MAX_IN_FLIGHT: u64 = 4;
const MODEL_DIRECT_CHANNEL_BYTES: u64 = 4_096;
const MODEL_DELTA_FRAGMENT_BYTES: u64 = 512;
const MODEL_PER_REQUEST_RESERVATION_BYTES: u64 =
    MODEL_DIRECT_CHANNEL_BYTES * (MODEL_MAX_IN_FLIGHT + 1);
const MODEL_GLOBAL_RESERVATION_BYTES: u64 =
    MODEL_PER_REQUEST_RESERVATION_BYTES * MODEL_MAX_ACTIVE_REQUESTS;
const MODEL_ACK_LEASE_MS: u64 = 30_000;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AckProfile {
    Immediate,
    Delayed,
    Never,
}

impl fmt::Display for AckProfile {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Immediate => "immediate",
            Self::Delayed => "delayed",
            Self::Never => "never",
        })
    }
}

impl FromStr for AckProfile {
    type Err = crate::util::Error;

    fn from_str(value: &str) -> Result<Self> {
        match value {
            "immediate" => Ok(Self::Immediate),
            "delayed" => Ok(Self::Delayed),
            "never" => Ok(Self::Never),
            _ => Err(invalid(
                "--ack-profile must be immediate, delayed, or never",
            )),
        }
    }
}

#[derive(Debug)]
pub struct StreamOptions {
    pub requests: u64,
    pub ack_profile: AckProfile,
    pub seed: u64,
    pub output: PathBuf,
    pub receipt: Option<PathBuf>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct StreamReceipt {
    artifact_kind: &'static str,
    evidence_class: &'static str,
    runtime_evidence: bool,
    tool_version: &'static str,
    seed: u64,
    ack_profile: AckProfile,
    requested: u64,
    admitted: u64,
    rejected_at_capacity: u64,
    timed_out: u64,
    terminal_completed: u64,
    peak_active_requests: u64,
    peak_reserved_bytes: u64,
    final_reserved_bytes: u64,
    max_active_requests: u64,
    max_in_flight: u64,
    ack_lease_ms: u64,
    per_request_reservation_bytes: u64,
    global_reservation_bytes: u64,
    output: String,
}

pub fn generate(options: StreamOptions) -> Result<()> {
    let (output, file) = prepare_new_file(&options.output)?;
    let mut writer = BufWriter::new(file);
    write_line(
        &mut writer,
        &json!({
            "recordType": "artifact_header",
            "artifactKind": "LOREPIA_STREAM_MODEL_SCHEDULE",
            "evidenceClass": "MODEL_SCHEDULE_ONLY",
            "runtimeEvidence": false,
            "warning": "NOT_TAURI_RUNTIME_EVIDENCE",
            "seed": options.seed,
            "ackProfile": options.ack_profile,
            "modelContract": {
                "maxActiveRequests": MODEL_MAX_ACTIVE_REQUESTS,
                "maxInFlight": MODEL_MAX_IN_FLIGHT,
                "directChannelBytes": MODEL_DIRECT_CHANNEL_BYTES,
                "deltaFragmentBytes": MODEL_DELTA_FRAGMENT_BYTES,
                "perRequestReservationBytes": MODEL_PER_REQUEST_RESERVATION_BYTES,
                "globalReservationBytes": MODEL_GLOBAL_RESERVATION_BYTES,
                "ackLeaseMs": MODEL_ACK_LEASE_MS
            }
        }),
    )?;

    let admitted = options.requests.min(MODEL_MAX_ACTIVE_REQUESTS);
    let rejected = options.requests - admitted;
    let peak_reserved = admitted
        .checked_mul(MODEL_PER_REQUEST_RESERVATION_BYTES)
        .ok_or_else(|| invalid("stream reservation model overflowed"))?;
    if peak_reserved > MODEL_GLOBAL_RESERVATION_BYTES {
        return Err(invalid("stream model exceeded its global byte budget"));
    }

    for index in 0..options.requests {
        let request_id = deterministic_id(REQUEST_NAMESPACE, options.seed, index);
        if index < admitted {
            write_line(
                &mut writer,
                &json!({
                    "recordType": "admission",
                    "requestId": request_id,
                    "requestIndex": index,
                    "decision": "admitted",
                    "reservedBytes": MODEL_PER_REQUEST_RESERVATION_BYTES,
                    "simulatedAtMs": 0
                }),
            )?;
        } else {
            write_line(
                &mut writer,
                &json!({
                    "recordType": "admission",
                    "requestId": request_id,
                    "requestIndex": index,
                    "decision": "rejected",
                    "code": "TOO_MANY_ACTIVE_STREAMS",
                    "reservedBytes": 0,
                    "simulatedAtMs": 0
                }),
            )?;
        }
    }

    let mut rng = DeterministicRng::new(options.seed);
    for index in 0..admitted {
        let request_id = deterministic_id(REQUEST_NAMESPACE, options.seed, index);
        for sequence in 1..=MODEL_MAX_IN_FLIGHT {
            let modeled_bytes = 128 + rng.next_u64() % (MODEL_DELTA_FRAGMENT_BYTES - 127);
            write_line(
                &mut writer,
                &json!({
                    "recordType": "delta_schedule",
                    "requestId": request_id,
                    "seq": sequence,
                    "payloadBytes": modeled_bytes,
                    "contentIncluded": false,
                    "simulatedAtMs": sequence
                }),
            )?;
            if options.ack_profile == AckProfile::Immediate {
                write_line(
                    &mut writer,
                    &json!({
                        "recordType": "ack_schedule",
                        "requestId": request_id,
                        "throughSeq": sequence,
                        "simulatedAtMs": sequence + 1
                    }),
                )?;
            }
        }
        match options.ack_profile {
            AckProfile::Immediate => write_line(
                &mut writer,
                &json!({
                    "recordType": "terminal_schedule",
                    "requestId": request_id,
                    "outcome": "completed",
                    "simulatedAtMs": 10
                }),
            )?,
            AckProfile::Delayed => {
                write_line(
                    &mut writer,
                    &json!({
                        "recordType": "ack_schedule",
                        "requestId": request_id,
                        "throughSeq": MODEL_MAX_IN_FLIGHT,
                        "simulatedAtMs": 10_000,
                        "delayClass": "BEFORE_ACK_LEASE_DEADLINE"
                    }),
                )?;
                write_line(
                    &mut writer,
                    &json!({
                        "recordType": "terminal_schedule",
                        "requestId": request_id,
                        "outcome": "completed",
                        "simulatedAtMs": 10_001
                    }),
                )?;
            }
            AckProfile::Never => write_line(
                &mut writer,
                &json!({
                    "recordType": "lease_expiry_schedule",
                    "requestId": request_id,
                    "outcome": "abandoned_timeout",
                    "code": "STREAM_ACK_TIMEOUT",
                    "simulatedAtMs": MODEL_ACK_LEASE_MS
                }),
            )?,
        }
        write_line(
            &mut writer,
            &json!({
                "recordType": "reservation_release",
                "requestId": request_id,
                "releasedBytes": MODEL_PER_REQUEST_RESERVATION_BYTES,
                "simulatedAtMs": if options.ack_profile == AckProfile::Never { MODEL_ACK_LEASE_MS } else { 10_002 }
            }),
        )?;
    }

    let receipt = StreamReceipt {
        artifact_kind: "LOREPIA_STREAM_MODEL_SCHEDULE",
        evidence_class: "MODEL_SCHEDULE_ONLY",
        runtime_evidence: false,
        tool_version: env!("CARGO_PKG_VERSION"),
        seed: options.seed,
        ack_profile: options.ack_profile,
        requested: options.requests,
        admitted,
        rejected_at_capacity: rejected,
        timed_out: if options.ack_profile == AckProfile::Never {
            admitted
        } else {
            0
        },
        terminal_completed: if options.ack_profile == AckProfile::Never {
            0
        } else {
            admitted
        },
        peak_active_requests: admitted,
        peak_reserved_bytes: peak_reserved,
        final_reserved_bytes: 0,
        max_active_requests: MODEL_MAX_ACTIVE_REQUESTS,
        max_in_flight: MODEL_MAX_IN_FLIGHT,
        ack_lease_ms: MODEL_ACK_LEASE_MS,
        per_request_reservation_bytes: MODEL_PER_REQUEST_RESERVATION_BYTES,
        global_reservation_bytes: MODEL_GLOBAL_RESERVATION_BYTES,
        output: output.display().to_string(),
    };
    write_line(
        &mut writer,
        &json!({
            "recordType": "artifact_footer",
            "receipt": receipt
        }),
    )?;
    writer.flush()?;
    writer.get_ref().sync_all()?;
    emit_receipt(options.receipt.as_deref(), &receipt)
}

fn write_line<W: Write, T: Serialize>(writer: &mut W, value: &T) -> Result<()> {
    serde_json::to_writer(&mut *writer, value)?;
    writer.write_all(b"\n")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    #[test]
    fn models_128_129_capacity_and_never_ack_without_claiming_runtime_evidence() {
        let directory = tempfile::tempdir().unwrap();
        let output = directory.path().join("stream.jsonl");
        let receipt = directory.path().join("stream-receipt.json");
        generate(StreamOptions {
            requests: 129,
            ack_profile: AckProfile::Never,
            seed: 42,
            output: output.clone(),
            receipt: Some(receipt),
        })
        .unwrap();
        let lines = fs::read_to_string(output).unwrap();
        let records = lines
            .lines()
            .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(records[0]["runtimeEvidence"], false);
        assert_eq!(records[0]["warning"], "NOT_TAURI_RUNTIME_EVIDENCE");
        assert_eq!(
            records
                .iter()
                .filter(|record| record["decision"] == "rejected")
                .count(),
            1
        );
        let footer = records.last().unwrap();
        assert_eq!(footer["receipt"]["admitted"], 128);
        assert_eq!(footer["receipt"]["timedOut"], 128);
        assert_eq!(footer["receipt"]["finalReservedBytes"], 0);
    }
}
