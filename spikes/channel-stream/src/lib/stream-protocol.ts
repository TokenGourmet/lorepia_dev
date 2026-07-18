import { Channel, invoke } from "@tauri-apps/api/core";

export type StreamFailure = {
  code: string;
  message: string;
};

export type StreamEvent =
  | {
      type: "started";
      requestId: string;
      seq: number;
      batchWindowMs: number;
      maxInFlight: number;
    }
  | {
      type: "delta";
      requestId: string;
      seq: number;
      text: string;
    }
  | {
      type: "completed";
      requestId: string;
      seq: number;
    }
  | {
      type: "cancelled";
      requestId: string;
      seq: number;
    }
  | {
      type: "failed";
      requestId: string;
      seq: number;
      error: StreamFailure;
    };

export type StreamConfig = {
  batchWindowMs?: number;
  maxInFlight?: number;
  chunkIntervalMs?: number;
  chunks?: string[];
  failAfterChunks?: number;
  ackTimeoutMs?: number;
};

export type StartStreamResponse = {
  requestId: string;
};

export type AckStreamResponse = {
  requestId: string;
  acknowledgedThrough: number;
  inFlight: number;
};

export type CancelStreamResponse = {
  requestId: string;
  accepted: boolean;
};

export type StreamSnapshotStatus =
  | "queued"
  | "streaming"
  | "completed"
  | "cancelled"
  | "failed";

export type StreamSnapshot = {
  requestId: string;
  status: StreamSnapshotStatus;
  lastSeq: number;
  lastAckedSeq: number;
  inFlight: number;
  text: string;
  error: StreamFailure | null;
  batchWindowMs: number;
  effectiveBatchWindowMs: number;
  maxInFlight: number;
};

const command = {
  start: "start_mock_stream",
  acknowledge: "ack_stream",
  cancel: "cancel_stream",
  snapshot: "get_stream_snapshot",
} as const;

export function createStreamChannel(
  onMessage: (event: StreamEvent) => void,
): Channel<StreamEvent> {
  const channel = new Channel<StreamEvent>();
  channel.onmessage = onMessage;
  return channel;
}

export function startMockStream(
  onEvent: Channel<StreamEvent>,
  config?: StreamConfig,
): Promise<StartStreamResponse> {
  return invoke<StartStreamResponse>(command.start, {
    onEvent,
    ...(config === undefined ? {} : { config }),
  });
}

export function acknowledgeStream(
  requestId: string,
  seq: number,
): Promise<AckStreamResponse> {
  return invoke<AckStreamResponse>(command.acknowledge, { requestId, seq });
}

export function cancelStream(requestId: string): Promise<CancelStreamResponse> {
  return invoke<CancelStreamResponse>(command.cancel, { requestId });
}

export function getStreamSnapshot(requestId: string): Promise<StreamSnapshot> {
  return invoke<StreamSnapshot>(command.snapshot, { requestId });
}

export function describeCommandError(error: unknown): string {
  if (typeof error === "string") return error;

  if (typeof error === "object" && error !== null) {
    const code = "code" in error && typeof error.code === "string" ? error.code : null;
    const message =
      "message" in error && typeof error.message === "string" ? error.message : null;

    if (code && message) return `${code}: ${message}`;
    if (message) return message;
  }

  return "알 수 없는 명령 오류";
}
