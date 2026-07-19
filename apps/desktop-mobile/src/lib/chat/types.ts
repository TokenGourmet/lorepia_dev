export type SpeakerRole = "character" | "user";

export type ThreadMode = "chat" | "story";

export interface ChatMessage {
  id: string;
  role: SpeakerRole;
  narration?: string;
  text: string;
  sentAt: Date;
  streaming?: boolean;
}
