export const AUDIO_POLICY_VERSION = "m1-audio-playback-v1" as const;
export const AUDIO_BACKEND = "HTML_AUDIO_ELEMENT" as const;
export const AUDIO_MAX_RECEIPT_BYTES = 4_096 as const;
export const AUDIO_TRACE_LIMIT = 16 as const;
export const AUDIO_METADATA_TOLERANCE_MS = 50 as const;
export const AUDIO_PROGRESS_MIN_MS = 250 as const;
export const AUDIO_MONOTONIC_TOLERANCE_MS = 5 as const;
export const AUDIO_PAUSE_WINDOW_MS = 500 as const;
export const AUDIO_PAUSE_DRIFT_TOLERANCE_MS = 100 as const;
export const AUDIO_SEEK_TOLERANCE_MS = 100 as const;
export const AUDIO_OBSERVATION_POSITION_TOLERANCE_MS = 100 as const;

export const AUDIO_FIXTURE = {
  fixtureId: "m1-audio-v1",
  publicPath: "/fixtures/m1-audio-v1.wav",
  sha256: "8559ab4de943a983094b3e27af499ee5fbff80d48263a36fa3c0d1e1339ead25",
  bytes: 1_152_044,
  format: "wav-pcm-s16le",
  durationMs: 12_000,
  sampleRateHz: 48_000,
  channels: 1,
  bitsPerSample: 16,
  frameCount: 576_000,
  seekCheckpointMs: 6_000,
} as const;

export const AUDIO_STATES = [
  "RELEASED",
  "LOADED",
  "PLAYING",
  "PAUSED",
  "STOPPED",
  "ENDED",
] as const;

export const AUDIO_ACTIONS = [
  "INITIALIZE",
  "LOAD",
  "PLAY",
  "PAUSE",
  "RESUME",
  "SEEK_CHECKPOINT",
  "STOP",
  "RELEASE",
  "BACKGROUND",
  "FOREGROUND",
  "ENDED",
  "MEDIA_ERROR",
  "PROBE_FAILURE",
] as const;

export const AUDIO_ERROR_CODES = [
  "BUSY",
  "INVALID_TRANSITION",
  "APP_BACKGROUND",
  "FIXTURE_LOAD_FAILED",
  "FIXTURE_TOO_LARGE",
  "FIXTURE_MISMATCH",
  "FIXTURE_UNSUPPORTED",
  "METADATA_MISMATCH",
  "MEDIA_EVENT_TIMEOUT",
  "PLAY_REJECTED",
  "MEDIA_FAILURE",
  "PLAYBACK_NO_PROGRESS",
  "PLAYBACK_NON_MONOTONIC",
  "PAUSE_UNSTABLE",
  "SEEK_FAILED",
  "LIFECYCLE_INTERRUPTED",
  "RECEIPT_INVALID",
] as const;

export type AudioState = (typeof AUDIO_STATES)[number];
export type AudioAction = (typeof AUDIO_ACTIONS)[number];
export type AudioErrorCode = (typeof AUDIO_ERROR_CODES)[number];
export type AppPhase = "FOREGROUND" | "BACKGROUND";
export type AudioActionCounts = { [Action in AudioAction]: number };

export type AudioTransition = {
  seq: number;
  action: AudioAction;
  before: AudioState;
  after: AudioState;
  positionMs: number;
};

