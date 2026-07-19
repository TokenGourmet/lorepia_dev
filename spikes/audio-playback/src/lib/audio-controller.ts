import {
  AUDIO_ACTIONS,
  AUDIO_FIXTURE,
  AUDIO_METADATA_TOLERANCE_MS,
  AUDIO_MONOTONIC_TOLERANCE_MS,
  AUDIO_PAUSE_DRIFT_TOLERANCE_MS,
  AUDIO_PAUSE_WINDOW_MS,
  AUDIO_POLICY_VERSION,
  AUDIO_PROGRESS_MIN_MS,
  AUDIO_SEEK_TOLERANCE_MS,
  AUDIO_TRACE_LIMIT,
  parseAudioReceipt,
  type AudioActionCounts,
  type AppPhase,
  type AudioAction,
  type AudioErrorCode,
  type AudioReceipt,
  type AudioState,
  type AudioTransition,
} from "./audio-contract";
import { AudioM1Error } from "./audio-error";
import { loadApprovedFixture, type FixtureVerification } from "./audio-fixture";

const MEDIA_EVENT_TIMEOUT_MS = 5_000;
const PLAYBACK_PROGRESS_TIMEOUT_MS = 2_500;

function initialActionCounts(): AudioActionCounts {
  const counts = {} as AudioActionCounts;
  for (const action of AUDIO_ACTIONS) counts[action] = 0;
  counts.INITIALIZE = 1;
  return counts;
}

export interface AudioPort {
  preload: string;
  src: string;
  currentTime: number;
  readonly duration: number;
  readonly paused: boolean;
  load(): void;
  play(): Promise<void>;
  pause(): void;
  removeAttribute(qualifiedName: string): void;
  addEventListener(type: string, listener: EventListener): void;
  removeEventListener(type: string, listener: EventListener): void;
}

export type AudioControllerDependencies = {
  loadFixture: (signal: AbortSignal) => Promise<FixtureVerification>;
  createMedia: () => AudioPort;
  createObjectUrl: (blob: Blob) => string;
  revokeObjectUrl: (url: string) => void;
  setTimer: (callback: () => void, delayMs: number) => ReturnType<typeof setTimeout>;
  clearTimer: (handle: ReturnType<typeof setTimeout>) => void;
  onChange?: (receipt: AudioReceipt) => void;
};

function browserDependencies(): AudioControllerDependencies {
  return {
    loadFixture: (signal) => loadApprovedFixture(undefined, undefined, signal),
    createMedia: () => document.createElement("audio"),
    createObjectUrl: (blob) => URL.createObjectURL(blob),
    revokeObjectUrl: (url) => URL.revokeObjectURL(url),
    setTimer: (callback, delayMs) => setTimeout(callback, delayMs),
    clearTimer: (handle) => clearTimeout(handle),
  };
}

export class AudioM1Controller {
  private readonly dependencies: AudioControllerDependencies;
  private state: AudioState = "RELEASED";
  private appPhase: AppPhase = "FOREGROUND";
  private media: AudioPort | null = null;
  private objectUrl: string | null = null;
  private ownedListeners: Array<{ type: string; listener: EventListener }> = [];
  private pendingWaitAborts = new Map<() => void, number>();
  private pendingPlayback: { generation: number; media: AudioPort } | null = null;
  private busy = false;
  private generation = 0;
  private invalidationError: AudioErrorCode = "LIFECYCLE_INTERRUPTED";
  private seq = 0;
  private actionCounts = initialActionCounts();
  private trace: AudioTransition[] = [
    {
      seq: 0,
      action: "INITIALIZE",
      before: "RELEASED",
      after: "RELEASED",
      positionMs: 0,
    },
  ];
  private verification = {
    bytesAndSha256Matched: false,
    wavHeaderMatched: false,
    metadataDurationMatched: false,
  };
  private observations = {
    playProgressDeltaMs: 0,
    pauseWindowMs: 0,
    pauseDriftMs: 0,
    resumeProgressDeltaMs: 0,
    seekEventObserved: false,
    stopResetObserved: false,
  };
  private events = {
    loadedMetadataCount: 0,
    timeUpdateCount: 0,
    seekedCount: 0,
    endedCount: 0,
    mediaErrorCount: 0,
  };
  private lifecycle = {
    backgroundCount: 0,
    foregroundCount: 0,
    automaticPauseCount: 0,
    automaticReleaseCount: 0,
    automaticResumeCount: 0 as const,
  };

