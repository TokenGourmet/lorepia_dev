import { describe, expect, it, vi } from "vitest";

import {
  AUDIO_FIXTURE,
  AudioReceiptProtocolError,
  parseAudioReceipt,
} from "./audio-contract";
import {
  AudioM1Controller,
  type AudioControllerDependencies,
  type AudioPort,
} from "./audio-controller";
import { AudioM1Error } from "./audio-error";
import type { FixtureVerification } from "./audio-fixture";

const FIXTURE_VERIFICATION: FixtureVerification = {
  bytes: new Uint8Array(AUDIO_FIXTURE.bytes),
  bytesAndSha256Matched: true,
  wavHeaderMatched: true,
};

class FakeAudio extends EventTarget implements AudioPort {
  preload = "";
  paused = true;
  duration = AUDIO_FIXTURE.durationMs / 1_000;
  loadCalls = 0;
  pauseCalls = 0;
  removeSourceCalls = 0;
  srcAssignments: string[] = [];
  emitMetadata = true;
  emitSeeked = true;
  loadFailure = false;
  seekFailure = false;
  playFailure = false;
  autoProgress = true;
  driftOnPause = false;
  pauseLeavesPlaying = false;
  playGate: Promise<void> | null = null;
  private readonly activeListeners = new Map<
    string,
    Set<EventListenerOrEventListenerObject>
  >();
  private source = "";
  private position = 0;

  override addEventListener(
    type: string,
    listener: EventListenerOrEventListenerObject | null,
    options?: boolean | AddEventListenerOptions,
  ): void {
    super.addEventListener(type, listener, options);
    if (listener === null) return;
    const listeners = this.activeListeners.get(type) ?? new Set();
    listeners.add(listener);
    this.activeListeners.set(type, listeners);
  }

  override removeEventListener(
    type: string,
    listener: EventListenerOrEventListenerObject | null,
    options?: boolean | EventListenerOptions,
  ): void {
    super.removeEventListener(type, listener, options);
    if (listener === null) return;
    this.activeListeners.get(type)?.delete(listener);
  }

  activeListenerCount(type?: string): number {
    if (type !== undefined) return this.activeListeners.get(type)?.size ?? 0;
    return [...this.activeListeners.values()].reduce(
      (count, listeners) => count + listeners.size,
      0,
    );
  }

  get src(): string {
    return this.source;
  }

  set src(value: string) {
    this.source = value;
    this.srcAssignments.push(value);
  }

  get currentTime(): number {
    return this.position;
  }

  set currentTime(value: number) {
    if (this.seekFailure) throw new Error("raw seek setter detail");
    this.position = value;
    if (this.emitSeeked) {
      queueMicrotask(() => this.dispatchEvent(new Event("seeked")));
    }
  }

  load(): void {
    if (this.loadFailure) throw new Error("raw load detail");
    this.loadCalls += 1;
    if (this.source.length > 0 && this.emitMetadata) {
      queueMicrotask(() => this.dispatchEvent(new Event("loadedmetadata")));
    }
  }

  async play(): Promise<void> {
    this.paused = false;
    if (this.playGate !== null) await this.playGate;
    if (this.playFailure) throw new Error("raw playback detail");
    if (this.autoProgress && !this.paused) {
      this.position += 0.3;
      this.dispatchEvent(new Event("timeupdate"));
    }
  }

  pause(): void {
    if (!this.pauseLeavesPlaying) this.paused = true;
    this.pauseCalls += 1;
    if (this.driftOnPause) this.position += 0.25;
  }

  removeAttribute(name: string): void {
    if (name === "src") {
      this.source = "";
      this.removeSourceCalls += 1;
    }
  }
}

function harness(overrides: Partial<AudioControllerDependencies> = {}) {
  const media = new FakeAudio();
  const revoked: string[] = [];
  const onChange = vi.fn();
  const timerDelays: number[] = [];
  const controller = new AudioM1Controller({
    loadFixture: async () => FIXTURE_VERIFICATION,
    createMedia: () => media,
    createObjectUrl: () => "blob:m1-audio-v1",
    revokeObjectUrl: (url) => revoked.push(url),
    setTimer: (callback, delayMs) => {
      timerDelays.push(delayMs);
      return setTimeout(callback, 0);
    },
    clearTimer: (handle) => clearTimeout(handle),
    onChange,
    ...overrides,
  });
  return { controller, media, revoked, onChange, timerDelays };
}