export type AudioReceipt = {
  protocolVersion: 1;
  policyVersion: typeof AUDIO_POLICY_VERSION;
  backend: typeof AUDIO_BACKEND;
  actionCounts: AudioActionCounts;
  fixture: typeof AUDIO_FIXTURE;
  verification: {
    bytesAndSha256Matched: boolean;
    wavHeaderMatched: boolean;
    metadataDurationMatched: boolean;
  };
  observations: {
    playProgressDeltaMs: number;
    pauseWindowMs: number;
    pauseDriftMs: number;
    resumeProgressDeltaMs: number;
    seekEventObserved: boolean;
    stopResetObserved: boolean;
  };
  events: {
    loadedMetadataCount: number;
    timeUpdateCount: number;
    seekedCount: number;
    endedCount: number;
    mediaErrorCount: number;
  };
  state: AudioState;
  appPhase: AppPhase;
  positionMs: number;
  transition: AudioTransition;
  lifecycle: {
    backgroundCount: number;
    foregroundCount: number;
    automaticPauseCount: number;
    automaticReleaseCount: number;
    automaticResumeCount: 0;
  };
  resources: {
    elementAllocated: boolean;
    sourceAttached: boolean;
    objectUrlActive: boolean;
    listenerCount: number;
    osResourceRelease: "UNVERIFIED_REQUIRES_DEVICE_EVIDENCE";
  };
  trace: AudioTransition[];
};

const RECEIPT_KEYS = [
  "actionCounts",
  "appPhase",
  "backend",
  "events",
  "fixture",
  "lifecycle",
  "observations",
  "policyVersion",
  "positionMs",
  "protocolVersion",
  "resources",
  "state",
  "trace",
  "transition",
  "verification",
] as const;
const FIXTURE_KEYS = [
  "bitsPerSample",
  "bytes",
  "channels",
  "durationMs",
  "fixtureId",
  "format",
  "frameCount",
  "publicPath",
  "sampleRateHz",
  "seekCheckpointMs",
  "sha256",
] as const;
const VERIFICATION_KEYS = [
  "bytesAndSha256Matched",
  "metadataDurationMatched",
  "wavHeaderMatched",
] as const;
const OBSERVATION_KEYS = [
  "pauseDriftMs",
  "pauseWindowMs",
  "playProgressDeltaMs",
  "resumeProgressDeltaMs",
  "seekEventObserved",
  "stopResetObserved",
] as const;
const EVENT_KEYS = [
  "endedCount",
  "loadedMetadataCount",
  "mediaErrorCount",
  "seekedCount",
  "timeUpdateCount",
] as const;
const TRANSITION_KEYS = ["action", "after", "before", "positionMs", "seq"] as const;
const LIFECYCLE_KEYS = [
  "automaticPauseCount",
  "automaticReleaseCount",
  "automaticResumeCount",
  "backgroundCount",
  "foregroundCount",
] as const;
const RESOURCE_KEYS = [
  "elementAllocated",
  "listenerCount",
  "objectUrlActive",
  "osResourceRelease",
  "sourceAttached",
] as const;

export class AudioReceiptProtocolError extends Error {
  constructor() {
    super("Audio receipt did not match the bounded M-1 protocol");
    this.name = "AudioReceiptProtocolError";
  }
}

function fail(): never {
  throw new AudioReceiptProtocolError();
}

function record(value: unknown): Record<string, unknown> {
  if (value === null || typeof value !== "object" || Array.isArray(value)) fail();
  return value as Record<string, unknown>;
}

function exactKeys(value: Record<string, unknown>, expected: readonly string[]): void {
  const actual = Object.keys(value).sort();
  const sortedExpected = [...expected].sort();
  if (
    actual.length !== sortedExpected.length ||
    actual.some((key, index) => key !== sortedExpected[index])
  ) {
    fail();
  }
}

function boundedInteger(value: unknown, maximum: number): number {
  if (!Number.isSafeInteger(value) || (value as number) < 0 || (value as number) > maximum) {
    fail();
  }
  return value as number;
}

function literalBoolean(value: unknown): boolean {
  if (typeof value !== "boolean") fail();
  return value;
}

function enumValue<T extends string>(value: unknown, choices: readonly T[]): T {
  if (typeof value !== "string" || !choices.includes(value as T)) fail();
  return value as T;
}

function parseTransition(value: unknown): AudioTransition {
  const input = record(value);
  exactKeys(input, TRANSITION_KEYS);
  return {
    seq: boundedInteger(input.seq, Number.MAX_SAFE_INTEGER),
    action: enumValue(input.action, AUDIO_ACTIONS),
    before: enumValue(input.before, AUDIO_STATES),
    after: enumValue(input.after, AUDIO_STATES),
    positionMs: boundedInteger(input.positionMs, AUDIO_FIXTURE.durationMs),
  };
}