  constructor(dependencies: Partial<AudioControllerDependencies> = {}) {
    this.dependencies = { ...browserDependencies(), ...dependencies };
  }

  snapshot(): AudioReceipt {
    const transition = this.trace.at(-1);
    if (transition === undefined) throw new AudioM1Error("RECEIPT_INVALID");
    return parseAudioReceipt({
      protocolVersion: 1,
      policyVersion: AUDIO_POLICY_VERSION,
      backend: "HTML_AUDIO_ELEMENT",
      actionCounts: { ...this.actionCounts },
      fixture: AUDIO_FIXTURE,
      verification: { ...this.verification },
      observations: { ...this.observations },
      events: { ...this.events },
      state: this.state,
      appPhase: this.appPhase,
      positionMs: this.positionMs(),
      transition: { ...transition },
      lifecycle: { ...this.lifecycle },
      resources: {
        elementAllocated: this.media !== null,
        sourceAttached: this.media !== null && this.media.src.length > 0,
        objectUrlActive: this.objectUrl !== null,
        listenerCount:
          this.ownedListeners.length +
          [...this.pendingWaitAborts.values()].reduce((sum, count) => sum + count, 0),
        osResourceRelease: "UNVERIFIED_REQUIRES_DEVICE_EVIDENCE",
      },
      trace: this.trace.map((entry) => ({ ...entry })),
    });
  }

  async load(): Promise<AudioReceipt> {
    this.requireAvailable();
    this.requireForeground();
    this.requireState("RELEASED");
    const token = this.beginOperation();
    this.resetSessionEvidence();
    try {
      const verified = await this.waitForFixtureLoad();
      this.requireCurrentGeneration(token);
      const byteCopy = Uint8Array.from(verified.bytes);
      const objectUrl = this.dependencies.createObjectUrl(
        new Blob([byteCopy.buffer], { type: "audio/wav" }),
      );
      this.objectUrl = objectUrl;
      const media = this.dependencies.createMedia();
      this.media = media;
      media.preload = "auto";
      media.src = objectUrl;

      const metadata = this.waitForMediaEvent(
        media,
        "loadedmetadata",
        "error",
        "FIXTURE_UNSUPPORTED",
        "MEDIA_EVENT_TIMEOUT",
      );
      void metadata.catch(() => undefined);
      media.load();
      await metadata;
      this.requireCurrentGeneration(token);
      this.events.loadedMetadataCount += 1;

      const durationMs = Math.round(media.duration * 1_000);
      if (
        !Number.isFinite(media.duration) ||
        Math.abs(durationMs - AUDIO_FIXTURE.durationMs) > AUDIO_METADATA_TOLERANCE_MS
      ) {
        throw new AudioM1Error("METADATA_MISMATCH");
      }

      this.attachOwnedListener(media, "timeupdate", this.onTimeUpdate);
      this.attachOwnedListener(media, "ended", this.onEnded);
      this.attachOwnedListener(media, "error", this.onMediaError);
      this.verification = {
        bytesAndSha256Matched: verified.bytesAndSha256Matched,
        wavHeaderMatched: verified.wavHeaderMatched,
        metadataDurationMatched: true,
      };
      const before = this.state;
      this.state = "LOADED";
      this.appendTransition("LOAD", before, this.state);
      return this.changedSnapshot();
    } catch (error) {
      if (token !== this.generation) {
        throw new AudioM1Error(this.invalidationError);
      }
      if (token === this.generation) {
        this.cleanupResources();
        this.state = "RELEASED";
      }
      if (error instanceof AudioM1Error) throw error;
      throw new AudioM1Error("FIXTURE_LOAD_FAILED");
    } finally {
      this.finishOperation(token);
    }
  }

  async play(): Promise<AudioReceipt> {
    this.requireAvailable();
    this.requireForeground();
    this.requireState("LOADED");
    return this.startPlayback("PLAY");
  }

