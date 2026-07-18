<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { onMount } from "svelte";

  import {
    createIsolationBroker,
    defineIsolationHandler,
    type IsolationBroker,
    type RegisteredIsolationHandler,
  } from "$lib/isolation-broker";
  import {
    ISOLATION_PROTOCOL_VERSION,
    type JsonObject,
  } from "$lib/isolation-protocol";
  import {
    PluginWatchdog,
    type PluginWatchdogSnapshot,
  } from "$lib/plugin-watchdog";

  type TestResult = {
    passed: boolean;
    detail: string;
  };

  type SanitizedHtmlResponse = {
    html: string;
    inputBytes: number;
    outputBytes: number;
  };

  type PrivilegedProbeResponse = {
    sentinel: string;
    callCount: number;
  };

  const TEST_IDS = [
    "parent-document-blocked",
    "local-storage-blocked",
    "window-open-blocked",
    "external-fetch-csp-blocked",
    "direct-tauri-invoke-blocked",
    "broker-state-read",
    "broker-render-sanitize",
    "broker-network-denied",
    "broker-secret-permission-denied",
    "broker-replay-rejected",
    "broker-unknown-field-rejected",
  ] as const;

  const TEST_ID_SET = new Set<string>(TEST_IDS);
  const WATCHDOG_PING_INTERVAL_MS = 250;
  const WATCHDOG_TIMEOUT_MS = 300;
  const WATCHDOG_MISSED_DEADLINE_THRESHOLD = 2;

  let frame = $state<HTMLIFrameElement | null>(null);
  let sessionNonce = $state("");
  let frameReady = $state(false);
  let frameDisabled = $state(false);
  let suiteRunning = $state(false);
  let results = $state<Record<string, TestResult>>({});
  let statusMessage = $state("격리 프레임을 준비하는 중입니다.");
  let broker: IsolationBroker | null = null;
  let watchdog: PluginWatchdog | null = null;
  let watchdogSnapshot = $state<PluginWatchdogSnapshot | null>(null);
  let hostProbeControl = $state<TestResult | null>(null);
  let probeCountBefore = $state<number | null>(null);
  let probeCountAfter = $state<number | null>(null);
  let pingTimer: number | null = null;
  let deadlineTimer: number | null = null;
  let removeMessageListener: (() => void) | null = null;

  let completedCount = $derived(Object.keys(results).length);
  let passedCount = $derived(
    Object.values(results).filter((result) => result.passed).length,
  );

  function randomHex(bytes: number): string {
    const values = new Uint8Array(bytes);
    crypto.getRandomValues(values);
    return Array.from(values, (value) => value.toString(16).padStart(2, "0")).join("");
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

  function clearWatchdogTimers(): void {
    if (pingTimer !== null) window.clearTimeout(pingTimer);
    if (deadlineTimer !== null) window.clearTimeout(deadlineTimer);
    pingTimer = null;
    deadlineTimer = null;
  }

  function disableFrame(): void {
    clearWatchdogTimers();
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

  function createHandlers(): Record<string, RegisteredIsolationHandler> {
    return {
      "state.read": defineIsolationHandler({
        permission: "state.read",
        payloadKeys: [],
        parsePayload: () => ({ ok: true, value: null }),
        handle: () => ({ moduleId: "fixture.plugin", state: "ready" }),
      }),
      "render.sanitize": defineIsolationHandler({
        permission: "render.html",
        payloadKeys: ["html"],
        parsePayload(payload: JsonObject) {
          return typeof payload.html === "string"
            ? { ok: true as const, value: payload.html }
            : { ok: false as const, message: "html must be a string" };
        },
        async handle(html) {
          const sanitized = await invoke<SanitizedHtmlResponse>(
            "sanitize_plugin_html",
            { html },
          );
          if (
            !isPlainRecord(sanitized) ||
            typeof sanitized.html !== "string" ||
            !Number.isSafeInteger(sanitized.inputBytes) ||
            !Number.isSafeInteger(sanitized.outputBytes)
          ) {
            throw new Error("native sanitizer returned an invalid response");
          }
          return {
            html: sanitized.html,
            inputBytes: sanitized.inputBytes,
            outputBytes: sanitized.outputBytes,
          };
        },
      }),
      "network.fetch": defineIsolationHandler({
        permission: "network.fetch",
        network: true,
        payloadKeys: ["url"],
        parsePayload(payload: JsonObject) {
          return typeof payload.url === "string"
            ? { ok: true as const, value: payload.url }
            : { ok: false as const, message: "url must be a string" };
        },
        handle: (url) => ({ url, reached: true }),
      }),
      "secret.read": defineIsolationHandler({
        permission: "secret.read",
        payloadKeys: [],
        parsePayload: () => ({ ok: true, value: null }),
        handle: () => ({ secret: "must-never-be-returned" }),
      }),
    };
  }

  function startFrameRuntime(source: Window): void {
    broker = createIsolationBroker({
      expectedSource: source,
      sessionNonce,
      manifestPermissions: [
        "state.read",
        "render.html",
        "network.fetch",
        "secret.read",
      ],
      approvedPermissions: ["state.read", "render.html", "network.fetch"],
      handlers: createHandlers(),
      networkPolicy: "deny",
    });

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
      !hasExactKeys(value, ["type", "sessionNonce", "testId", "passed", "detail"]) ||
      value.type !== "lorepia:plugin:test-result" ||
      value.sessionNonce !== sessionNonce ||
      typeof value.testId !== "string" ||
      !TEST_ID_SET.has(value.testId) ||
      typeof value.passed !== "boolean" ||
      typeof value.detail !== "string" ||
      value.detail.length === 0 ||
      value.detail.length > 256
    ) {
      return false;
    }

    results = {
      ...results,
      [value.testId]: { passed: value.passed, detail: value.detail },
    };
    if (Object.keys(results).length === TEST_IDS.length) {
      void finishSuiteAudit();
    }
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

  async function handleFrameMessage(event: MessageEvent<unknown>): Promise<void> {
    const target = frame?.contentWindow;
    if (target === null || target === undefined) return;
    if (event.source !== target) return;
    if (event.origin !== "null") {
      statusMessage = `격리 프레임 메시지가 예상 밖 origin(${event.origin})에서 도착해 거부했습니다.`;
      return;
    }

    if (
      isPlainRecord(event.data) &&
      hasExactKeys(event.data, ["type", "detail"]) &&
      event.data.type === "lorepia:plugin:bootstrap-error" &&
      typeof event.data.detail === "string"
    ) {
      statusMessage = `플러그인 bootstrap 실패: ${event.data.detail}`;
      return;
    }

    if (isReadyMessage(event.data)) {
      if (!frameReady) {
        frameReady = true;
        statusMessage = "격리 프레임 준비 완료. negative test를 실행할 수 있습니다.";
        startFrameRuntime(target);
      }
      return;
    }
    if (acceptWatchdogPong(event.data) || acceptTestResult(event.data)) return;

    const activeBroker = broker;
    if (activeBroker === null) return;
    const response = await activeBroker.handleEvent({
      source: event.source,
      origin: event.origin,
      data: event.data,
    });
    if (response !== null && frame?.contentWindow === target && !frameDisabled) {
      target.postMessage(response, "*");
    }
  }

  async function finishSuiteAudit(): Promise<void> {
    try {
      probeCountAfter = await invoke<number>("privileged_probe_count");
      if (
        probeCountBefore !== null &&
        probeCountAfter !== probeCountBefore
      ) {
        results = {
          ...results,
          "direct-tauri-invoke-blocked": {
            passed: false,
            detail: `응답 여부와 무관하게 native side effect가 ${probeCountBefore}→${probeCountAfter}로 증가했습니다.`,
          },
        };
      }
      statusMessage = "negative test 실행과 native side-effect 감사를 완료했습니다.";
    } catch (error) {
      statusMessage = `native side-effect 감사 실패: ${String(error)}`;
    } finally {
      suiteRunning = false;
    }
  }

  async function runSuite(): Promise<void> {
    const target = frame?.contentWindow;
    if (!frameReady || frameDisabled || suiteRunning || target === null || target === undefined) {
      return;
    }

    suiteRunning = true;
    results = {};
    probeCountBefore = null;
    probeCountAfter = null;
    statusMessage = "negative test 실행 중입니다.";

    try {
      const control = await invoke<PrivilegedProbeResponse>("privileged_probe");
      hostProbeControl = {
        passed:
          control.sentinel === "LOREPIA_PRIVILEGED_COMMAND_REACHED" &&
          Number.isSafeInteger(control.callCount),
        detail: `top-frame positive control count=${control.callCount}`,
      };
      probeCountBefore = await invoke<number>("privileged_probe_count");
      target.postMessage(
        {
          type: "lorepia:plugin:run-suite",
          sessionNonce,
        },
        "*",
      );
    } catch (error) {
      hostProbeControl = { passed: false, detail: String(error) };
      statusMessage = "top-frame positive control이 실패해 실험 장치가 유효하지 않습니다.";
      suiteRunning = false;
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

  function reloadFrame(): void {
    clearWatchdogTimers();
    broker = null;
    watchdog = null;
    watchdogSnapshot = null;
    frameReady = false;
    frameDisabled = false;
    suiteRunning = false;
    results = {};
    hostProbeControl = null;
    probeCountBefore = null;
    probeCountAfter = null;
    sessionNonce = randomHex(32);
    statusMessage = "새 session nonce로 격리 프레임을 다시 불러오는 중입니다.";
  }

  onMount(() => {
    sessionNonce = randomHex(32);
    const listener = (event: MessageEvent<unknown>) => {
      void handleFrameMessage(event);
    };
    window.addEventListener("message", listener);
    removeMessageListener = () => window.removeEventListener("message", listener);

    return () => {
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
      <button type="button" onclick={runSuite} disabled={!frameReady || frameDisabled || suiteRunning}>
        negative test 실행
      </button>
      <button type="button" onclick={makeWatchdogSilent} disabled={!frameReady || frameDisabled}>
        워치독 무응답 시험
      </button>
      <button type="button" onclick={reloadFrame}>플러그인 새 세션</button>
    </div>
    <p role="status" aria-live="polite">{statusMessage}</p>
  </section>

  <section aria-labelledby="frame-heading">
    <h2 id="frame-heading">sandboxed plugin frame</h2>
    {#if sessionNonce.length > 0}
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
    <h2 id="results-heading">공격 결과 ({passedCount}/{completedCount} 통과)</h2>
    <dl>
      <dt>top-frame probe 대조군</dt>
      <dd class:pass={hostProbeControl?.passed} class:fail={hostProbeControl?.passed === false}>
        {hostProbeControl === null ? "미실행" : `${hostProbeControl.passed ? "PASS" : "FAIL"}: ${hostProbeControl.detail}`}
      </dd>
      <dt>probe side-effect count</dt>
      <dd>{probeCountBefore ?? "—"} → {probeCountAfter ?? "—"}</dd>
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
      postMessage는 exact source + opaque origin + 256-bit session nonce로 묶습니다.
    </p>
    <p>
      모바일 실기기 결과가 없으므로 이 화면의 로컬 결과만으로 5 OS PASS를 주장하지 않습니다.
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