function exactFixture(value: unknown): typeof AUDIO_FIXTURE {
  const input = record(value);
  exactKeys(input, FIXTURE_KEYS);
  for (const [key, expected] of Object.entries(AUDIO_FIXTURE)) {
    if (input[key] !== expected) fail();
  }
  return AUDIO_FIXTURE;
}

function parseActionCounts(value: unknown): AudioActionCounts {
  const input = record(value);
  exactKeys(input, AUDIO_ACTIONS);
  const counts = {} as AudioActionCounts;
  for (const action of AUDIO_ACTIONS) {
    counts[action] = boundedInteger(input[action], Number.MAX_SAFE_INTEGER);
  }
  return counts;
}

function sameTransition(left: AudioTransition, right: AudioTransition): boolean {
  return (
    left.seq === right.seq &&
    left.action === right.action &&
    left.before === right.before &&
    left.after === right.after &&
    left.positionMs === right.positionMs
  );
}

function validateTransition(transition: AudioTransition): void {
  if ((transition.seq === 0) !== (transition.action === "INITIALIZE")) fail();
  const atStart = transition.positionMs === 0;
  const hasProgress = transition.positionMs >= AUDIO_PROGRESS_MIN_MS;
  switch (transition.action) {
    case "INITIALIZE":
      if (
        transition.seq !== 0 ||
        transition.before !== "RELEASED" ||
        transition.after !== "RELEASED" ||
        !atStart
      ) fail();
      break;
    case "LOAD":
      if (
        transition.seq !== 1 ||
        transition.before !== "RELEASED" ||
        transition.after !== "LOADED" ||
        !atStart
      ) fail();
      break;
    case "PLAY":
      if (
        !(["LOADED", "STOPPED"] as AudioState[]).includes(transition.before) ||
        transition.after !== "PLAYING" ||
        !hasProgress
      ) fail();
      break;
    case "PAUSE":
      if (transition.before !== "PLAYING" || transition.after !== "PAUSED" || !hasProgress) fail();
      break;
    case "RESUME":
      if (transition.before !== "PAUSED" || transition.after !== "PLAYING" || !hasProgress) fail();
      break;
    case "SEEK_CHECKPOINT":
      if (
        transition.before !== "PAUSED" ||
        transition.after !== "PAUSED" ||
        Math.abs(transition.positionMs - AUDIO_FIXTURE.seekCheckpointMs) > AUDIO_SEEK_TOLERANCE_MS
      ) fail();
      break;
    case "STOP":
      if (
        !(["PLAYING", "PAUSED", "ENDED"] as AudioState[]).includes(transition.before) ||
        transition.after !== "STOPPED" ||
        !atStart
      ) fail();
      break;
    case "RELEASE":
      if (
        transition.before === "RELEASED" ||
        transition.after !== "RELEASED" ||
        !atStart
      ) fail();
      break;
    case "BACKGROUND":
      if (transition.after !== "RELEASED" || !atStart) fail();
      break;
    case "FOREGROUND":
      if (transition.before !== "RELEASED" || transition.after !== "RELEASED" || !atStart) fail();
      break;
    case "ENDED":
      if (
        transition.before !== "PLAYING" ||
        transition.after !== "ENDED" ||
        transition.positionMs < AUDIO_FIXTURE.durationMs - AUDIO_SEEK_TOLERANCE_MS
      ) fail();
      break;
    case "MEDIA_ERROR":
      if (
        transition.before === "RELEASED" ||
        transition.after !== "RELEASED" ||
        !atStart
      ) fail();
      break;
    case "PROBE_FAILURE":
      if (
        transition.before === "RELEASED" ||
        transition.after !== "RELEASED" ||
        !atStart
      ) fail();
      break;
  }
}