  async pause(): Promise<AudioReceipt> {
    this.requireAvailable();
    this.requireForeground();
    this.requireState("PLAYING");
    if (this.actionCounts.PAUSE > 0) throw new AudioM1Error("INVALID_TRANSITION");
    const media = this.requireMedia();
    const before = this.state;
    const pausedAtMs = this.positionMs();
    const token = this.beginOperation();
    let maximumDriftMs = 0;
    const observePauseDrift: EventListener = () => {
      maximumDriftMs = Math.max(
        maximumDriftMs,
        Math.abs(this.positionMs() - pausedAtMs),
      );
    };
    const detachPauseObserver = this.attachOwnedListener(
      media,
      "timeupdate",
      observePauseDrift,
    );
    try {
      if (this.playbackRetreatedFromLastTransition(pausedAtMs)) {
        throw new AudioM1Error("PLAYBACK_NON_MONOTONIC");
      }
      media.pause();
      if (!media.paused) throw new AudioM1Error("PAUSE_UNSTABLE");
      await this.waitForDelay(AUDIO_PAUSE_WINDOW_MS);
      this.requireCurrentGeneration(token);
      observePauseDrift(new Event("timeupdate"));
      if (!media.paused || maximumDriftMs > AUDIO_PAUSE_DRIFT_TOLERANCE_MS) {
        throw new AudioM1Error("PAUSE_UNSTABLE");
      }
      if (this.playbackRetreatedFromLastTransition()) {
        throw new AudioM1Error("PLAYBACK_NON_MONOTONIC");
      }
      this.observations.pauseWindowMs = AUDIO_PAUSE_WINDOW_MS;
      this.observations.pauseDriftMs = maximumDriftMs;
      detachPauseObserver();
      this.state = "PAUSED";
      this.appendTransition("PAUSE", before, this.state);
      return this.changedSnapshot();
    } catch (error) {
      this.failOperation(error, token, before, "PAUSE_UNSTABLE");
    } finally {
      detachPauseObserver();
      this.finishOperation(token);
    }
  }

  async resume(): Promise<AudioReceipt> {
    this.requireAvailable();
    this.requireForeground();
    this.requireState("PAUSED");
    if (this.actionCounts.RESUME > 0) throw new AudioM1Error("INVALID_TRANSITION");
    return this.startPlayback("RESUME");
  }

  async seekCheckpoint(): Promise<AudioReceipt> {
    this.requireAvailable();
    this.requireForeground();
    this.requireState("PAUSED");
    if (this.actionCounts.SEEK_CHECKPOINT > 0) {
      throw new AudioM1Error("INVALID_TRANSITION");
    }
    const media = this.requireMedia();
    const token = this.beginOperation();
    try {
      const targetSeconds = AUDIO_FIXTURE.seekCheckpointMs / 1_000;
      const seeked = this.waitForMediaEvent(
        media,
        "seeked",
        "error",
        "SEEK_FAILED",
        "SEEK_FAILED",
      );
      void seeked.catch(() => undefined);
      try {
        media.currentTime = targetSeconds;
      } catch {
        throw new AudioM1Error("SEEK_FAILED");
      }
      await seeked;
      this.requireCurrentGeneration(token);
      if (
        Math.abs(media.currentTime * 1_000 - AUDIO_FIXTURE.seekCheckpointMs) >
        AUDIO_SEEK_TOLERANCE_MS
      ) {
        throw new AudioM1Error("SEEK_FAILED");
      }
      this.events.seekedCount += 1;
      this.observations.seekEventObserved = true;
      this.appendTransition("SEEK_CHECKPOINT", this.state, this.state);
      return this.changedSnapshot();
    } catch (error) {
      this.failOperation(error, token, this.state, "SEEK_FAILED");
    } finally {
      this.finishOperation(token);
    }
  }