function expectCode(error: unknown, code: AudioM1Error["code"]): void {
  expect(error).toBeInstanceOf(AudioM1Error);
  expect((error as AudioM1Error).code).toBe(code);
  expect((error as AudioM1Error).message).not.toContain("raw");
}

describe("AudioM1Controller foreground contract", () => {
  it("runs load, play, pause, seek, resume, stop, and release in order", async () => {
    const { controller, media, revoked, timerDelays } = harness();

    expect(controller.snapshot()).toMatchObject({
      state: "RELEASED",
      positionMs: 0,
      resources: {
        elementAllocated: false,
        sourceAttached: false,
        objectUrlActive: false,
        listenerCount: 0,
      },
    });

    const loaded = await controller.load();
    expect(loaded).toMatchObject({
      state: "LOADED",
      verification: {
        bytesAndSha256Matched: true,
        wavHeaderMatched: true,
        metadataDurationMatched: true,
      },
      events: { loadedMetadataCount: 1 },
      resources: {
        elementAllocated: true,
        sourceAttached: true,
        objectUrlActive: true,
        listenerCount: 3,
      },
    });

    expect(await controller.play()).toMatchObject({
      state: "PLAYING",
      observations: { playProgressDeltaMs: 300 },
      events: { timeUpdateCount: 1 },
    });
    media.currentTime = 1.25;
    media.dispatchEvent(new Event("timeupdate"));
    expect(controller.snapshot().positionMs).toBe(1_250);

    expect(await controller.pause()).toMatchObject({
      state: "PAUSED",
      positionMs: 1_250,
      observations: { pauseWindowMs: 500, pauseDriftMs: 0 },
    });
    expect(media.paused).toBe(true);
    expect(controller.snapshot().positionMs).toBe(1_250);

    expect(await controller.seekCheckpoint()).toMatchObject({
      state: "PAUSED",
      positionMs: AUDIO_FIXTURE.seekCheckpointMs,
    });
    expect(await controller.resume()).toMatchObject({
      state: "PLAYING",
      observations: { resumeProgressDeltaMs: 300 },
    });
    await expect(controller.pause()).rejects.toMatchObject({ code: "INVALID_TRANSITION" });
    expect(await controller.stop()).toMatchObject({
      state: "STOPPED",
      positionMs: 0,
      observations: { stopResetObserved: true },
    });

    const released = controller.release();
    expect(released).toMatchObject({
      state: "RELEASED",
      positionMs: 0,
      resources: {
        elementAllocated: false,
        sourceAttached: false,
        objectUrlActive: false,
        listenerCount: 0,
        osResourceRelease: "UNVERIFIED_REQUIRES_DEVICE_EVIDENCE",
      },
    });
    expect(revoked).toEqual(["blob:m1-audio-v1"]);
    expect(media.removeSourceCalls).toBe(1);
    expect(media.srcAssignments).toEqual(["blob:m1-audio-v1"]);
    expect(media.loadCalls).toBe(2);
    expect(timerDelays).toContain(500);
    expect(released.trace.map((entry) => entry.action)).toEqual([
      "INITIALIZE",
      "LOAD",
      "PLAY",
      "PAUSE",
      "SEEK_CHECKPOINT",
      "RESUME",
      "STOP",
      "RELEASE",
    ]);
    expect(() =>
      parseAudioReceipt({
        ...released,
        observations: {
          ...released.observations,
          playProgressDeltaMs: AUDIO_FIXTURE.durationMs,
          resumeProgressDeltaMs: AUDIO_FIXTURE.durationMs,
        },
      }),
    ).toThrow(AudioReceiptProtocolError);
  });

  it("records natural end without dropping the releasable media object", async () => {
    const { controller, media } = harness();
    await controller.load();
    await controller.play();
    media.currentTime = 12;
    media.dispatchEvent(new Event("ended"));
    expect(controller.snapshot()).toMatchObject({
      state: "ENDED",
      positionMs: 12_000,
      transition: { action: "ENDED", before: "PLAYING", after: "ENDED" },
      events: { endedCount: 1 },
      resources: { elementAllocated: true },
    });
    expect(controller.release().state).toBe("RELEASED");
  });

  it("rejects invalid transitions with stable codes", async () => {
    const { controller } = harness();
    await expect(controller.play()).rejects.toSatisfy((error: unknown) => {
      expectCode(error, "INVALID_TRANSITION");
      return true;
    });
    await expect(controller.pause()).rejects.toBeInstanceOf(AudioM1Error);
    await controller.load();
    await expect(controller.resume()).rejects.toMatchObject({ code: "INVALID_TRANSITION" });
    await expect(controller.seekCheckpoint()).rejects.toMatchObject({
      code: "INVALID_TRANSITION",
    });
    await expect(controller.load()).rejects.toMatchObject({ code: "INVALID_TRANSITION" });
    controller.release();
    expect(() => controller.release()).toThrowError(
      expect.objectContaining({ code: "INVALID_TRANSITION" }),
    );
  });

  it("maps a browser play rejection to a bounded error", async () => {
    const { controller, media, onChange } = harness();
    await controller.load();
    media.playFailure = true;
    await expect(controller.play()).rejects.toMatchObject({
      name: "AudioM1Error",
      code: "PLAY_REJECTED",
      message: "Audio M-1 operation failed",
    });
    expect(controller.snapshot()).toMatchObject({
      state: "RELEASED",
      transition: { action: "PROBE_FAILURE", before: "LOADED", after: "RELEASED" },
    });
    expect(onChange).toHaveBeenLastCalledWith(
      expect.objectContaining({
        state: "RELEASED",
        transition: expect.objectContaining({ action: "PROBE_FAILURE" }),
      }),
    );
  });

  it("fails closed when media time never progresses", async () => {
    const { controller, media } = harness();
    await controller.load();
    media.autoProgress = false;
    await expect(controller.play()).rejects.toMatchObject({
      code: "PLAYBACK_NO_PROGRESS",
    });
    expect(controller.snapshot()).toMatchObject({
      state: "RELEASED",
      transition: { action: "PROBE_FAILURE", before: "LOADED", after: "RELEASED" },
      resources: { elementAllocated: false, listenerCount: 0 },
    });
  });

  it("fails closed when sampled playback time moves backward", async () => {
    const { controller, media } = harness();
    await controller.load();
    media.autoProgress = false;

    const playing = controller.play();
    await Promise.resolve();
    await Promise.resolve();
    media.currentTime = 0.2;
    media.dispatchEvent(new Event("timeupdate"));
    media.currentTime = 0.1;
    media.dispatchEvent(new Event("timeupdate"));
    media.currentTime = 0.3;
    media.dispatchEvent(new Event("timeupdate"));

    await expect(playing).rejects.toMatchObject({ code: "PLAYBACK_NON_MONOTONIC" });
    expect(controller.snapshot()).toMatchObject({
      state: "RELEASED",
      transition: { action: "PROBE_FAILURE" },
    });
  });

  it("fails closed when paused media time drifts beyond tolerance", async () => {
    const { controller, media } = harness();
    await controller.load();
    await controller.play();
    media.driftOnPause = true;
    await expect(controller.pause()).rejects.toMatchObject({ code: "PAUSE_UNSTABLE" });
    expect(controller.snapshot()).toMatchObject({
      state: "RELEASED",
      transition: { action: "PROBE_FAILURE", before: "PLAYING", after: "RELEASED" },
    });
  });

  it("fails a pause whose sampled excursion returns to its starting position", async () => {
    const { controller, media } = harness();
    await controller.load();
    await controller.play();
    const pausedAt = media.currentTime;

    const pausing = controller.pause();
    media.currentTime = pausedAt + 0.2;
    media.dispatchEvent(new Event("timeupdate"));
    media.currentTime = pausedAt;
    media.dispatchEvent(new Event("timeupdate"));

    await expect(pausing).rejects.toMatchObject({ code: "PAUSE_UNSTABLE" });
  });

  it("fails closed when playback retreats before pause commits", async () => {
    const { controller, media } = harness();
    await controller.load();
    await controller.play();
    media.currentTime = 0.25;

    await expect(controller.pause()).rejects.toMatchObject({
      code: "PLAYBACK_NON_MONOTONIC",
    });
    expect(controller.snapshot()).toMatchObject({
      state: "RELEASED",
      events: { endedCount: 0 },
      transition: { action: "PROBE_FAILURE", before: "PLAYING", after: "RELEASED" },
    });
  });

  it("turns a regressed ended event into a bounded probe failure", async () => {
    const { controller, media } = harness();
    await controller.load();
    await controller.play();
    media.currentTime = 0.25;

    media.dispatchEvent(new Event("ended"));
    expect(controller.snapshot()).toMatchObject({
      state: "RELEASED",
      events: { endedCount: 0 },
      transition: { action: "PROBE_FAILURE", before: "PLAYING", after: "RELEASED" },
      resources: { elementAllocated: false, listenerCount: 0 },
    });
  });

  it("requires the media element to report paused throughout the pause window", async () => {
    const { controller, media } = harness();
    await controller.load();
    await controller.play();
    media.pauseLeavesPlaying = true;

    await expect(controller.pause()).rejects.toMatchObject({ code: "PAUSE_UNSTABLE" });
    expect(controller.snapshot()).toMatchObject({ state: "RELEASED" });
  });

  it("ignores a queued ended event while a pause observation owns the state", async () => {
    const { controller, media } = harness();
    await controller.load();
    await controller.play();

    const pausing = controller.pause();
    media.dispatchEvent(new Event("ended"));
    const paused = await pausing;

    expect(paused).toMatchObject({
      state: "PAUSED",
      events: { endedCount: 0 },
      transition: { action: "PAUSE", before: "PLAYING", after: "PAUSED" },
    });
  });

  it("removes the in-flight pause observer before publishing background release", async () => {
    const activeTimers = new Map<
      ReturnType<typeof setTimeout>,
      { callback: () => void; delayMs: number }
    >();
    const { controller, media } = harness({
      setTimer: (callback, delayMs) => {
        const handle = setTimeout(() => undefined, 60_000);
        activeTimers.set(handle, { callback, delayMs });
        return handle;
      },
      clearTimer: (handle) => {
        clearTimeout(handle);
        activeTimers.delete(handle);
      },
    });
    await controller.load();
    await controller.play();

    const pausing = controller.pause();
    const interrupted = expect(pausing).rejects.toMatchObject({
      code: "LIFECYCLE_INTERRUPTED",
    });
    expect(media.activeListenerCount()).toBe(4);

    const background = controller.enterBackground();
    expect(background.resources.listenerCount).toBe(0);
    expect(media.activeListenerCount()).toBe(0);
    await interrupted;
    expect(activeTimers.size).toBe(0);
  });

  it("requires a real seeked event even when the media property changes", async () => {
    const { controller, media } = harness();
    await controller.load();
    await controller.play();
    await controller.pause();
    media.emitSeeked = false;
    await expect(controller.seekCheckpoint()).rejects.toMatchObject({ code: "SEEK_FAILED" });
    expect(controller.snapshot()).toMatchObject({
      state: "RELEASED",
      transition: { action: "PROBE_FAILURE", before: "PAUSED", after: "RELEASED" },
    });
  });

  it("handles synchronous load and seek setter failures without leaking pending waits", async () => {
    const loadHarness = harness();
    loadHarness.media.loadFailure = true;
    await expect(loadHarness.controller.load()).rejects.toMatchObject({
      code: "FIXTURE_LOAD_FAILED",
    });
    expect(loadHarness.controller.snapshot()).toMatchObject({
      state: "RELEASED",
      resources: { listenerCount: 0 },
    });

    const seekHarness = harness();
    await seekHarness.controller.load();
    await seekHarness.controller.play();
    await seekHarness.controller.pause();
    seekHarness.media.seekFailure = true;
    await expect(seekHarness.controller.seekCheckpoint()).rejects.toMatchObject({
      code: "SEEK_FAILED",
    });
    expect(seekHarness.controller.snapshot()).toMatchObject({
      state: "RELEASED",
      transition: { action: "PROBE_FAILURE" },
      resources: { listenerCount: 0 },
    });
  });

  it("handles a synchronous stop setter failure without leaking its pending wait", async () => {
    const { controller, media } = harness();
    await controller.load();
    await controller.play();
    media.seekFailure = true;
    await expect(controller.stop()).rejects.toMatchObject({ code: "SEEK_FAILED" });
    expect(controller.snapshot()).toMatchObject({
      state: "RELEASED",
      transition: { action: "PROBE_FAILURE" },
      resources: { listenerCount: 0 },
    });
  });

  it("rolls back a metadata mismatch and revokes the object URL", async () => {
    const { controller, media, revoked } = harness();
    media.duration = 11.5;
    await expect(controller.load()).rejects.toMatchObject({ code: "METADATA_MISMATCH" });
    expect(controller.snapshot()).toMatchObject({
      state: "RELEASED",
      resources: { elementAllocated: false, listenerCount: 0 },
    });
    expect(revoked).toEqual(["blob:m1-audio-v1"]);
  });

  it("treats a media error after load as a bounded release transition", async () => {
    const { controller, media } = harness();
    await controller.load();
    await controller.play();
    media.dispatchEvent(new Event("error"));
    expect(controller.snapshot()).toMatchObject({
      state: "RELEASED",
      transition: { action: "MEDIA_ERROR", before: "PLAYING", after: "RELEASED" },
      events: { mediaErrorCount: 1 },
      resources: { elementAllocated: false, listenerCount: 0 },
    });
  });
});

