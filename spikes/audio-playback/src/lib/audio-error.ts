import { AUDIO_ERROR_CODES, type AudioErrorCode } from "./audio-contract";

export class AudioM1Error extends Error {
  readonly code: AudioErrorCode;

  constructor(code: AudioErrorCode) {
    super("Audio M-1 operation failed");
    this.name = "AudioM1Error";
    this.code = code;
  }
}

export function isAudioErrorCode(value: unknown): value is AudioErrorCode {
  return typeof value === "string" && AUDIO_ERROR_CODES.includes(value as AudioErrorCode);
}