  async stop(): Promise<AudioReceipt> {
    this.requireAvailable();
    this.requireForeground();
    this.requireState("PLAYING", "PAUSED", "ENDED");
    if (this.actionCounts.STOP > 0) throw new AudioM1Error("INVALID_TRANSITION");
    const media = this.requireMedia();
    const token = this.beginOperation();
    const before = this.state;
    try {
      media.pause();
      const seeked = this.waitForMediaEvent(
        media,
        "seeked",
        "error",
        "SEEK_FAILED",
        "SEEK_FAILED",
      );
      void seeked.catch(() => undefined);
      media.currentTime = 0;
      await seeked;
      this.requireCurrentGeneration(token);
      if (this.positionMs() !== 0) throw new AudioM1Error("SEEK_FAILED");
      this.events.seekedCount += 1;
      this.observations.stopResetObserved = true;
      this.state = "STOPPED";
      this.appendTransition("STOP", before, this.state);
      return this.changedSnapshot();
    } catch (error) {
      this.failOperation(error, token, before, "SEEK_FAILED");
    } finally {
      this.finishOperation(token);
    }
  }

  release(): AudioReceipt {
    this.requireAvailable();
    this.requireState("LOADED", "PLAYING", "PAUSED", "STOPPED", "ENDED");
    const before = this.state;
    this.invalidationError = "LIFECYCLE_INTERRUPTED";
    this.generation += 1;
    this.cleanupResources();
    this.state = "RELEASED";
    this.appendTransition("RELEASE", before, this.state);
    return this.changedSnapshot();
  }

  enterBackground(): AudioReceipt {
    if (this.appPhase === "BACKGROUND") return this.snapshot();
    if (
      this.state === "RELEASED" &&
      this.media === null &&
      this.objectUrl === null &&
      this.actionCounts.BACKGROUND > 0
    ) {
      this.resetSessionEvidence();
    }
    const before = this.state;
    const media = this.media;
    const pendingPlaybackWasUnpaused =
      media !== null &&
      !media.paused &&
      this.pendingPlayback?.generation === this.generation &&
      this.pendingPlayback.media === media;
    const wasPlaying = before === "PLAYING" || pendingPlaybackWasUnpaused;
    const hadResources = this.media !== null || this.objectUrl !== null;
    this.invalidationError = "LIFECYCLE_INTERRUPTED";
    this.generation += 1;
    this.busy = false;
    this.cleanupResources();
    this.state = "RELEASED";
    this.appPhase = "BACKGROUND";
    this.lifecycle.backgroundCount += 1;
    if (wasPlaying) this.lifecycle.automaticPauseCount += 1;
    if (hadResources) this.lifecycle.automaticReleaseCount += 1;
    this.appendTransition("BACKGROUND", before, this.state);
    return this.changedSnapshot();
  }

  enterForeground(): AudioReceipt {
    if (this.appPhase === "FOREGROUND") return this.snapshot();
    this.appPhase = "FOREGROUND";
    this.lifecycle.foregroundCount += 1;
    this.appendTransition("FOREGROUND", this.state, this.state);
    return this.changedSnapshot();
  }

  dispose(): void {
    this.invalidationError = "LIFECYCLE_INTERRUPTED";
    this.generation += 1;
    this.busy = false;
    this.cleanupResources();
    this.state = "RELEASED";
  }

  private async startPlayback(action: "PLAY" | "RESUME"): Promise<AudioReceipt> {
    const media = this.requireMedia();
    const before = this.state;
    const startedAtMs = this.positionMs();
    const token = this.beginOperation();
    this.pendingPlayback = { generation: token, media };
    try {
      const progress = this.waitForProgress(media, startedAtMs);
      void progress.catch(() => undefined);
      await this.waitForPlayStart(media);
      this.requireCurrentGeneration(token);
      const progressDeltaMs = await progress;
      this.requireCurrentGeneration(token);
      if (action === "PLAY") {
        this.observations.playProgressDeltaMs = progressDeltaMs;
      } else {
        this.observations.resumeProgressDeltaMs = progressDeltaMs;
      }
      this.state = "PLAYING";
      this.appendTransition(action, before, this.state);
      return this.changedSnapshot();
    } catch (error) {
      return this.failOperation(error, token, before, "PLAY_REJECTED");
    } finally {
      if (
        this.pendingPlayback?.generation === token &&
        this.pendingPlayback.media === media
      ) {
        this.pendingPlayback = null;
      }
      this.finishOperation(token);
    }
  }

  private beginOperation(): number {
    this.requireAvailable();
    this.busy = true;
    return this.generation;
  }

  private finishOperation(token: number): void {
    if (token === this.generation) this.busy = false;
  }