type TraceContext = {
  state: AudioState;
  appPhase: AppPhase;
};

const INITIAL_TRACE_CONTEXT: TraceContext = {
  state: "RELEASED",
  appPhase: "FOREGROUND",
};

function advanceVisibleTrace(
  context: TraceContext,
  transition: AudioTransition,
): TraceContext | null {
  if (context.state !== transition.before) return null;
  if (transition.action === "BACKGROUND") {
    if (context.appPhase !== "FOREGROUND") return null;
    return { state: "RELEASED", appPhase: "BACKGROUND" };
  }
  if (transition.action === "FOREGROUND") {
    if (context.appPhase !== "BACKGROUND") return null;
    return { state: "RELEASED", appPhase: "FOREGROUND" };
  }
  if (transition.action === "INITIALIZE" || context.appPhase !== "FOREGROUND") {
    return null;
  }
  return { state: transition.after, appPhase: "FOREGROUND" };
}

function validateTraceExecution(
  trace: AudioTransition[],
  expectedState: AudioState,
  expectedPhase: AppPhase,
): void {
  let context = INITIAL_TRACE_CONTEXT;
  for (let index = 1; index < trace.length; index += 1) {
    const next = advanceVisibleTrace(context, trace[index]);
    if (next === null) fail();
    context = next;
  }
  if (context.state !== expectedState || context.appPhase !== expectedPhase) fail();
}