describe("AudioM1Controller lifecycle and bounds", () => {
  it("pauses and releases on background and never auto-resumes", async () => {
    const { controller, media, revoked } = harness();
    await controller.load();
    await controller.play();
    media.currentTime = 2;

    const background = controller.enterBackground();
    expect(background).toMatchObject({
      state: "RELEASED",
      appPhase: "BACKGROUND",
      positionMs: 0,
      lifecycle: {
        backgroundCount: 1,
        foregroundCount: 0,
        automaticPauseCount: 1,
        automaticReleaseCount: 1,
        automaticResumeCount: 0,
      },
      resources: { elementAllocated: false, listenerCount: 0 },
    });
    expect(media.paused).toBe(true);
    expect(revoked).toEqual(["blob:m1-audio-v1"]);

    expect(controller.enterBackground()).toEqual(background);
    const foreground = controller.enterForeground();
    expect(foreground).toMatchObject({
      state: "RELEASED",
      appPhase: "FOREGROUND",
      lifecycle: { backgroundCount: 1, foregroundCount: 1, automaticResumeCount: 0 },
    });
    await expect(controller.play()).rejects.toMatchObject({ code: "INVALID_TRANSITION" });
  });

  it("rejects every public playback action while backgrounded", async () => {
    const { controller } = harness();
    controller.enterBackground();
    await expect(controller.load()).rejects.toMatchObject({ code: "APP_BACKGROUND" });
    await expect(controller.play()).rejects.toMatchObject({ code: "APP_BACKGROUND" });
    await expect(controller.resume()).rejects.toMatchObject({ code: "APP_BACKGROUND" });
    await expect(controller.seekCheckpoint()).rejects.toMatchObject({ code: "APP_BACKGROUND" });
  });

  it("starts a fresh empty evidence epoch for a repeated released lifecycle cycle", () => {
    const { controller } = harness();
    controller.enterBackground();
    controller.enterForeground();

    const secondBackground = controller.enterBackground();
    expect(secondBackground).toMatchObject({
      appPhase: "BACKGROUND",
      lifecycle: { backgroundCount: 2, foregroundCount: 1 },
      actionCounts: { INITIALIZE: 1, BACKGROUND: 1, FOREGROUND: 0 },
    });
    expect(secondBackground.trace.map((entry) => entry.action)).toEqual([
      "INITIALIZE",
      "BACKGROUND",
    ]);

    expect(controller.enterForeground()).toMatchObject({
      appPhase: "FOREGROUND",
      lifecycle: { backgroundCount: 2, foregroundCount: 2 },
      actionCounts: { BACKGROUND: 1, FOREGROUND: 1 },
    });
  });

  it("cancels an in-flight fixture load on background", async () => {
    let fixtureSignal: AbortSignal | undefined;
    const fixtureGate = new Promise<FixtureVerification>(() => undefined);
    const { controller } = harness({
      loadFixture: (signal) => {
        fixtureSignal = signal;
        return fixtureGate;
      },
    });
    const loading = controller.load();
    const interrupted = expect(loading).rejects.toMatchObject({
      code: "LIFECYCLE_INTERRUPTED",
    });
    await expect(controller.pause()).rejects.toMatchObject({ code: "BUSY" });
    await expect(controller.load()).rejects.toMatchObject({ code: "BUSY" });
    controller.enterBackground();
    expect(fixtureSignal?.aborted).toBe(true);
    await interrupted;
    expect(controller.snapshot()).toMatchObject({
      appPhase: "BACKGROUND",
      state: "RELEASED",
      resources: { elementAllocated: false, listenerCount: 0 },
    });
  });

  it("classifies a rejecting in-flight play as lifecycle cancellation after background", async () => {
    let rejectPlay: ((reason: unknown) => void) | undefined;
    const playGate = new Promise<void>((_resolve, reject) => {
      rejectPlay = reject;
    });
    const { controller, media } = harness();
    await controller.load();
    media.playGate = playGate;
    const playing = controller.play();
    await Promise.resolve();
    controller.enterBackground();
    rejectPlay?.(new Error("browser AbortError detail"));
    await expect(playing).rejects.toMatchObject({ code: "LIFECYCLE_INTERRUPTED" });
    expect(controller.snapshot()).toMatchObject({
      appPhase: "BACKGROUND",
      state: "RELEASED",
      transition: { action: "BACKGROUND", before: "LOADED", after: "RELEASED" },
      lifecycle: { automaticPauseCount: 1, automaticReleaseCount: 1 },
    });
  });

  it("cancels a never-settling play promise immediately on background", async () => {
    const { controller, media } = harness();
    await controller.load();
    media.playGate = new Promise<void>(() => undefined);

    const playing = controller.play();
    await Promise.resolve();
    const background = controller.enterBackground();

    await expect(playing).rejects.toMatchObject({ code: "LIFECYCLE_INTERRUPTED" });
    expect(background).toMatchObject({
      state: "RELEASED",
      appPhase: "BACKGROUND",
      lifecycle: { automaticPauseCount: 1, automaticReleaseCount: 1 },
      resources: { listenerCount: 0 },
    });
  });

  it("starts the 2500 ms progress deadline before the play promise settles", async () => {
    const activeTimers = new Map<
      ReturnType<typeof setTimeout>,
      { callback: () => void; delayMs: number }
    >();
    const { controller, media } = harness({
      setTimer: (callback, delayMs) => {
        const handle = setTimeout(() => undefined, 60_000);
        activeTimers.set(handle, { callback, delayMs });
        return handle;
      },
      clearTimer: (handle) => {
        clearTimeout(handle);
        activeTimers.delete(handle);
      },
    });
    await controller.load();
    media.playGate = new Promise<void>(() => undefined);

    const playing = controller.play();
    await Promise.resolve();
    expect(
      [...activeTimers.values()].map(({ delayMs }) => delayMs).sort((a, b) => a - b),
    ).toEqual([2_500, 2_500]);
    for (const { callback } of [...activeTimers.values()]) callback();

    await expect(playing).rejects.toMatchObject({ code: "PLAY_REJECTED" });
    expect(activeTimers.size).toBe(0);
  });

  it("records an automatic pause when background interrupts progress confirmation", async () => {
    const { controller, media } = harness();
    await controller.load();
    media.autoProgress = false;

    const playing = controller.play();
    await Promise.resolve();
    expect(media.paused).toBe(false);

    const background = controller.enterBackground();
    await expect(playing).rejects.toMatchObject({ code: "LIFECYCLE_INTERRUPTED" });
    expect(background).toMatchObject({
      appPhase: "BACKGROUND",
      state: "RELEASED",
      transition: { action: "BACKGROUND", before: "LOADED", after: "RELEASED" },
      lifecycle: {
        backgroundCount: 1,
        automaticPauseCount: 1,
        automaticReleaseCount: 1,
      },
    });
    expect(media.paused).toBe(true);
    expect(media.pauseCalls).toBe(1);
  });

  it("keeps a complete bounded trace and rejects replay inside one diagnostic session", async () => {
    const { controller } = harness();
    await controller.load();
    await controller.play();
    await controller.stop();
    await expect(controller.play()).rejects.toMatchObject({ code: "INVALID_TRANSITION" });
    controller.release();
    const receipt = controller.snapshot();
    expect(receipt.trace).toHaveLength(5);
    expect(receipt.trace.at(-1)?.seq).toBe(4);
    expect(receipt.actionCounts).toMatchObject({
      INITIALIZE: 1,
      LOAD: 1,
      PLAY: 1,
      STOP: 1,
      RELEASE: 1,
    });
    expect(new TextEncoder().encode(JSON.stringify(receipt)).byteLength).toBeLessThanOrEqual(4_096);
    expect(() => parseAudioReceipt(JSON.parse(JSON.stringify(receipt)))).not.toThrow(
      AudioReceiptProtocolError,
    );
    expect(() => parseAudioReceipt({ ...receipt, trace: receipt.trace.slice(1) })).toThrow(
      AudioReceiptProtocolError,
    );
    expect(() =>
      parseAudioReceipt({
        ...receipt,
        observations: { ...receipt.observations, playProgressDeltaMs: 0 },
      }),
    ).toThrow(AudioReceiptProtocolError);
  });
});
