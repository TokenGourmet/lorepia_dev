import { describe, expect, it } from "vitest";

import {
  AUDIO_BACKEND,
  AUDIO_ACTIONS,
  AUDIO_FIXTURE,
  AUDIO_MAX_RECEIPT_BYTES,
  AUDIO_POLICY_VERSION,
  AudioReceiptProtocolError,
  parseAudioReceipt,
  type AudioReceipt,
  type AudioActionCounts,
  type AudioTransition,
} from "./audio-contract";

function actionCounts(overrides: Partial<AudioActionCounts> = {}): AudioActionCounts {
  const counts = {} as AudioActionCounts;
  for (const action of AUDIO_ACTIONS) counts[action] = 0;
  counts.INITIALIZE = 1;
  return { ...counts, ...overrides };
}

function initialReceipt(): AudioReceipt {
  const transition = {
    seq: 0,
    action: "INITIALIZE" as const,
    before: "RELEASED" as const,
    after: "RELEASED" as const,
    positionMs: 0,
  };
  return {
    protocolVersion: 1,
    policyVersion: AUDIO_POLICY_VERSION,
    backend: AUDIO_BACKEND,
    actionCounts: actionCounts(),
    fixture: AUDIO_FIXTURE,
    verification: {
      bytesAndSha256Matched: false,
      wavHeaderMatched: false,
      metadataDurationMatched: false,
    },
    observations: {
      playProgressDeltaMs: 0,
      pauseWindowMs: 0,
      pauseDriftMs: 0,
      resumeProgressDeltaMs: 0,
      seekEventObserved: false,
      stopResetObserved: false,
    },
    events: {
      loadedMetadataCount: 0,
      timeUpdateCount: 0,
      seekedCount: 0,
      endedCount: 0,
      mediaErrorCount: 0,
    },
    state: "RELEASED",
    appPhase: "FOREGROUND",
    positionMs: 0,
    transition,
    lifecycle: {
      backgroundCount: 0,
      foregroundCount: 0,
      automaticPauseCount: 0,
      automaticReleaseCount: 0,
      automaticResumeCount: 0,
    },
    resources: {
      elementAllocated: false,
      sourceAttached: false,
      objectUrlActive: false,
      listenerCount: 0,
      osResourceRelease: "UNVERIFIED_REQUIRES_DEVICE_EVIDENCE",
    },
    trace: [transition],
  };
}

function loadedReceipt(): AudioReceipt {
  const initial = initialReceipt();
  const loaded = {
    seq: 1,
    action: "LOAD" as const,
    before: "RELEASED" as const,
    after: "LOADED" as const,
    positionMs: 0,
  };
  return {
    ...initial,
    actionCounts: actionCounts({ LOAD: 1 }),
    verification: {
      bytesAndSha256Matched: true,
      wavHeaderMatched: true,
      metadataDurationMatched: true,
    },
    events: { ...initial.events, loadedMetadataCount: 1 },
    state: "LOADED",
    transition: loaded,
    resources: {
      ...initial.resources,
      elementAllocated: true,
      sourceAttached: true,
      objectUrlActive: true,
      listenerCount: 3,
    },
    trace: [initial.transition, loaded],
  };
}

function playingReceipt(): AudioReceipt {
  const loaded = loadedReceipt();
  const playing = {
    seq: 2,
    action: "PLAY" as const,
    before: "LOADED" as const,
    after: "PLAYING" as const,
    positionMs: 300,
  };
  return {
    ...loaded,
    actionCounts: actionCounts({ LOAD: 1, PLAY: 1 }),
    observations: { ...loaded.observations, playProgressDeltaMs: 300 },
    events: { ...loaded.events, timeUpdateCount: 1 },
    state: "PLAYING",
    positionMs: 300,
    transition: playing,
    trace: [...loaded.trace, playing],
  };
}

function clone(value: unknown): Record<string, unknown> {
  return JSON.parse(JSON.stringify(value)) as Record<string, unknown>;
}

function expectInvalid(value: unknown): void {
  expect(() => parseAudioReceipt(value)).toThrow(AudioReceiptProtocolError);
}

