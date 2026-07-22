<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { onMount } from "svelte";

  import {
    ISOLATION_PROTOCOL_VERSION,
    safeIsolationMessageTypeHint,
    type IsolationErrorCode,
    type JsonObject,
  } from "$lib/isolation-protocol";
  import {
    createHostBrokerForwarder,
    type NativeBrokerForwardOutcome,
  } from "$lib/host-broker-forwarder";
  import { createFixedWindowAdmission } from "$lib/fixed-window-admission";
  import {
    beginHostBrokerRotation,
    finalizeHostBrokerRotationAfterAudit,
    persistHostBrokerToken,
    readHostBrokerToken,
    recoverHostBrokerRotation,
  } from "$lib/host-broker-session-journal";
  import {
    createIsolationSuiteRunGate,
    isValidSuiteRunId,
  } from "$lib/isolation-suite-run";
  import { auditRetiredRawCommands } from "$lib/native-command-audit";
  import {
    PluginWatchdog,
    type PluginWatchdogSnapshot,
  } from "$lib/plugin-watchdog";

  type TestResult = {
    passed: boolean;
    detail: string;
  };

  type RegisterHostBrokerSessionResponse = {
    outcome: "registered" | "idempotent";
    generation: number;
    moduleId: string;
    networkPolicy: "deny";
  };

  type RotateHostBrokerSessionResponse = {
    outcome: "rotated";
    generation: number;
    moduleId: string;
    networkPolicy: "deny";
  };

  type NativeBrokerResult =
    | { type: "state_read"; state: string }
    | {
        type: "render_sanitize";
        html: string;
        inputBytes: number;
        outputBytes: number;
      }
    | {
        type: "probe_increment";
        sentinel: string;
        callCount: number;
      };

  type HostBrokerRequestResponse = {
    requestId: string;
    moduleId: string;
    result: NativeBrokerResult;
  };

  type HostBrokerProbeCountResponse = {
    probeCallCount: number;
    sanitizeCallCount: number;
    commandSurfaceVersion: number;
    commandNames: string[];
    commandSha256: string;
  };

  type NativeSecurityAudit = {
    probeCallCount: number;
    sanitizeCallCount: number;
  };

  class NativeBrokerDenied extends Error {
    readonly code: IsolationErrorCode;

    constructor(code: IsolationErrorCode, message: string) {
      super(message);
      this.name = "NativeBrokerDenied";
      this.code = code;
    }
  }

  const PLUGIN_TEST_IDS = [
    "parent-document-blocked",
    "local-storage-blocked",
    "host-token-storage-inaccessible",
    "window-open-blocked",
    "external-fetch-csp-blocked",
    "direct-broker-missing-token-denied",
    "direct-broker-wrong-token-denied",
    "direct-registration-takeover-denied",
    "broker-state-read",
    "broker-render-sanitize",
    "broker-probe-increment",
    "broker-network-denied",
    "broker-secret-permission-denied",
    "broker-replay-rejected",
    "broker-unknown-field-rejected",
  ] as const;

  const NATIVE_COMMAND_AUDIT_TEST_ID = "retired-raw-command-denied";
  const AUDIT_TEST_ID = "native-side-effect-audit";
  const STALE_TOKEN_AUDIT_TEST_ID = "stale-host-token-denied";
  const TEST_IDS: readonly string[] = [
    STALE_TOKEN_AUDIT_TEST_ID,
    ...PLUGIN_TEST_IDS,
    NATIVE_COMMAND_AUDIT_TEST_ID,
    AUDIT_TEST_ID,
  ];
  const PLUGIN_TEST_ID_SET = new Set<string>(PLUGIN_TEST_IDS);
  const suiteRunGate = createIsolationSuiteRunGate(PLUGIN_TEST_IDS);
  const EXPECTED_NATIVE_COMMAND_NAMES = [
    "ack_stream",
    "cancel_stream",
    "get_stream_snapshot",
    "host_broker_probe_count",
    "host_broker_request",
    "register_host_broker_session",
    "release_stream",
    "rotate_host_broker_session",
    "start_mock_stream",
    "wait_stream_terminal",
  ] as const;
  const EXPECTED_NATIVE_COMMAND_SHA256 =
    "679411179e22a191fe48f8fdc503c62d6d302a888aba93fe1606c9a553bc57ce";
  const MODULE_ID = "fixture.plugin";
  const MANIFEST_PERMISSIONS = [
    "state.read",
    "render.sanitize",
    "probe.increment",
    "network.fetch",
    "secret.read",
  ] as const;
  const APPROVED_PERMISSIONS = [
    "state.read",
    "render.sanitize",
    "probe.increment",
    "network.fetch",
  ] as const;
  const FORWARDABLE_NATIVE_ERROR_CODES = new Set<IsolationErrorCode>([
    "MALFORMED_REQUEST",
    "REPLAYED_REQUEST",
    "RATE_LIMITED",
    "SESSION_EXHAUSTED",
    "UNKNOWN_METHOD",
    "PERMISSION_DENIED",
    "NETWORK_DENIED",
    "INVALID_PAYLOAD",
  ]);
  const WATCHDOG_PING_INTERVAL_MS = 250;
  const WATCHDOG_TIMEOUT_MS = 300;
  const WATCHDOG_MISSED_DEADLINE_THRESHOLD = 2;
  const NATIVE_INVOKE_TIMEOUT_MS = 2_000;
  const FRAME_MESSAGE_MAX_ATTEMPTS_PER_WINDOW = 64;
  const FRAME_MESSAGE_ATTEMPT_WINDOW_MS = 1_000;
  const IMPORTED_CODE_EXECUTION_ENABLED =
    __LOREPIA_BUILD_PROFILE__.importedJavaScriptFixtureAllowed &&
    __LOREPIA_BUILD_PROFILE__.importedLuaFixtureAllowed;
  const hostBrokerForwarder = createHostBrokerForwarder();
  const frameMessageAdmission = createFixedWindowAdmission({
    maxAttempts: FRAME_MESSAGE_MAX_ATTEMPTS_PER_WINDOW,
    windowMs: FRAME_MESSAGE_ATTEMPT_WINDOW_MS,
    clock: () => performance.now(),
  });

  let frame = $state<HTMLIFrameElement | null>(null);
  let sessionNonce = $state("");
  let hostBrokerReady = $state(false);
  let frameReady = $state(false);
  let frameDisabled = $state(false);
  let suiteRunning = $state(false);
  let results = $state<Record<string, TestResult>>({});
  let statusMessage = $state("격리 프레임을 준비하는 중입니다.");
  let watchdog: PluginWatchdog | null = null;
  let watchdogSnapshot = $state<PluginWatchdogSnapshot | null>(null);
  let hostProbeControl = $state<TestResult | null>(null);
  let probeCountBefore = $state<number | null>(null);
  let probeCountAfter = $state<number | null>(null);
  let sanitizeCountBefore = $state<number | null>(null);
  let sanitizeCountAfter = $state<number | null>(null);
  let pingTimer: number | null = null;
  let deadlineTimer: number | null = null;
  let removeMessageListener: (() => void) | null = null;
  let hostToken: string | null = null;
  let hostGeneration = $state<number | null>(null);
  let brokerRotating = $state(false);
  let staleTokenAudit = $state<TestResult | null>(null);
  let componentDisposed = false;

  let completedCount = $derived(
    TEST_IDS.filter((testId) => Object.prototype.hasOwnProperty.call(results, testId)).length,
  );
  let passedCount = $derived(
    Object.values(results).filter((result) => result.passed).length,
  );

  function randomHex(bytes: number): string {
    const values = new Uint8Array(bytes);
    crypto.getRandomValues(values);
    return Array.from(values, (value) => value.toString(16).padStart(2, "0")).join("");
  }

  function recordStaleTokenAudit(result: TestResult): void {
    staleTokenAudit = result;
    results = {
      ...results,
      [STALE_TOKEN_AUDIT_TEST_ID]: result,
    };
  }

  function invalidateSuiteRun(): void {
    suiteRunGate.invalidate();
    suiteRunning = false;
  }

  function isActiveSuiteRun(runId: string): boolean {
    return (
      !componentDisposed &&
      suiteRunning &&
      suiteRunGate.isActive(runId)
    );
  }

  function finishSuiteRun(runId: string): void {
    if (suiteRunGate.finish(runId)) suiteRunning = false;
  }

  function isPlainRecord(value: unknown): value is Record<string, unknown> {
    return (
      typeof value === "object" &&
      value !== null &&
      !Array.isArray(value) &&
      Object.getPrototypeOf(value) === Object.prototype
    );
  }

  function hasExactKeys(
    value: Record<string, unknown>,
    expected: readonly string[],
  ): boolean {
    const keys = Object.keys(value);
    return (
      keys.length === expected.length &&
      expected.every((key) => Object.prototype.hasOwnProperty.call(value, key))
    );
  }

  function getOrCreateHostToken(): string {
    const existing = readHostBrokerToken(window.sessionStorage);
    if (existing !== null) {
      return existing;
    }

    const created = randomHex(32);
    persistHostBrokerToken(window.sessionStorage, created);
    return created;
  }

  function parseNativeBrokerError(
    error: unknown,
  ): { code: string; message: string; requestId: string | null } | null {
    let candidate = error;
    if (typeof candidate === "string" && candidate.startsWith("{")) {
      try {
        candidate = JSON.parse(candidate);
      } catch {
        return null;
      }
    }
    if (
      !isPlainRecord(candidate) ||
      !(
        hasExactKeys(candidate, ["code", "message"]) ||
        hasExactKeys(candidate, ["code", "message", "requestId"])
      ) ||
      typeof candidate.code !== "string" ||
      typeof candidate.message !== "string" ||
      (Object.prototype.hasOwnProperty.call(candidate, "requestId") &&
        typeof candidate.requestId !== "string")
    ) {
      return null;
    }
    return {
      code: candidate.code,
      message: candidate.message.slice(0, 256),
      requestId:
        typeof candidate.requestId === "string" ? candidate.requestId : null,
    };
  }

  function nativeBrokerFailure(error: unknown, expectedRequestId: string): never {
    const parsed = parseNativeBrokerError(error);
    if (
      parsed !== null &&
      parsed.requestId !== null &&
      parsed.requestId !== expectedRequestId
    ) {
      throw new Error("native broker error request id mismatch");
    }
    if (
      parsed !== null &&
      FORWARDABLE_NATIVE_ERROR_CODES.has(parsed.code as IsolationErrorCode)
    ) {
      throw new NativeBrokerDenied(
        parsed.code as IsolationErrorCode,
        parsed.message,
      );
    }
    throw new Error("native host broker request failed");
  }

  async function invokeHostBrokerJson(
    token: string,
    requestId: string,
    requestJson: string,
  ): Promise<HostBrokerRequestResponse> {
    let response: unknown;
    try {
      response = await invoke("host_broker_request", {
        hostToken: token,
        requestJson,
      });
    } catch (error) {
      nativeBrokerFailure(error, requestId);
    }

    if (
      !isPlainRecord(response) ||
      !hasExactKeys(response, ["requestId", "moduleId", "result"]) ||
      response.requestId !== requestId ||
      response.moduleId !== MODULE_ID ||
      !isPlainRecord(response.result) ||
      typeof response.result.type !== "string"
    ) {
      throw new Error("native host broker returned an invalid response");
    }
    return response as HostBrokerRequestResponse;
  }

  async function invokeHostBroker(
    token: string,
    requestId: string,
    method: string,
    payload: JsonObject,
  ): Promise<HostBrokerRequestResponse> {
    return invokeHostBrokerJson(
      token,
      requestId,
      JSON.stringify({ request_id: requestId, method, payload }),
    );
  }

  async function readNativeSecurityAudit(): Promise<NativeSecurityAudit> {
    const response = await invoke<HostBrokerProbeCountResponse>(
      "host_broker_probe_count",
    );
    if (
      !isPlainRecord(response) ||
      !hasExactKeys(response, [
        "probeCallCount",
        "sanitizeCallCount",
        "commandSurfaceVersion",
        "commandNames",
        "commandSha256",
      ]) ||
      typeof response.probeCallCount !== "number" ||
      !Number.isSafeInteger(response.probeCallCount) ||
      response.probeCallCount < 0 ||
      typeof response.sanitizeCallCount !== "number" ||
      !Number.isSafeInteger(response.sanitizeCallCount) ||
      response.sanitizeCallCount < 0 ||
      response.commandSurfaceVersion !== 3 ||
      !Array.isArray(response.commandNames) ||
      response.commandNames.length !== EXPECTED_NATIVE_COMMAND_NAMES.length ||
      !EXPECTED_NATIVE_COMMAND_NAMES.every(
        (command, index) => response.commandNames[index] === command,
      ) ||
      response.commandSha256 !== EXPECTED_NATIVE_COMMAND_SHA256
    ) {
      throw new Error("native security contract returned an invalid response");
    }
    return {
      probeCallCount: response.probeCallCount,
      sanitizeCallCount: response.sanitizeCallCount,
    };
  }

  function validGeneration(value: unknown): value is number {
    return typeof value === "number" && Number.isSafeInteger(value) && value >= 1;
  }

  async function registerHostBroker(token: string): Promise<number> {
    const response = await invoke<RegisterHostBrokerSessionResponse>(
      "register_host_broker_session",
      {
        hostToken: token,
        policy: {
          moduleId: MODULE_ID,
          manifestPermissions: [...MANIFEST_PERMISSIONS],
          approvedPermissions: [...APPROVED_PERMISSIONS],
        },
      },
    );
    if (
      !isPlainRecord(response) ||
      !hasExactKeys(response, ["outcome", "generation", "moduleId", "networkPolicy"]) ||
      (response.outcome !== "registered" && response.outcome !== "idempotent") ||
      !validGeneration(response.generation) ||
      response.moduleId !== MODULE_ID ||
      response.networkPolicy !== "deny"
    ) {
      throw new Error("native host broker registration response is invalid");
    }
    return response.generation;
  }

  async function rotateHostBroker(
    currentToken: string,
    nextToken: string,
    expectedGeneration: number,
  ): Promise<number> {
    const response = await invoke<RotateHostBrokerSessionResponse>(
      "rotate_host_broker_session",
      {
        currentHostToken: currentToken,
        nextHostToken: nextToken,
        expectedGeneration,
      },
    );
    if (
      !isPlainRecord(response) ||
      !hasExactKeys(response, ["outcome", "generation", "moduleId", "networkPolicy"]) ||
      response.outcome !== "rotated" ||
      response.generation !== expectedGeneration + 1 ||
      !validGeneration(response.generation) ||
      response.moduleId !== MODULE_ID ||
      response.networkPolicy !== "deny"
    ) {
      throw new Error("native host broker rotation response is invalid");
    }
    return response.generation;
  }

  async function invokeWithTimeout<T>(
    promise: Promise<T>,
    label: string,
  ): Promise<T> {
    let timer: number | null = null;
    try {
      return await Promise.race([
        promise,
        new Promise<never>((_resolve, reject) => {
          timer = window.setTimeout(
            () => reject(new Error(`${label} timed out`)),
            NATIVE_INVOKE_TIMEOUT_MS,
          );
        }),
      ]);
    } finally {
      if (timer !== null) window.clearTimeout(timer);
    }
  }

  async function auditStaleHostToken(staleToken: string): Promise<TestResult> {
    const before = await readNativeSecurityAudit();
    const requestId = `host-stale-${randomHex(8)}`;
    try {
      await invokeWithTimeout(
        invoke("host_broker_request", {
          hostToken: staleToken,
          requestJson: JSON.stringify({
            request_id: requestId,
            method: "probe.increment",
            payload: {},
          }),
        }),
        "stale host token audit",
      );
      return {
        passed: false,
        detail: "rotated host token unexpectedly reached the native broker",
      };
    } catch (error) {
      const parsed = parseNativeBrokerError(error);
      const after = await readNativeSecurityAudit();
      const sinksUnchanged =
        before.probeCallCount === after.probeCallCount &&
        before.sanitizeCallCount === after.sanitizeCallCount;
      const passed = parsed?.code === "INVALID_HOST_TOKEN" && sinksUnchanged;
      return {
        passed,
        detail: passed
          ? "old generation token returned INVALID_HOST_TOKEN with no sink change"
          : `stale-token audit failed: ${parsed?.code ?? String(error)}`,
      };
    }
  }

  async function auditStaleTokenBeforeFinalize(
    staleToken: string,
  ): Promise<boolean> {
    try {
      const audit = await auditStaleHostToken(staleToken);
      if (componentDisposed) return false;
      recordStaleTokenAudit(audit);
      return audit.passed;
    } catch (error) {
      if (componentDisposed) return false;
      recordStaleTokenAudit({
        passed: false,
        detail: `stale-token audit unavailable: ${String(error).slice(0, 192)}`,
      });
      return false;
    }
  }

  function clearWatchdogTimers(): void {
    if (pingTimer !== null) window.clearTimeout(pingTimer);
    if (deadlineTimer !== null) window.clearTimeout(deadlineTimer);
    pingTimer = null;
    deadlineTimer = null;
  }

  function disableFrame(): void {
    clearWatchdogTimers();
    invalidateSuiteRun();
    frameDisabled = true;
    frameReady = false;
    statusMessage = "워치독이 응답 없는 플러그인을 비활성화했습니다.";
    if (frame !== null) frame.src = "about:blank";
  }

  function updateWatchdogSnapshot(): void {
    if (watchdog !== null) watchdogSnapshot = watchdog.snapshot;
  }

  function scheduleWatchdogPing(delayMs = WATCHDOG_PING_INTERVAL_MS): void {
    if (watchdog === null || frameDisabled) return;
    if (pingTimer !== null) window.clearTimeout(pingTimer);
    pingTimer = window.setTimeout(() => {
      pingTimer = null;
      const activeWatchdog = watchdog;
      const target = frame?.contentWindow;
      if (activeWatchdog === null || target === null || target === undefined) return;

      const issued = activeWatchdog.issuePing();
      updateWatchdogSnapshot();
      if (!issued.issued) {
        if (issued.reason !== "disabled") scheduleWatchdogPing();
        return;
      }

      target.postMessage(issued.ping.message, "*");
      const waitMs = Math.max(0, issued.ping.deadlineAtMs - performance.now() + 1);
      deadlineTimer = window.setTimeout(() => {
        deadlineTimer = null;
        if (watchdog !== activeWatchdog) return;
        activeWatchdog.checkDeadline();
        updateWatchdogSnapshot();
        if (activeWatchdog.snapshot.state.status !== "disabled") {
          scheduleWatchdogPing();
        }
      }, waitMs);
    }, delayMs);
  }

  function startFrameRuntime(): void {
    if (hostToken === null || hostGeneration === null || !hostBrokerReady || brokerRotating) {
      throw new Error("host broker must be registered before the iframe starts");
    }

    watchdog = new PluginWatchdog({
      clock: { now: () => performance.now() },
      sessionIdFactory: () => sessionNonce,
      pingTimeoutMs: WATCHDOG_TIMEOUT_MS,
      missedDeadlineThreshold: WATCHDOG_MISSED_DEADLINE_THRESHOLD,
      onDisabled: disableFrame,
    });
    updateWatchdogSnapshot();
    scheduleWatchdogPing(0);
  }

  function isReadyMessage(value: unknown): boolean {
    return (
      isPlainRecord(value) &&
      hasExactKeys(value, ["type", "sessionNonce"]) &&
      value.type === "lorepia:plugin:ready" &&
      value.sessionNonce === sessionNonce
    );
  }

  function acceptTestResult(value: unknown): boolean {
    if (
      !isPlainRecord(value) ||
      !hasExactKeys(value, [
        "type",
        "sessionNonce",
        "runId",
        "testId",
        "passed",
        "detail",
      ]) ||
      value.type !== "lorepia:plugin:test-result" ||
      value.sessionNonce !== sessionNonce ||
      !isValidSuiteRunId(value.runId) ||
      typeof value.testId !== "string" ||
      !PLUGIN_TEST_ID_SET.has(value.testId) ||
      typeof value.passed !== "boolean" ||
      typeof value.detail !== "string" ||
      value.detail.length === 0 ||
      value.detail.length > 256
    ) {
      return false;
    }

    if (!isActiveSuiteRun(value.runId)) return true;
    const admission = suiteRunGate.accept(value.runId, value.testId);
    if (admission === "ignored") return true;

    results = {
      ...results,
      [value.testId]: { passed: value.passed, detail: value.detail },
    };
    if (admission === "complete") void finishSuiteAudit(value.runId);
    return true;
  }

  function acceptWatchdogPong(value: unknown): boolean {
    if (!isPlainRecord(value) || value.type !== "lorepia:watchdog:pong") return false;
    const activeWatchdog = watchdog;
    if (activeWatchdog === null) return true;
    activeWatchdog.receivePong(value);
    updateWatchdogSnapshot();
    return true;
  }

  function validateNativeResult(method: unknown, result: NativeBrokerResult): JsonObject {
    if (
      method === "state.read" &&
      hasExactKeys(result, ["type", "state"]) &&
      result.type === "state_read" &&
      result.state === "ready"
    ) {
      return result;
    }
    if (
      method === "render.sanitize" &&
      hasExactKeys(result, ["type", "html", "inputBytes", "outputBytes"]) &&
      result.type === "render_sanitize" &&
      typeof result.html === "string" &&
      typeof result.inputBytes === "number" &&
      Number.isSafeInteger(result.inputBytes) &&
      result.inputBytes >= 0 &&
      typeof result.outputBytes === "number" &&
      Number.isSafeInteger(result.outputBytes) &&
      result.outputBytes >= 0
    ) {
      return result;
    }
    if (
      method === "probe.increment" &&
      hasExactKeys(result, ["type", "sentinel", "callCount"]) &&
      result.type === "probe_increment" &&
      result.sentinel === "LOREPIA_HOST_BROKER_PROBE_REACHED" &&
      typeof result.callCount === "number" &&
      Number.isSafeInteger(result.callCount) &&
      result.callCount >= 1
    ) {
      return result;
    }
    throw new Error("native broker result does not match the requested method");
  }

  async function forwardPluginBrokerRequest(
    value: unknown,
    target: Window,
  ): Promise<boolean> {
    const token = hostToken;
    const activeNonce = sessionNonce;
    const activeGeneration = hostGeneration;
    if (
      token === null ||
      activeGeneration === null ||
      !hostBrokerReady ||
      brokerRotating
    ) {
      return true;
    }

    const response = await hostBrokerForwarder.forward(
      value,
      activeNonce,
      async ({ requestId, method, requestJson }): Promise<NativeBrokerForwardOutcome> => {
        try {
          const native = await invokeHostBrokerJson(token, requestId, requestJson);
          return {
            ok: true,
            result: validateNativeResult(method, native.result),
          };
        } catch (error) {
          return error instanceof NativeBrokerDenied
            ? {
                ok: false,
                error: { code: error.code, message: error.message },
              }
            : {
                ok: false,
                error: {
                  code: "HANDLER_FAILED",
                  message: "The native host broker failed without exposing details.",
                },
              };
        }
      },
    );
    if (response === null) return false;

    if (
      frame?.contentWindow === target &&
      sessionNonce === activeNonce &&
      hostToken === token &&
      hostGeneration === activeGeneration &&
      hostBrokerReady &&
      !brokerRotating &&
      !frameDisabled
    ) {
      target.postMessage(response, "*");
    }
    return true;
  }

  async function handleFrameMessage(event: MessageEvent<unknown>): Promise<void> {
    const target = frame?.contentWindow;
    if (target === null || target === undefined) return;
    if (event.source !== target) return;
    if (event.origin !== "null") {
      statusMessage = `격리 프레임 메시지가 예상 밖 origin(${event.origin})에서 도착해 거부했습니다.`;
      return;
    }
    if (!frameMessageAdmission.consume()) return;

    // Route by one own data property before any exact-key enumeration. An
    // unknown attacker message is therefore rejected in constant work instead
    // of being walked by every protocol decoder in sequence.
    const messageType = safeIsolationMessageTypeHint(event.data);
    if (messageType === "lorepia:plugin:bootstrap-error") {
      if (
        isPlainRecord(event.data) &&
        hasExactKeys(event.data, ["type", "detail"]) &&
        typeof event.data.detail === "string" &&
        event.data.detail.length > 0 &&
        event.data.detail.length <= 256
      ) {
        statusMessage = `플러그인 bootstrap 실패: ${event.data.detail}`;
      }
      return;
    }

    if (messageType === "lorepia:plugin:ready") {
      if (isReadyMessage(event.data) && !frameReady) {
        try {
          startFrameRuntime();
          frameReady = true;
          statusMessage =
            staleTokenAudit?.passed === true
              ? "격리 프레임 준비 완료. negative test를 실행할 수 있습니다."
              : "격리 프레임 준비 완료. stale-token 감사를 위해 새 세션 회전이 필요합니다.";
        } catch {
          disableFrame();
          statusMessage = "host broker 연결 실패로 플러그인 iframe을 비활성화했습니다.";
        }
      }
      return;
    }
    if (messageType === "lorepia:watchdog:pong") {
      acceptWatchdogPong(event.data);
      return;
    }
    if (messageType === "lorepia:plugin:test-result") {
      acceptTestResult(event.data);
      return;
    }
    if (messageType === "request") {
      await forwardPluginBrokerRequest(event.data, target);
    }
  }

  async function finishSuiteAudit(runId: string): Promise<void> {
    try {
      const after = await readNativeSecurityAudit();
      if (!isActiveSuiteRun(runId)) return;
      probeCountAfter = after.probeCallCount;
      sanitizeCountAfter = after.sanitizeCallCount;
      const expectedProbe = probeCountBefore === null ? null : probeCountBefore + 1;
      const expectedSanitize =
        sanitizeCountBefore === null ? null : sanitizeCountBefore + 1;
      const passed =
        expectedProbe !== null &&
        expectedSanitize !== null &&
        probeCountAfter === expectedProbe &&
        sanitizeCountAfter === expectedSanitize;
      results = {
        ...results,
        [AUDIT_TEST_ID]: {
          passed,
          detail:
            expectedProbe === null || expectedSanitize === null
              ? "suite 시작 전 native sink counter가 없습니다."
              : `broker sink delta: probe ${probeCountBefore}->${probeCountAfter}, sanitize ${sanitizeCountBefore}->${sanitizeCountAfter}`,
        },
      };
      statusMessage = "negative test 실행과 native side-effect 감사를 완료했습니다.";
    } catch (error) {
      if (!isActiveSuiteRun(runId)) return;
      results = {
        ...results,
        [AUDIT_TEST_ID]: {
          passed: false,
          detail: "native side-effect counter를 검증하지 못했습니다.",
        },
      };
      statusMessage = `native side-effect 감사 실패: ${String(error)}`;
    } finally {
      finishSuiteRun(runId);
    }
  }

  async function runSuite(): Promise<void> {
    const target = frame?.contentWindow;
    const token = hostToken;
    const activeNonce = sessionNonce;
    if (
      !hostBrokerReady ||
      token === null ||
      hostGeneration === null ||
      staleTokenAudit?.passed !== true ||
      brokerRotating ||
      !frameReady ||
      frameDisabled ||
      suiteRunning ||
      target === null ||
      target === undefined
    ) {
      return;
    }

    const runId = `suite-${randomHex(16)}`;
    suiteRunGate.start(runId);
    suiteRunning = true;
    results = {
      [STALE_TOKEN_AUDIT_TEST_ID]: staleTokenAudit,
    };
    probeCountBefore = null;
    probeCountAfter = null;
    sanitizeCountBefore = null;
    sanitizeCountAfter = null;
    statusMessage = "negative test 실행 중입니다.";

    try {
      const rawSinkBefore = await readNativeSecurityAudit();
      const runtimeCommandAudit = await auditRetiredRawCommands(
        (command, args) => invoke(command, args),
      );
      const rawSinkAfter = await readNativeSecurityAudit();
      if (!isActiveSuiteRun(runId)) return;
      const rawSinkUnchanged =
        rawSinkAfter.probeCallCount === rawSinkBefore.probeCallCount &&
        rawSinkAfter.sanitizeCallCount === rawSinkBefore.sanitizeCallCount;
      results = {
        ...results,
        [NATIVE_COMMAND_AUDIT_TEST_ID]: {
          passed: runtimeCommandAudit.passed && rawSinkUnchanged,
          detail: rawSinkUnchanged
            ? `attested 10-command surface v3; ${runtimeCommandAudit.detail}`
            : `retired command audit changed a privileged sink: probe ${rawSinkBefore.probeCallCount}->${rawSinkAfter.probeCallCount}, sanitize ${rawSinkBefore.sanitizeCallCount}->${rawSinkAfter.sanitizeCallCount}`,
        },
      };

      const control = await invokeHostBroker(
        token,
        `host-${randomHex(16)}`,
        "probe.increment",
        {},
      );
      if (!isActiveSuiteRun(runId)) return;
      const controlResult = control.result;
      hostProbeControl = {
        passed:
          hasExactKeys(controlResult, ["type", "sentinel", "callCount"]) &&
          controlResult.type === "probe_increment" &&
          controlResult.sentinel === "LOREPIA_HOST_BROKER_PROBE_REACHED" &&
          typeof controlResult.callCount === "number" &&
          Number.isSafeInteger(controlResult.callCount),
        detail:
          controlResult.type === "probe_increment"
            ? `host-authenticated positive control count=${controlResult.callCount}`
            : "host broker returned an unexpected control result",
      };
      if (!hostProbeControl.passed) {
        throw new Error("host broker positive control response is invalid");
      }
      const suiteBaseline = await readNativeSecurityAudit();
      if (
        !isActiveSuiteRun(runId) ||
        frame?.contentWindow !== target ||
        sessionNonce !== activeNonce
      ) {
        return;
      }
      probeCountBefore = suiteBaseline.probeCallCount;
      sanitizeCountBefore = suiteBaseline.sanitizeCallCount;
      target.postMessage(
        {
          type: "lorepia:plugin:run-suite",
          sessionNonce: activeNonce,
          runId,
        },
        "*",
      );
    } catch (error) {
      if (!isActiveSuiteRun(runId)) return;
      hostProbeControl = { passed: false, detail: String(error) };
      statusMessage = "top-frame positive control이 실패해 실험 장치가 유효하지 않습니다.";
      finishSuiteRun(runId);
    }
  }

  function makeWatchdogSilent(): void {
    const target = frame?.contentWindow;
    if (!frameReady || target === null || target === undefined) return;
    target.postMessage(
      {
        type: "lorepia:plugin:set-watchdog-mode",
        sessionNonce,
        mode: "silent",
      },
      "*",
    );
    statusMessage = "플러그인 pong을 중단했습니다. 워치독 비활성화를 기다리는 중입니다.";
  }

  async function reloadFrame(): Promise<void> {
    const currentToken = hostToken;
    const expectedGeneration = hostGeneration;
    if (
      !hostBrokerReady ||
      currentToken === null ||
      expectedGeneration === null ||
      brokerRotating ||
      suiteRunning
    ) {
      statusMessage = "host broker가 등록되지 않아 iframe을 만들지 않습니다.";
      return;
    }

    // Revoke the old frame's host-side authority before starting native token
    // rotation. No failure path below is allowed to create a replacement frame.
    brokerRotating = true;
    hostBrokerReady = false;
    invalidateSuiteRun();
    clearWatchdogTimers();
    watchdog = null;
    watchdogSnapshot = null;
    frameReady = false;
    frameDisabled = true;
    results = {};
    hostProbeControl = null;
    staleTokenAudit = null;
    probeCountBefore = null;
    probeCountAfter = null;
    sanitizeCountBefore = null;
    sanitizeCountAfter = null;
    sessionNonce = "";
    if (frame !== null) frame.src = "about:blank";
    statusMessage = "기존 iframe을 차단하고 host token을 회전하는 중입니다.";

    const nextToken = randomHex(32);
    try {
      if (hostBrokerForwarder.inFlightCount !== 0) {
        throw new Error("기존 generation의 native broker 요청이 아직 진행 중입니다.");
      }
      beginHostBrokerRotation(window.sessionStorage, {
        currentToken,
        nextToken,
        expectedGeneration,
      });
      const nextGeneration = await rotateHostBroker(
        currentToken,
        nextToken,
        expectedGeneration,
      );
      if (componentDisposed) return;
      await finalizeHostBrokerRotationAfterAudit(
        window.sessionStorage,
        nextToken,
        nextGeneration,
        auditStaleTokenBeforeFinalize,
      );
      if (componentDisposed) return;

      hostToken = nextToken;
      hostGeneration = nextGeneration;
      frameDisabled = false;
      hostBrokerReady = true;
      sessionNonce = randomHex(32);
      statusMessage = `broker generation ${nextGeneration} 회전 및 stale-token 감사를 완료했습니다.`;
    } catch (error) {
      if (componentDisposed) return;
      hostToken = null;
      hostGeneration = null;
      hostBrokerReady = false;
      frameDisabled = true;
      sessionNonce = "";
      if (staleTokenAudit === null) {
        recordStaleTokenAudit({ passed: false, detail: String(error) });
      }
      statusMessage = "host token 회전이 실패해 새 iframe을 만들지 않습니다.";
    } finally {
      brokerRotating = false;
    }
  }

  onMount(() => {
    componentDisposed = false;
    if (!IMPORTED_CODE_EXECUTION_ENABLED) {
      hostBrokerReady = false;
      frameReady = false;
      frameDisabled = true;
      statusMessage = `Store-Safe ${__LOREPIA_BUILD_PROFILE__.targetPlatform} build: imported JavaScript와 Lua 실행은 OFF입니다.`;
      return () => {
        componentDisposed = true;
      };
    }

    const listener = (event: MessageEvent<unknown>) => {
      void handleFrameMessage(event);
    };
    window.addEventListener("message", listener);
    removeMessageListener = () => window.removeEventListener("message", listener);

    void (async () => {
      try {
        const recovered = await recoverHostBrokerRotation(
          window.sessionStorage,
          registerHostBroker,
          auditStaleTokenBeforeFinalize,
        );
        const token = recovered?.token ?? getOrCreateHostToken();
        const generation =
          recovered?.generation ?? (await registerHostBroker(token));
        if (componentDisposed) return;
        hostToken = token;
        hostGeneration = generation;
        hostBrokerReady = true;
        sessionNonce = randomHex(32);
        statusMessage = recovered
          ? recovered.outcome === "recovered_next"
            ? `rotation journal의 stale-token 감사를 재통과했습니다. broker generation ${generation}.`
            : `미완료 rotation을 current token으로 롤백했습니다. broker generation ${generation}.`
          : `host broker generation ${generation} 등록 완료. 격리 프레임을 불러오는 중입니다.`;
      } catch {
        if (componentDisposed) return;
        hostToken = null;
        hostGeneration = null;
        hostBrokerReady = false;
        sessionNonce = "";
        statusMessage = "host broker 등록 실패. 플러그인 iframe을 만들지 않습니다.";
      }
    })();

    return () => {
      componentDisposed = true;
      invalidateSuiteRun();
      clearWatchdogTimers();
      removeMessageListener?.();
      removeMessageListener = null;
    };
  });