  private requireCurrentGeneration(token: number): void {
    if (token !== this.generation) throw new AudioM1Error(this.invalidationError);
  }

  private requireAvailable(): void {
    if (this.busy) throw new AudioM1Error("BUSY");
  }

  private requireForeground(): void {
    if (this.appPhase !== "FOREGROUND") throw new AudioM1Error("APP_BACKGROUND");
  }

  private requireState(...states: AudioState[]): void {
    if (!states.includes(this.state)) throw new AudioM1Error("INVALID_TRANSITION");
  }

  private requireMedia(): AudioPort {
    if (this.media === null) throw new AudioM1Error("INVALID_TRANSITION");
    return this.media;
  }

  private resetSessionEvidence(): void {
    this.seq = 0;
    this.actionCounts = initialActionCounts();
    this.trace = [
      {
        seq: 0,
        action: "INITIALIZE",
        before: "RELEASED",
        after: "RELEASED",
        positionMs: 0,
      },
    ];
    this.verification = {
      bytesAndSha256Matched: false,
      wavHeaderMatched: false,
      metadataDurationMatched: false,
    };
    this.observations = {
      playProgressDeltaMs: 0,
      pauseWindowMs: 0,
      pauseDriftMs: 0,
      resumeProgressDeltaMs: 0,
      seekEventObserved: false,
      stopResetObserved: false,
    };
    this.events = {
      loadedMetadataCount: 0,
      timeUpdateCount: 0,
      seekedCount: 0,
      endedCount: 0,
      mediaErrorCount: 0,
    };
  }

  private positionMs(): number {
    if (this.media === null) return 0;
    const position = Math.round(this.media.currentTime * 1_000);
    if (!Number.isFinite(position)) return 0;
    return Math.max(0, Math.min(AUDIO_FIXTURE.durationMs, position));
  }

  private playbackRetreatedFromLastTransition(positionMs = this.positionMs()): boolean {
    const previousPositionMs = this.trace.at(-1)?.positionMs;
    return (
      previousPositionMs === undefined ||
      positionMs + AUDIO_MONOTONIC_TOLERANCE_MS < previousPositionMs
    );
  }

  private appendTransition(action: AudioAction, before: AudioState, after: AudioState): void {
    if (this.trace.length >= AUDIO_TRACE_LIMIT) {
      throw new AudioM1Error("RECEIPT_INVALID");
    }
    const nextSeq = this.seq + 1;
    const positionMs = this.positionMs();
    this.seq = nextSeq;
    this.actionCounts[action] += 1;
    this.trace.push({
      seq: nextSeq,
      action,
      before,
      after,
      positionMs,
    });
  }

  private attachOwnedListener(
    media: AudioPort,
    type: string,
    listener: EventListener,
  ): () => void {
    const owned = { type, listener };
    media.addEventListener(type, listener);
    this.ownedListeners.push(owned);
    return () => {
      media.removeEventListener(type, listener);
      const index = this.ownedListeners.indexOf(owned);
      if (index >= 0) this.ownedListeners.splice(index, 1);
    };
  }

  private failOperation(
    error: unknown,
    token: number,
    before: AudioState,
    fallbackCode: AudioErrorCode,
  ): never {
    if (token !== this.generation) throw new AudioM1Error(this.invalidationError);
    const boundedError =
      error instanceof AudioM1Error ? error : new AudioM1Error(fallbackCode);
    this.invalidationError = boundedError.code;
    this.generation += 1;
    this.busy = false;
    this.cleanupResources();
    this.state = "RELEASED";
    this.appendTransition("PROBE_FAILURE", before, this.state);
    this.changedSnapshot();
    throw boundedError;
  }

  private waitForFixtureLoad(): Promise<FixtureVerification> {
    return new Promise((resolve, reject) => {
      const abortController = new AbortController();
      let settled = false;
      const cleanup = (): void => {
        this.pendingWaitAborts.delete(onAbort);
      };
      const onAbort = (): void => {
        if (settled) return;
        settled = true;
        cleanup();
        abortController.abort();
        reject(new AudioM1Error("LIFECYCLE_INTERRUPTED"));
      };
      const succeed = (verification: FixtureVerification): void => {
        if (settled) return;
        settled = true;
        cleanup();
        resolve(verification);
      };
      const fail = (error: unknown): void => {
        if (settled) return;
        settled = true;
        cleanup();
        reject(error);
      };

      this.pendingWaitAborts.set(onAbort, 0);
      try {
        void this.dependencies.loadFixture(abortController.signal).then(succeed, fail);
      } catch (error) {
        fail(error);
      }
    });
  }