describe("strict audio receipt parser", () => {
  it("accepts the exact initial bounded receipt", () => {
    const receipt = initialReceipt();
    expect(parseAudioReceipt(receipt)).toEqual(receipt);
    expect(new TextEncoder().encode(JSON.stringify(receipt)).byteLength).toBeLessThan(
      AUDIO_MAX_RECEIPT_BYTES,
    );
  });

  it.each([
    ["protocol", { protocolVersion: 2 }],
    ["policy", { policyVersion: "m1-audio-playback-v2" }],
    ["backend", { backend: "RODIO" }],
    ["state", { state: "BUFFERING" }],
    ["phase", { appPhase: "SUSPENDED" }],
    ["fractional position", { positionMs: 1.5 }],
    ["oversized position", { positionMs: 12_001 }],
  ])("rejects a wrong %s", (_label, mutation) => {
    expectInvalid({ ...initialReceipt(), ...mutation });
  });

  it("rejects missing and extra top-level fields", () => {
    const missing = clone(initialReceipt());
    delete missing.resources;
    expectInvalid(missing);
    expectInvalid({ ...initialReceipt(), rawBrowserError: "device path and stack" });
  });

  it.each(Object.entries(AUDIO_FIXTURE))("pins fixture field %s", (key, value) => {
    const replacement = typeof value === "number" ? value + 1 : `${value}-changed`;
    expectInvalid({
      ...initialReceipt(),
      fixture: { ...AUDIO_FIXTURE, [key]: replacement },
    });
  });

  it("requires exact verification, lifecycle, resource, and transition keys", () => {
    expectInvalid({
      ...initialReceipt(),
      actionCounts: { ...initialReceipt().actionCounts, rawAction: 1 },
    });
    expectInvalid({
      ...initialReceipt(),
      verification: { ...initialReceipt().verification, decoderName: "native detail" },
    });
    expectInvalid({
      ...initialReceipt(),
      observations: { ...initialReceipt().observations, rawClock: 1 },
    });
    expectInvalid({
      ...initialReceipt(),
      events: { ...initialReceipt().events, nativeEvent: 1 },
    });
    expectInvalid({
      ...initialReceipt(),
      lifecycle: { ...initialReceipt().lifecycle, automaticResumeCount: 1 },
    });
    expectInvalid({
      ...initialReceipt(),
      resources: { ...initialReceipt().resources, nativeHandle: 1 },
    });
    expectInvalid({
      ...initialReceipt(),
      transition: { ...initialReceipt().transition, error: "raw" },
    });
  });

  it("requires a bounded contiguous trace ending in the named transition", () => {
    expectInvalid({ ...initialReceipt(), trace: [] });
    expectInvalid({
      ...initialReceipt(),
      trace: Array.from({ length: 17 }, (_, seq) => ({
        seq,
        action: "INITIALIZE",
        before: "RELEASED",
        after: "RELEASED",
        positionMs: 0,
      })),
    });

    const second = {
      seq: 2,
      action: "LOAD",
      before: "RELEASED",
      after: "LOADED",
      positionMs: 0,
    };
    expectInvalid({
      ...initialReceipt(),
      state: "LOADED",
      transition: second,
      trace: [initialReceipt().transition, second],
      resources: {
        ...initialReceipt().resources,
        elementAllocated: true,
        sourceAttached: true,
        objectUrlActive: true,
        listenerCount: 3,
      },
    });
    expectInvalid({
      ...initialReceipt(),
      transition: { ...initialReceipt().transition, seq: 1 },
    });

    const playing = playingReceipt();
    const forgedFirstPlay = { ...playing.transition, seq: 0 };
    expectInvalid({
      ...playing,
      transition: forgedFirstPlay,
      trace: [forgedFirstPlay],
    });
    expectInvalid({
      ...playing,
      transition: { ...playing.transition, seq: 2 },
      trace: [{ ...playing.transition, seq: 2 }],
    });

    const impossibleRolloverTrace: AudioTransition[] = Array.from(
      { length: 16 },
      (_, index) => {
        const seq = index + 1;
        const isPlay = seq % 2 === 1;
        return {
          seq,
          action: isPlay ? "PLAY" : "STOP",
          before: isPlay ? "STOPPED" : "PLAYING",
          after: isPlay ? "PLAYING" : "STOPPED",
          positionMs: isPlay ? 300 : 0,
        };
      },
    );
    impossibleRolloverTrace[0] = {
      ...impossibleRolloverTrace[0],
      before: "LOADED",
    };
    const lastImpossible = impossibleRolloverTrace.at(-1)!;
    expectInvalid({
      ...playing,
      state: "STOPPED",
      positionMs: 0,
      transition: lastImpossible,
      observations: { ...playing.observations, stopResetObserved: true },
      events: { ...playing.events, seekedCount: 8 },
      trace: impossibleRolloverTrace,
    });
  });

  it("binds media allocation to non-released states", () => {
    expectInvalid({
      ...initialReceipt(),
      resources: { ...initialReceipt().resources, elementAllocated: true },
    });
    expectInvalid({
      ...initialReceipt(),
      state: "LOADED",
      transition: { ...initialReceipt().transition, after: "LOADED" },
      trace: [{ ...initialReceipt().transition, after: "LOADED" }],
    });
    expectInvalid({
      ...initialReceipt(),
      resources: {
        ...initialReceipt().resources,
        elementAllocated: true,
        sourceAttached: true,
        objectUrlActive: false,
        listenerCount: 3,
      },
    });
  });

  it("accepts a fully evidenced play and rejects impossible or under-evidenced active states", () => {
    expect(parseAudioReceipt(loadedReceipt())).toEqual(loadedReceipt());
    expect(parseAudioReceipt(playingReceipt())).toEqual(playingReceipt());

    expectInvalid({
      ...playingReceipt(),
      verification: { ...playingReceipt().verification, metadataDurationMatched: false },
    });
    expectInvalid({
      ...playingReceipt(),
      resources: { ...playingReceipt().resources, listenerCount: 1 },
    });
    expectInvalid({
      ...playingReceipt(),
      observations: { ...playingReceipt().observations, playProgressDeltaMs: 0 },
    });
    const impossible = {
      ...playingReceipt().transition,
      action: "INITIALIZE",
      before: "RELEASED",
    };
    expectInvalid({
      ...playingReceipt(),
      transition: impossible,
      trace: [...loadedReceipt().trace, impossible],
    });
  });

  it("binds the live position to the last committed transition", () => {
    const playing = playingReceipt();
    expect(parseAudioReceipt({ ...playing, positionMs: 900 }).positionMs).toBe(900);
    expectInvalid({ ...playing, positionMs: 294 });

    const pausedTransition: AudioTransition = {
      seq: 3,
      action: "PAUSE",
      before: "PLAYING",
      after: "PAUSED",
      positionMs: 300,
    };
    const paused: AudioReceipt = {
      ...playing,
      actionCounts: actionCounts({ LOAD: 1, PLAY: 1, PAUSE: 1 }),
      observations: {
        ...playing.observations,
        pauseWindowMs: 500,
      },
      state: "PAUSED",
      transition: pausedTransition,
      trace: [...playing.trace, pausedTransition],
    };
    expect(parseAudioReceipt({ ...paused, positionMs: 400 }).positionMs).toBe(400);
    expectInvalid({ ...paused, positionMs: 401 });

    const toleratedPause = {
      ...pausedTransition,
      positionMs: 295,
    };
    expect(
      parseAudioReceipt({
        ...paused,
        positionMs: 295,
        transition: toleratedPause,
        trace: [...playing.trace, toleratedPause],
      }).positionMs,
    ).toBe(295);
    const retreatedPause = {
      ...pausedTransition,
      positionMs: 250,
    };
    expectInvalid({
      ...paused,
      positionMs: 250,
      transition: retreatedPause,
      trace: [...playing.trace, retreatedPause],
    });
  });

  it("binds persistent load and one-shot seek effects to cumulative action counts", () => {
    const loaded = loadedReceipt();
    const releasedTransition: AudioTransition = {
      seq: 2,
      action: "RELEASE",
      before: "LOADED",
      after: "RELEASED",
      positionMs: 0,
    };
    const released: AudioReceipt = {
      ...loaded,
      actionCounts: actionCounts({ LOAD: 1, RELEASE: 1 }),
      state: "RELEASED",
      positionMs: 0,
      transition: releasedTransition,
      resources: { ...initialReceipt().resources },
      trace: [...loaded.trace, releasedTransition],
    };
    expect(parseAudioReceipt(released)).toEqual(released);
    expectInvalid({
      ...released,
      verification: { ...initialReceipt().verification },
      events: { ...released.events, loadedMetadataCount: 0 },
    });

    const playing = playingReceipt();
    const paused: AudioTransition = {
      seq: 3,
      action: "PAUSE",
      before: "PLAYING",
      after: "PAUSED",
      positionMs: 300,
    };
    const firstSeek: AudioTransition = {
      seq: 4,
      action: "SEEK_CHECKPOINT",
      before: "PAUSED",
      after: "PAUSED",
      positionMs: AUDIO_FIXTURE.seekCheckpointMs,
    };
    const onceSeeked: AudioReceipt = {
      ...playing,
      actionCounts: actionCounts({
        LOAD: 1,
        PLAY: 1,
        PAUSE: 1,
        SEEK_CHECKPOINT: 1,
      }),
      observations: {
        ...playing.observations,
        pauseWindowMs: 500,
        seekEventObserved: true,
      },
      events: { ...playing.events, seekedCount: 1 },
      state: "PAUSED",
      positionMs: AUDIO_FIXTURE.seekCheckpointMs,
      transition: firstSeek,
      trace: [...playing.trace, paused, firstSeek],
    };
    expect(parseAudioReceipt(onceSeeked)).toEqual(onceSeeked);
    expectInvalid({
      ...onceSeeked,
      events: { ...onceSeeked.events, seekedCount: 0 },
    });

    const secondSeek: AudioTransition = { ...firstSeek, seq: 5 };
    const twiceSeeked: AudioReceipt = {
      ...playing,
      actionCounts: actionCounts({
        LOAD: 1,
        PLAY: 1,
        PAUSE: 1,
        SEEK_CHECKPOINT: 2,
      }),
      observations: {
        ...playing.observations,
        pauseWindowMs: 500,
        seekEventObserved: true,
      },
      events: { ...playing.events, seekedCount: 2 },
      state: "PAUSED",
      positionMs: AUDIO_FIXTURE.seekCheckpointMs,
      transition: secondSeek,
      trace: [...playing.trace, paused, firstSeek, secondSeek],
    };
    expectInvalid(twiceSeeked);
  });

  it("enforces foreground/background lifecycle counter balance", () => {
    expectInvalid({
      ...initialReceipt(),
      lifecycle: { ...initialReceipt().lifecycle, backgroundCount: 999 },
    });
    expectInvalid({
      ...initialReceipt(),
      lifecycle: {
        ...initialReceipt().lifecycle,
        automaticPauseCount: 1,
        automaticReleaseCount: 0,
      },
    });
  });

  it("requires background to be released with no auto-resume", () => {
    const backgroundTransition = {
      seq: 1,
      action: "BACKGROUND" as const,
      before: "RELEASED" as const,
      after: "RELEASED" as const,
      positionMs: 0,
    };
    const background = {
      ...initialReceipt(),
      actionCounts: actionCounts({ BACKGROUND: 1 }),
      appPhase: "BACKGROUND",
      transition: backgroundTransition,
      lifecycle: {
        ...initialReceipt().lifecycle,
        backgroundCount: 1,
      },
      trace: [initialReceipt().transition, backgroundTransition],
    };
    expect(parseAudioReceipt(background)).toEqual(background);
    expectInvalid({
      ...background,
      lifecycle: { ...background.lifecycle, foregroundCount: 1 },
    });
    expectInvalid({
      ...background,
      lifecycle: { ...background.lifecycle, automaticResumeCount: 1 },
    });
  });

  it("rejects cyclic and oversized values before reflection", () => {
    const cyclic: Record<string, unknown> = {};
    cyclic.self = cyclic;
    expectInvalid(cyclic);
    expectInvalid({ ...initialReceipt(), padding: "한".repeat(2_000) });
  });
});