</script>

<svelte:head>
  <title>LorePia M-1 plugin isolation probe</title>
  <meta
    name="description"
    content="Sandboxed iframe, typed broker, sanitizer, Tauri IPC and watchdog negative-test harness"
  />
</svelte:head>

<main>
  <nav><a href="/">Channel 실증으로 돌아가기</a></nav>
  <header>
    <h1>M-1 플러그인 격리 실증</h1>
    <p>동일 WebView iframe의 DOM·네트워크·Tauri IPC·broker·워치독 경계를 실제로 공격합니다.</p>
  </header>

  <section aria-labelledby="controls-heading">
    <h2 id="controls-heading">제어</h2>
    <div class="controls">
      <button
        type="button"
        onclick={runSuite}
        disabled={!hostBrokerReady || staleTokenAudit?.passed !== true || brokerRotating || !frameReady || frameDisabled || suiteRunning}
      >
        negative test 실행
      </button>
      <button type="button" onclick={makeWatchdogSilent} disabled={!frameReady || frameDisabled}>
        워치독 무응답 시험
      </button>
      <button
        type="button"
        onclick={reloadFrame}
        disabled={!hostBrokerReady || brokerRotating || suiteRunning}
      >
        플러그인 새 세션
      </button>
    </div>
    <p role="status" aria-live="polite">{statusMessage}</p>
  </section>

  <section aria-labelledby="frame-heading">
    <h2 id="frame-heading">sandboxed plugin frame</h2>
    {#if !IMPORTED_CODE_EXECUTION_ENABLED}
      <p data-testid="store-safe-imported-code-status">
        Store-Safe mobile profile: imported JavaScript OFF, imported Lua OFF. 이 빌드에는 실행
        fixture가 포함되지 않으며 이 경로는 iframe을 만들지 않습니다.
      </p>
    {:else if hostBrokerReady && sessionNonce.length > 0}
      <iframe
        bind:this={frame}
        title="악성 플러그인 격리 시험 프레임"
        src={`/plugin-frame.html?session=${sessionNonce}`}
        sandbox="allow-scripts"
        allow=""
        referrerpolicy="no-referrer"
      ></iframe>
    {/if}
  </section>

  <section aria-labelledby="watchdog-heading">
    <h2 id="watchdog-heading">워치독</h2>
    <dl>
      <dt>상태</dt>
      <dd data-testid="watchdog-status">{watchdogSnapshot?.state.status ?? "준비 전"}</dd>
      <dt>마지막 ping seq</dt>
      <dd>{watchdogSnapshot?.lastIssuedSeq ?? "—"}</dd>
      <dt>마지막 pong seq</dt>
      <dd>{watchdogSnapshot?.lastAcceptedSeq ?? "—"}</dd>
      <dt>연속 deadline miss</dt>
      <dd>{watchdogSnapshot?.consecutiveMissedDeadlines ?? "—"}</dd>
      <dt>프레임 비활성화</dt>
      <dd>{frameDisabled ? "예" : "아니오"}</dd>
    </dl>
  </section>

  <section aria-labelledby="results-heading">
    <h2 id="results-heading">
      공격 결과 ({passedCount}/{TEST_IDS.length} 통과, {completedCount}/{TEST_IDS.length} 완료)
    </h2>
    <dl>
      <dt>host broker generation</dt>
      <dd>{hostGeneration ?? "—"}</dd>
      <dt>stale-token 회전 감사</dt>
      <dd class:pass={staleTokenAudit?.passed} class:fail={staleTokenAudit?.passed === false}>
        {staleTokenAudit === null ? "미실행" : `${staleTokenAudit.passed ? "PASS" : "FAIL"}: ${staleTokenAudit.detail}`}
      </dd>
      <dt>host-authenticated probe 대조군</dt>
      <dd class:pass={hostProbeControl?.passed} class:fail={hostProbeControl?.passed === false}>
        {hostProbeControl === null ? "미실행" : `${hostProbeControl.passed ? "PASS" : "FAIL"}: ${hostProbeControl.detail}`}
      </dd>
      <dt>probe side-effect count</dt>
      <dd>{probeCountBefore ?? "—"} → {probeCountAfter ?? "—"}</dd>
      <dt>sanitize sink count</dt>
      <dd>{sanitizeCountBefore ?? "—"} → {sanitizeCountAfter ?? "—"}</dd>
      {#each TEST_IDS as testId}
        {@const result = results[testId]}
        <dt>{testId}</dt>
        <dd
          data-testid={`result-${testId}`}
          class:pass={result?.passed}
          class:fail={result?.passed === false}
        >
          {result === undefined ? "미실행" : `${result.passed ? "PASS" : "FAIL"}: ${result.detail}`}
        </dd>
      {/each}
    </dl>
  </section>

  <section aria-labelledby="contract-heading">
    <h2 id="contract-heading">현재 실험 계약</h2>
    <p>
      protocol v{ISOLATION_PROTOCOL_VERSION}, iframe sandbox는 allow-scripts만 허용하고,
      postMessage는 exact source + opaque origin + 256-bit session nonce로 묶습니다. 이 실험의
      sanitizer/probe sink는 iframe 생성 전에 등록한 별도의 256-bit host token이 있어야 Rust
      broker가 승인합니다.
    </p>
    <p>
      같은 창에 Channel 실증용 raw command 4개가 남아 있고 모바일 실기기 결과도 없으므로, 이
      화면만으로 production-safe 또는 5 OS PASS를 주장하지 않습니다.
    </p>
  </section>
</main>

<style>
  :global(html) {
    font-family: system-ui, sans-serif;
    line-height: 1.5;
  }

  :global(body) {
    margin: 0;
  }

  main {
    box-sizing: border-box;
    max-width: 64rem;
    margin: 0 auto;
    padding: 1rem;
  }

  section {
    margin-block: 1.5rem;
  }

  .controls {
    display: flex;
    flex-wrap: wrap;
    gap: 0.5rem;
  }

  button {
    padding: 0.45rem 0.75rem;
    font: inherit;
  }

  iframe {
    box-sizing: border-box;
    width: 100%;
    min-height: 8rem;
    border: 1px solid currentcolor;
  }

  dl {
    display: grid;
    grid-template-columns: minmax(12rem, max-content) minmax(0, 1fr);
    gap: 0.3rem 1rem;
  }

  dt {
    font-weight: 600;
  }

  dd {
    margin: 0;
    overflow-wrap: anywhere;
  }

  .pass {
    color: #087830;
  }

  .fail {
    color: #b00020;
    font-weight: 600;
  }

  @media (prefers-color-scheme: dark) {
    .pass {
      color: #7ee2a8;
    }

    .fail {
      color: #ffb4ab;
    }
  }
</style>