export function parseAudioReceipt(value: unknown): AudioReceipt {
  let serialized: string;
  try {
    serialized = JSON.stringify(value);
  } catch {
    fail();
  }
  if (new TextEncoder().encode(serialized).byteLength > AUDIO_MAX_RECEIPT_BYTES) fail();

  const input = record(value);
  exactKeys(input, RECEIPT_KEYS);
  if (input.protocolVersion !== 1) fail();
  if (input.policyVersion !== AUDIO_POLICY_VERSION || input.backend !== AUDIO_BACKEND) fail();
  const actionCounts = parseActionCounts(input.actionCounts);
  const fixture = exactFixture(input.fixture);

  const verificationInput = record(input.verification);
  exactKeys(verificationInput, VERIFICATION_KEYS);
  const verification = {
    bytesAndSha256Matched: literalBoolean(verificationInput.bytesAndSha256Matched),
    wavHeaderMatched: literalBoolean(verificationInput.wavHeaderMatched),
    metadataDurationMatched: literalBoolean(verificationInput.metadataDurationMatched),
  };

  const observationInput = record(input.observations);
  exactKeys(observationInput, OBSERVATION_KEYS);
  const observations = {
    playProgressDeltaMs: boundedInteger(observationInput.playProgressDeltaMs, AUDIO_FIXTURE.durationMs),
    pauseWindowMs: boundedInteger(observationInput.pauseWindowMs, AUDIO_PAUSE_WINDOW_MS),
    pauseDriftMs: boundedInteger(observationInput.pauseDriftMs, AUDIO_FIXTURE.durationMs),
    resumeProgressDeltaMs: boundedInteger(
      observationInput.resumeProgressDeltaMs,
      AUDIO_FIXTURE.durationMs,
    ),
    seekEventObserved: literalBoolean(observationInput.seekEventObserved),
    stopResetObserved: literalBoolean(observationInput.stopResetObserved),
  };
  if (observations.pauseWindowMs !== 0 && observations.pauseWindowMs !== AUDIO_PAUSE_WINDOW_MS) fail();
  if (
    observations.pauseWindowMs === AUDIO_PAUSE_WINDOW_MS &&
    observations.pauseDriftMs > AUDIO_PAUSE_DRIFT_TOLERANCE_MS
  ) fail();

  const eventInput = record(input.events);
  exactKeys(eventInput, EVENT_KEYS);
  const events = {
    loadedMetadataCount: boundedInteger(eventInput.loadedMetadataCount, 1),
    timeUpdateCount: boundedInteger(eventInput.timeUpdateCount, 1_000_000),
    seekedCount: boundedInteger(eventInput.seekedCount, 1_000_000),
    endedCount: boundedInteger(eventInput.endedCount, 1_000_000),
    mediaErrorCount: boundedInteger(eventInput.mediaErrorCount, 1_000_000),
  };

  const state = enumValue(input.state, AUDIO_STATES);
  const appPhase = enumValue(input.appPhase, ["FOREGROUND", "BACKGROUND"] as const);
  const positionMs = boundedInteger(input.positionMs, AUDIO_FIXTURE.durationMs);
  const transition = parseTransition(input.transition);

  const lifecycleInput = record(input.lifecycle);
  exactKeys(lifecycleInput, LIFECYCLE_KEYS);
  const lifecycle = {
    backgroundCount: boundedInteger(lifecycleInput.backgroundCount, 1_000_000),
    foregroundCount: boundedInteger(lifecycleInput.foregroundCount, 1_000_000),
    automaticPauseCount: boundedInteger(lifecycleInput.automaticPauseCount, 1_000_000),
    automaticReleaseCount: boundedInteger(lifecycleInput.automaticReleaseCount, 1_000_000),
    automaticResumeCount: boundedInteger(lifecycleInput.automaticResumeCount, 0) as 0,
  };

  const resourcesInput = record(input.resources);
  exactKeys(resourcesInput, RESOURCE_KEYS);
  const resources = {
    elementAllocated: literalBoolean(resourcesInput.elementAllocated),
    sourceAttached: literalBoolean(resourcesInput.sourceAttached),
    objectUrlActive: literalBoolean(resourcesInput.objectUrlActive),
    listenerCount: boundedInteger(resourcesInput.listenerCount, 8),
    osResourceRelease: resourcesInput.osResourceRelease,
  };
  if (resources.osResourceRelease !== "UNVERIFIED_REQUIRES_DEVICE_EVIDENCE") fail();

  if (!Array.isArray(input.trace) || input.trace.length === 0 || input.trace.length > AUDIO_TRACE_LIMIT) {
    fail();
  }
  const trace = input.trace.map(parseTransition);
  for (const entry of trace) validateTransition(entry);
  for (let index = 1; index < trace.length; index += 1) {
    if (trace[index].seq !== trace[index - 1].seq + 1) fail();
    if (trace[index].before !== trace[index - 1].after) fail();
    if (
      trace[index].before === "PLAYING" &&
      (trace[index].action === "PAUSE" || trace[index].action === "ENDED") &&
      trace[index].positionMs + AUDIO_MONOTONIC_TOLERANCE_MS <
        trace[index - 1].positionMs
    ) {
      fail();
    }
  }
  const last = trace.at(-1);
  if (last === undefined || !sameTransition(last, transition)) fail();
  const first = trace[0];
  if (first.seq !== 0 || trace.length !== last.seq + 1) fail();
  if (
    first.seq === 0 &&
    (first.action !== "INITIALIZE" ||
      first.before !== "RELEASED" ||
      first.after !== "RELEASED" ||
      first.positionMs !== 0)
  ) fail();
  if (transition.after !== state) fail();
  validateTraceExecution(trace, state, appPhase);

  let totalActionCount = 0;
  const visibleActionCounts = {} as AudioActionCounts;
  for (const action of AUDIO_ACTIONS) visibleActionCounts[action] = 0;
  for (const entry of trace) visibleActionCounts[entry.action] += 1;
  for (const action of AUDIO_ACTIONS) {
    const count = actionCounts[action];
    if (totalActionCount > Number.MAX_SAFE_INTEGER - count) fail();
    totalActionCount += count;
    if (visibleActionCounts[action] !== count) fail();
  }
  if (actionCounts.INITIALIZE !== 1 || actionCounts.LOAD > 1) fail();
  for (const action of AUDIO_ACTIONS) {
    if (action !== "INITIALIZE" && actionCounts[action] > 1) fail();
  }
  if (totalActionCount !== last.seq + 1) fail();

  const hasSuccessfulLoad = actionCounts.LOAD === 1;
  const verificationValues = [
    verification.bytesAndSha256Matched,
    verification.wavHeaderMatched,
    verification.metadataDurationMatched,
  ];
  if (verificationValues.some((value) => value !== hasSuccessfulLoad)) fail();
  if (hasSuccessfulLoad && events.loadedMetadataCount !== 1) fail();

  const requiresSuccessfulLoad = [
    "PLAY",
    "PAUSE",
    "RESUME",
    "SEEK_CHECKPOINT",
    "STOP",
    "RELEASE",
    "ENDED",
    "MEDIA_ERROR",
    "PROBE_FAILURE",
  ] as const;
  if (
    !hasSuccessfulLoad &&
    requiresSuccessfulLoad.some((action) => actionCounts[action] > 0)
  ) fail();
  if (
    (actionCounts.PAUSE > 0 ||
      actionCounts.RESUME > 0 ||
      actionCounts.SEEK_CHECKPOINT > 0 ||
      actionCounts.STOP > 0 ||
      actionCounts.ENDED > 0) &&
    actionCounts.PLAY === 0
  ) fail();
  if (
    (actionCounts.RESUME > 0 || actionCounts.SEEK_CHECKPOINT > 0) &&
    actionCounts.PAUSE === 0
  ) fail();
  if (actionCounts.RESUME > actionCounts.PAUSE) fail();
  if (actionCounts.PAUSE > actionCounts.PLAY + actionCounts.RESUME) fail();
  if (actionCounts.STOP > actionCounts.PLAY) fail();
  if (actionCounts.PLAY > actionCounts.STOP + 1) fail();
  if (actionCounts.ENDED > actionCounts.PLAY + actionCounts.RESUME) fail();
  if (actionCounts.MEDIA_ERROR + actionCounts.PROBE_FAILURE > 1) fail();
  if (actionCounts.RELEASE > 1) fail();

  const backgroundBalance = actionCounts.BACKGROUND - actionCounts.FOREGROUND;
  if (backgroundBalance < 0 || backgroundBalance > 1) fail();
  if (appPhase === "BACKGROUND" && backgroundBalance !== 1) fail();
  if (appPhase === "FOREGROUND" && backgroundBalance !== 0) fail();

  if (actionCounts.PLAY > 0) {
    if (observations.playProgressDeltaMs < AUDIO_PROGRESS_MIN_MS || events.timeUpdateCount === 0) fail();
  } else if (observations.playProgressDeltaMs !== 0) fail();
  if (actionCounts.RESUME > 0) {
    if (observations.resumeProgressDeltaMs < AUDIO_PROGRESS_MIN_MS || events.timeUpdateCount === 0) fail();
  } else if (observations.resumeProgressDeltaMs !== 0) fail();
  if (actionCounts.PAUSE > 0) {
    if (
      observations.pauseWindowMs !== AUDIO_PAUSE_WINDOW_MS ||
      observations.pauseDriftMs > AUDIO_PAUSE_DRIFT_TOLERANCE_MS
    ) fail();
  } else if (observations.pauseWindowMs !== 0 || observations.pauseDriftMs !== 0) fail();
  if (observations.seekEventObserved !== (actionCounts.SEEK_CHECKPOINT > 0)) fail();
  if (observations.stopResetObserved !== (actionCounts.STOP > 0)) fail();
  if (events.seekedCount !== actionCounts.SEEK_CHECKPOINT + actionCounts.STOP) fail();
  if (events.endedCount !== actionCounts.ENDED) fail();
  if (events.mediaErrorCount !== actionCounts.MEDIA_ERROR) fail();

  const playIndex = trace.findIndex((entry) => entry.action === "PLAY");
  if (playIndex >= 0) {
    if (
      Math.abs(trace[playIndex].positionMs - observations.playProgressDeltaMs) >
      AUDIO_OBSERVATION_POSITION_TOLERANCE_MS
    ) fail();
  }
  const resumeIndex = trace.findIndex((entry) => entry.action === "RESUME");
  if (resumeIndex >= 0) {
    const resumeStart = trace[resumeIndex - 1]?.positionMs;
    if (resumeStart === undefined || trace[resumeIndex].positionMs < resumeStart) fail();
    const traceDelta = trace[resumeIndex].positionMs - resumeStart;
    if (
      Math.abs(traceDelta - observations.resumeProgressDeltaMs) >
      AUDIO_OBSERVATION_POSITION_TOLERANCE_MS
    ) fail();
  }

  if (!resources.elementAllocated) {
    if (resources.sourceAttached || resources.objectUrlActive || resources.listenerCount !== 0) fail();
  } else if (!resources.sourceAttached || !resources.objectUrlActive || resources.listenerCount !== 3) {
    fail();
  }
  if (state === "RELEASED" && resources.elementAllocated) fail();
  if (state !== "RELEASED" && !resources.elementAllocated) fail();
  if (state !== "RELEASED") {
    if (
      !verification.bytesAndSha256Matched ||
      !verification.wavHeaderMatched ||
      !verification.metadataDurationMatched ||
      events.loadedMetadataCount !== 1
    ) fail();
  }
  if ((state === "RELEASED" || state === "LOADED" || state === "STOPPED") && positionMs !== 0) fail();
  if ((state === "PLAYING" || state === "PAUSED") && positionMs < AUDIO_PROGRESS_MIN_MS) fail();
  if (state === "ENDED" && positionMs < AUDIO_FIXTURE.durationMs - AUDIO_SEEK_TOLERANCE_MS) fail();
  if (state === "PLAYING") {
    if (positionMs + AUDIO_MONOTONIC_TOLERANCE_MS < transition.positionMs) fail();
  } else if (
    Math.abs(positionMs - transition.positionMs) >
    AUDIO_OBSERVATION_POSITION_TOLERANCE_MS
  ) {
    fail();
  }
  if (appPhase === "BACKGROUND" && state !== "RELEASED") fail();
  if (appPhase === "BACKGROUND" && lifecycle.backgroundCount !== lifecycle.foregroundCount + 1) fail();
  if (appPhase === "FOREGROUND" && lifecycle.backgroundCount !== lifecycle.foregroundCount) fail();
  if (lifecycle.automaticPauseCount > lifecycle.backgroundCount) fail();
  if (lifecycle.automaticReleaseCount > lifecycle.backgroundCount) fail();
  if (lifecycle.automaticPauseCount > lifecycle.automaticReleaseCount) fail();
  const visibleBackgroundCount = visibleActionCounts.BACKGROUND;
  const visibleForegroundCount = visibleActionCounts.FOREGROUND;
  const visiblePlayingBackgroundCount = trace.filter(
    (entry) => entry.action === "BACKGROUND" && entry.before === "PLAYING",
  ).length;
  const visibleResourceBackgroundCount = trace.filter(
    (entry) => entry.action === "BACKGROUND" && entry.before !== "RELEASED",
  ).length;
  if (lifecycle.backgroundCount < visibleBackgroundCount) fail();
  if (lifecycle.foregroundCount < visibleForegroundCount) fail();
  if (lifecycle.automaticPauseCount < visiblePlayingBackgroundCount) fail();
  if (lifecycle.automaticReleaseCount < visibleResourceBackgroundCount) fail();
  if (lifecycle.backgroundCount < actionCounts.BACKGROUND) fail();
  if (lifecycle.foregroundCount < actionCounts.FOREGROUND) fail();

  return {
    protocolVersion: 1,
    policyVersion: AUDIO_POLICY_VERSION,
    backend: AUDIO_BACKEND,
    actionCounts,
    fixture,
    verification,
    observations,
    events,
    state,
    appPhase,
    positionMs,
    transition,
    lifecycle,
    resources: {
      ...resources,
      osResourceRelease: "UNVERIFIED_REQUIRES_DEVICE_EVIDENCE",
    },
    trace,
  };
}