  private waitForPlayStart(media: AudioPort): Promise<void> {
    return new Promise((resolve, reject) => {
      let timer: ReturnType<typeof setTimeout>;
      let settled = false;
      const cleanup = (): void => {
        this.dependencies.clearTimer(timer);
        this.pendingWaitAborts.delete(onAbort);
      };
      const finish = (outcome: "SUCCESS" | "FAILURE" | "ABORT" | "TIMEOUT"): void => {
        if (settled) return;
        settled = true;
        cleanup();
        if (outcome === "SUCCESS") resolve();
        else if (outcome === "ABORT") reject(new AudioM1Error("LIFECYCLE_INTERRUPTED"));
        else reject(new AudioM1Error("PLAY_REJECTED"));
      };
      const onAbort = (): void => finish("ABORT");

      this.pendingWaitAborts.set(onAbort, 0);
      timer = this.dependencies.setTimer(
        () => finish("TIMEOUT"),
        PLAYBACK_PROGRESS_TIMEOUT_MS,
      );
      try {
        void media.play().then(
          () => finish("SUCCESS"),
          () => finish("FAILURE"),
        );
      } catch {
        finish("FAILURE");
      }
    });
  }

  private waitForDelay(delayMs: number): Promise<void> {
    return new Promise((resolve, reject) => {
      let timer: ReturnType<typeof setTimeout>;
      let settled = false;
      const cleanup = (): void => {
        this.dependencies.clearTimer(timer);
        this.pendingWaitAborts.delete(onAbort);
      };
      const finish = (aborted: boolean): void => {
        if (settled) return;
        settled = true;
        cleanup();
        if (aborted) reject(new AudioM1Error("LIFECYCLE_INTERRUPTED"));
        else resolve();
      };
      const onAbort = (): void => finish(true);
      this.pendingWaitAborts.set(onAbort, 0);
      timer = this.dependencies.setTimer(() => finish(false), delayMs);
    });
  }

  private waitForProgress(media: AudioPort, startedAtMs: number): Promise<number> {
    return new Promise((resolve, reject) => {
      let timer: ReturnType<typeof setTimeout>;
      let settled = false;
      let lastPositionMs = startedAtMs;
      const cleanup = (): void => {
        this.dependencies.clearTimer(timer);
        media.removeEventListener("timeupdate", onTimeUpdate);
        media.removeEventListener("error", onFailure);
        media.removeEventListener("ended", onFailure);
        this.pendingWaitAborts.delete(onAbort);
      };
      const finish = (
        outcome: "SUCCESS" | "FAILURE" | "ABORT" | "TIMEOUT" | "NON_MONOTONIC",
        deltaMs = 0,
      ): void => {
        if (settled) return;
        settled = true;
        cleanup();
        if (outcome === "SUCCESS") resolve(deltaMs);
        else if (outcome === "ABORT") reject(new AudioM1Error("LIFECYCLE_INTERRUPTED"));
        else if (outcome === "TIMEOUT") reject(new AudioM1Error("PLAYBACK_NO_PROGRESS"));
        else if (outcome === "NON_MONOTONIC") {
          reject(new AudioM1Error("PLAYBACK_NON_MONOTONIC"));
        }
        else reject(new AudioM1Error("MEDIA_FAILURE"));
      };
      const checkProgress = (): void => {
        const positionMs = this.positionMs();
        if (positionMs + AUDIO_MONOTONIC_TOLERANCE_MS < lastPositionMs) {
          finish("NON_MONOTONIC");
          return;
        }
        lastPositionMs = Math.max(lastPositionMs, positionMs);
        const deltaMs = positionMs - startedAtMs;
        if (deltaMs >= AUDIO_PROGRESS_MIN_MS) finish("SUCCESS", deltaMs);
      };
      const onTimeUpdate: EventListener = () => checkProgress();
      const onFailure: EventListener = () => finish("FAILURE");
      const onAbort = (): void => finish("ABORT");
      media.addEventListener("timeupdate", onTimeUpdate);
      media.addEventListener("error", onFailure);
      media.addEventListener("ended", onFailure);
      this.pendingWaitAborts.set(onAbort, 3);
      timer = this.dependencies.setTimer(
        () => finish("TIMEOUT"),
        PLAYBACK_PROGRESS_TIMEOUT_MS,
      );
      checkProgress();
    });
  }

