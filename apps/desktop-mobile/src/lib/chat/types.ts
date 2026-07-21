export type SpeakerRole = "character" | "user";

export type ThreadMode = "chat" | "story";

export interface ChatMessage {
  id: string;
  role: SpeakerRole;
  narration?: string;
  text: string;
  /** Frame-sized text nodes retained separately until the stream terminates. */
  streamingChunks?: string[];
  sentAt: Date;
  streaming?: boolean;
  /** Durable delivery state restored from SQLite after reload/restart. */
  deliveryState?: "complete" | "partial" | "failed";
}