  private waitForMediaEvent(
    media: AudioPort,
    successType: string,
    failureType: string,
    failureCode: AudioErrorCode,
    timeoutCode: AudioErrorCode,
  ): Promise<void> {
    return new Promise((resolve, reject) => {
      let timer: ReturnType<typeof setTimeout>;
      let settled = false;
      const cleanup = (): void => {
        this.dependencies.clearTimer(timer);
        media.removeEventListener(successType, onSuccess);
        media.removeEventListener(failureType, onFailure);
        this.pendingWaitAborts.delete(onAbort);
      };
      const finish = (outcome: "SUCCESS" | "FAILURE" | "ABORT" | "TIMEOUT"): void => {
        if (settled) return;
        settled = true;
        cleanup();
        if (outcome === "SUCCESS") {
          resolve();
        } else if (outcome === "ABORT") {
          reject(new AudioM1Error("LIFECYCLE_INTERRUPTED"));
        } else if (outcome === "TIMEOUT") {
          reject(new AudioM1Error(timeoutCode));
        } else {
          reject(new AudioM1Error(failureCode));
        }
      };
      const onSuccess: EventListener = () => {
        finish("SUCCESS");
      };
      const onFailure: EventListener = () => {
        finish("FAILURE");
      };
      const onAbort = (): void => finish("ABORT");
      media.addEventListener(successType, onSuccess);
      media.addEventListener(failureType, onFailure);
      this.pendingWaitAborts.set(onAbort, 2);
      timer = this.dependencies.setTimer(() => {
        finish("TIMEOUT");
      }, MEDIA_EVENT_TIMEOUT_MS);
    });
  }

  private cleanupResources(): void {
    const media = this.media;
    this.pendingPlayback = null;
    for (const abort of [...this.pendingWaitAborts.keys()]) abort();
    this.pendingWaitAborts.clear();
    if (media !== null) {
      for (const { type, listener } of this.ownedListeners) {
        media.removeEventListener(type, listener);
      }
      this.ownedListeners = [];
      try {
        media.pause();
      } catch {
        // Release remains best-effort at the WebView boundary; OS proof is external.
      }
      try {
        media.removeAttribute("src");
        media.load();
      } catch {
        // The live reference is still dropped below.
      }
    }
    this.media = null;
    if (this.objectUrl !== null) {
      this.dependencies.revokeObjectUrl(this.objectUrl);
      this.objectUrl = null;
    }
  }

  private changedSnapshot(): AudioReceipt {
    const receipt = this.snapshot();
    this.dependencies.onChange?.(receipt);
    return receipt;
  }

  private readonly onTimeUpdate: EventListener = () => {
    this.events.timeUpdateCount += 1;
  };

  private readonly onEnded: EventListener = () => {
    if (this.busy || this.state !== "PLAYING") return;
    const before = this.state;
    if (this.playbackRetreatedFromLastTransition()) {
      this.invalidationError = "PLAYBACK_NON_MONOTONIC";
      this.generation += 1;
      this.cleanupResources();
      this.state = "RELEASED";
      this.appendTransition("PROBE_FAILURE", before, this.state);
      this.changedSnapshot();
      return;
    }
    this.events.endedCount += 1;
    this.state = "ENDED";
    this.appendTransition("ENDED", before, this.state);
    this.changedSnapshot();
  };

  private readonly onMediaError: EventListener = () => {
    if (this.media === null) return;
    const before = this.state;
    this.events.mediaErrorCount += 1;
    this.invalidationError = "MEDIA_FAILURE";
    this.generation += 1;
    this.busy = false;
    this.cleanupResources();
    this.state = "RELEASED";
    this.appendTransition("MEDIA_ERROR", before, this.state);
    this.changedSnapshot();
  };
}
