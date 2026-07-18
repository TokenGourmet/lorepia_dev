<script lang="ts">
  import {
    createStreamContractState,
    validateStreamEvent,
    validateTerminalSnapshot,
    type ExpectedTerminalSnapshot,
    type StreamContractState,
  } from "$lib/stream-contract";
  import {
    acknowledgeStream,
    cancelStream,
    createStreamChannel,
    describeCommandError,
    getStreamSnapshot,
    startMockStream,
    type StreamEvent,
    type StreamSnapshot,
    type StreamSnapshotStatus,
  } from "$lib/stream-protocol";

  type UiPhase =
    | "idle"
    | "starting"
    | "streaming"
    | "cancelling"
    | "completed"
    | "cancelled"
    | "failed";

  type RunContext = {
    channel: ReturnType<typeof createStreamChannel>;
    ackChain: Promise<void>;
    ackDelayMs: number;
    previousDeltaArrivalMs: number | null;
    contractState: StreamContractState;
    expectedTerminal: ExpectedTerminalSnapshot | null;
  };

  const phaseLabel: Record<UiPhase, string> = {
    idle: "대기",
    starting: "시작 중",
    streaming: "수신 중",
    cancelling: "중단 요청 중",
    completed: "완료",
    cancelled: "중단됨",
    failed: "실패",
  };

  const backendStatusLabel: Record<StreamSnapshotStatus, string> = {
    queued: "대기열",
    streaming: "스트리밍",
    completed: "완료",
    cancelled: "중단됨",
    failed: "실패",
  };

  let phase = $state<UiPhase>("idle");
  let backendStatus = $state<StreamSnapshotStatus | null>(null);
  let requestId = $state<string | null>(null);
  let lastSeq = $state<number | null>(null);
  let lastAckedSeq = $state<number | null>(null);
  let updateCount = $state(0);
  let pendingAckCount = $state(0);
  let backendInFlight = $state(0);
  let partialText = $state("");
  let errorMessage = $state<string | null>(null);
  let snapshot = $state<StreamSnapshot | null>(null);
  let batchWindowMs = $state<number | null>(null);
  let maxInFlight = $state<number | null>(null);
  let ackDelayMs = $state<number | undefined>(0);
  let appliedAckDelayMs = $state<number | null>(null);
  let minDeltaIntervalMs = $state<number | null>(null);
  let maxDeltaIntervalMs = $state<number | null>(null);
  let peakPendingAckCount = $state(0);
  let peakBackendInFlight = $state(0);
  let terminalSeen = $state(false);
  let effectiveBatchWindowMs = $state<number | null>(null);
  let batchWindowIncreaseMs = $state<number | null>(null);
  let backpressureObserved = $state<boolean | null>(null);
  let finalSnapshotPending = $state(false);

  let activeRun: RunContext | null = null;

  let isActive = $derived(
    phase === "starting" || phase === "streaming" || phase === "cancelling",
  );
  let isBusy = $derived(isActive || finalSnapshotPending);
  let canCancel = $derived(
    requestId !== null && (phase === "starting" || phase === "streaming"),
  );
  let ackDelayIsValid = $derived(
    typeof ackDelayMs === "number" &&
      Number.isSafeInteger(ackDelayMs) &&
      ackDelayMs >= 0 &&
      ackDelayMs <= 10_000,
  );

  function resetDisplay(): void {
    phase = "starting";
    backendStatus = "queued";
    requestId = null;
    lastSeq = null;
    lastAckedSeq = null;
    updateCount = 0;
    pendingAckCount = 0;
    backendInFlight = 0;
    partialText = "";
    errorMessage = null;
    snapshot = null;
    batchWindowMs = null;
    maxInFlight = null;
    appliedAckDelayMs = null;
    minDeltaIntervalMs = null;
    maxDeltaIntervalMs = null;
    peakPendingAckCount = 0;
    peakBackendInFlight = 0;
    terminalSeen = false;
    effectiveBatchWindowMs = null;
    batchWindowIncreaseMs = null;
    backpressureObserved = null;
    finalSnapshotPending = false;
  }

  function reportProtocolError(message: string): void {
    errorMessage = `프로토콜 오류: ${message}`;
  }

  function waitForAckDelay(delayMs: number): Promise<void> {
    if (delayMs === 0) return Promise.resolve();
    return new Promise((resolve) => window.setTimeout(resolve, delayMs));
  }

  function formatMilliseconds(value: number | null): string {
    return value === null ? "—" : `${value.toFixed(1)} ms`;
  }

  function queueAcknowledgement(run: RunContext, event: StreamEvent): void {
    pendingAckCount += 1;
    peakPendingAckCount = Math.max(peakPendingAckCount, pendingAckCount);
    run.ackChain = run.ackChain.then(async () => {
      try {
        await waitForAckDelay(run.ackDelayMs);
        const acknowledgement = await acknowledgeStream(event.requestId, event.seq);

        if (activeRun !== run) return;

        if (acknowledgement.requestId !== event.requestId) {
          reportProtocolError(
            `ACK 응답의 요청 ID가 다릅니다: ${acknowledgement.requestId}`,
          );
          return;
        }

        lastAckedSeq = acknowledgement.acknowledgedThrough;
        backendInFlight = acknowledgement.inFlight;
        peakBackendInFlight = Math.max(peakBackendInFlight, acknowledgement.inFlight);
      } catch (error) {
        if (activeRun === run) {
          errorMessage = `ACK 실패(seq ${event.seq}): ${describeCommandError(error)}`;
        }
      } finally {
        if (activeRun === run) {
          pendingAckCount = Math.max(0, pendingAckCount - 1);
        }
      }
    });
  }

  async function queryFinalSnapshot(
    run: RunContext,
    expected: ExpectedTerminalSnapshot,
  ): Promise<void> {
    try {
      await run.ackChain;
      if (activeRun !== run) return;

      const rawSnapshot: unknown = await getStreamSnapshot(expected.requestId);
      if (activeRun !== run) return;

      if (run.expectedTerminal !== expected) {
        reportProtocolError("기대하던 종료 이벤트가 스냅샷 조회 중 변경되었습니다.");
        return;
      }

      const validation = validateTerminalSnapshot(rawSnapshot, expected);
      if (!validation.accepted) {
        reportProtocolError(validation.error);
        return;
      }

      const result = validation.snapshot;
      snapshot = result;
      backendStatus = result.status;
      lastSeq = result.lastSeq;
      lastAckedSeq = result.lastAckedSeq;
      backendInFlight = result.inFlight;
      partialText = result.text;
      batchWindowMs = result.batchWindowMs;
      maxInFlight = result.maxInFlight;
      effectiveBatchWindowMs = result.effectiveBatchWindowMs;
      batchWindowIncreaseMs = result.effectiveBatchWindowMs - result.batchWindowMs;
      backpressureObserved = result.effectiveBatchWindowMs > result.batchWindowMs;

      if (result.error !== null) {
        errorMessage = `${result.error.code}: ${result.error.message}`;
      }
    } catch (error) {
      if (activeRun === run) {
        errorMessage = `최종 스냅샷 조회 실패: ${describeCommandError(error)}`;
      }
    } finally {
      if (activeRun === run) {
        finalSnapshotPending = false;
      }
    }
  }

  function handleTerminalEvent(event: Exclude<StreamEvent, { type: "started" | "delta" }>): void {
    if (event.type === "completed") {
      phase = "completed";
      backendStatus = "completed";
    } else if (event.type === "cancelled") {
      phase = "cancelled";
      backendStatus = "cancelled";
    } else {
      phase = "failed";
      backendStatus = "failed";
      errorMessage = `${event.error.code}: ${event.error.message}`;
    }
  }

  function recordDeltaArrival(run: RunContext): void {
    const arrivalMs = performance.now();
    if (run.previousDeltaArrivalMs !== null) {
      const intervalMs = arrivalMs - run.previousDeltaArrivalMs;
      minDeltaIntervalMs =
        minDeltaIntervalMs === null ? intervalMs : Math.min(minDeltaIntervalMs, intervalMs);
      maxDeltaIntervalMs =
        maxDeltaIntervalMs === null ? intervalMs : Math.max(maxDeltaIntervalMs, intervalMs);
    }
    run.previousDeltaArrivalMs = arrivalMs;
  }

  function handleStreamEvent(run: RunContext, event: StreamEvent): void {
    if (activeRun !== run) return;

    const validation = validateStreamEvent(run.contractState, event);
    if (!validation.accepted) {
      reportProtocolError(validation.error);
      return;
    }

    const acceptedEvent = validation.event;
    run.contractState = validation.nextState;
    const observedInFlight = Math.max(0, acceptedEvent.seq - (lastAckedSeq ?? -1));
    peakBackendInFlight = Math.max(peakBackendInFlight, observedInFlight);
    requestId ??= acceptedEvent.requestId;
    lastSeq = acceptedEvent.seq;
    const terminalExpectation = validation.terminalExpectation;

    if (acceptedEvent.type === "started") {
      batchWindowMs = acceptedEvent.batchWindowMs;
      maxInFlight = acceptedEvent.maxInFlight;
      backendStatus = "streaming";
      phase = "streaming";
    } else if (acceptedEvent.type === "delta") {
      recordDeltaArrival(run);
      updateCount += 1;
      partialText += acceptedEvent.text;
      backendStatus = "streaming";
      phase = "streaming";
    } else {
      terminalSeen = true;
      finalSnapshotPending = true;
      if (terminalExpectation === null) {
        reportProtocolError("종료 이벤트의 스냅샷 기대값을 만들지 못했습니다.");
        return;
      }
      run.expectedTerminal = terminalExpectation;
      handleTerminalEvent(acceptedEvent);
    }

    queueAcknowledgement(run, acceptedEvent);

    if (terminalExpectation !== null) {
      void queryFinalSnapshot(run, terminalExpectation);
    }
  }

  async function startStream(): Promise<void> {
    if (isBusy || !ackDelayIsValid || ackDelayMs === undefined) return;

    resetDisplay();
    appliedAckDelayMs = ackDelayMs;
    const run = {
      channel: null as unknown as ReturnType<typeof createStreamChannel>,
      ackChain: Promise.resolve(),
      ackDelayMs,
      previousDeltaArrivalMs: null,
      contractState: createStreamContractState(),
      expectedTerminal: null,
    } satisfies RunContext;

    run.channel = createStreamChannel((event) => handleStreamEvent(run, event));
    activeRun = run;

    try {
      const response = await startMockStream(run.channel);
      if (activeRun !== run) return;

      if (requestId !== null && requestId !== response.requestId) {
        reportProtocolError(`시작 응답의 요청 ID가 다릅니다: ${response.requestId}`);
        return;
      }

      requestId = response.requestId;
      if (run.contractState.requestId === null) {
        run.contractState = {
          ...run.contractState,
          requestId: response.requestId,
        };
      }
    } catch (error) {
      if (activeRun === run) {
        phase = "failed";
        backendStatus = "failed";
        errorMessage = `스트림 시작 실패: ${describeCommandError(error)}`;
      }
    }
  }

  async function cancelCurrentStream(): Promise<void> {
    const run = activeRun;
    const currentRequestId = requestId;
    if (run === null || currentRequestId === null || !canCancel) return;

    phase = "cancelling";

    try {
      const response = await cancelStream(currentRequestId);
      if (activeRun !== run) return;

      if (response.requestId !== currentRequestId) {
        reportProtocolError(`중단 응답의 요청 ID가 다릅니다: ${response.requestId}`);
      } else if (!response.accepted && phase === "cancelling") {
        errorMessage = "백엔드가 중단 요청을 수락하지 않았습니다.";
        phase = "streaming";
      }
    } catch (error) {
      if (activeRun === run && phase === "cancelling") {
        errorMessage = `중단 요청 실패: ${describeCommandError(error)}`;
        phase = backendStatus === "queued" ? "starting" : "streaming";
      }
    }
  }
</script>

<svelte:head>
  <meta
    name="description"
    content="LorePia M-1 Tauri Channel 스트리밍 동작을 검증하는 기능 스파이크"
  />
</svelte:head>

<main>
  <nav><a href="/isolation">플러그인 격리 실증 열기</a></nav>
  <header>
    <h1>M-1 Channel 스트리밍 실증</h1>
    <p>결정론적 mock 스트림의 순서, ACK, 중단, 부분 텍스트 저장 상태를 확인합니다.</p>
  </header>

  <section aria-labelledby="controls-heading">
    <h2 id="controls-heading">제어</h2>
    <div class="setting">
      <label for="ack-delay">ACK 지연 (ms)</label>
      <input
        id="ack-delay"
        type="number"
        min="0"
        max="10000"
        step="1"
        bind:value={ackDelayMs}
        disabled={isBusy}
        aria-invalid={!ackDelayIsValid}
        aria-describedby="ack-delay-help"
      />
      <p id="ack-delay-help">각 유효 이벤트의 ACK 전 대기 시간입니다. 기본값은 0ms입니다.</p>
      {#if !ackDelayIsValid}
        <p class="error" role="alert">0~10000 사이의 정수만 입력할 수 있습니다.</p>
      {/if}
    </div>
    <div class="controls" role="group" aria-label="스트림 제어">
      <button type="button" onclick={startStream} disabled={isBusy || !ackDelayIsValid}>
        스트림 시작
      </button>
      <button type="button" onclick={cancelCurrentStream} disabled={!canCancel}>스트림 중단</button>
    </div>
  </section>

  <section aria-labelledby="status-heading" aria-busy={isBusy}>
    <h2 id="status-heading">현재 상태</h2>
    <p>
      <output aria-live="polite" aria-atomic="true">{phaseLabel[phase]}</output>
    </p>
    <dl>
      <dt>백엔드 상태</dt>
      <dd>{backendStatus === null ? "—" : backendStatusLabel[backendStatus]}</dd>
      <dt>요청 ID</dt>
      <dd>{requestId ?? "—"}</dd>
      <dt>마지막 seq</dt>
      <dd>{lastSeq ?? "—"}</dd>
      <dt>delta 업데이트 수</dt>
      <dd>{updateCount}</dd>
      <dt>적용 ACK 지연</dt>
      <dd>{appliedAckDelayMs === null ? "—" : `${appliedAckDelayMs} ms`}</dd>
      <dt>마지막 ACK seq</dt>
      <dd>{lastAckedSeq ?? "—"}</dd>
      <dt>대기 중 ACK</dt>
      <dd>{pendingAckCount}</dd>
      <dt>ACK 대기 최고치</dt>
      <dd>{peakPendingAckCount}</dd>
      <dt>백엔드 in-flight</dt>
      <dd>{backendInFlight}</dd>
      <dt>백엔드 in-flight 최고치</dt>
      <dd>{peakBackendInFlight}</dd>
      <dt>delta 도착 간격 최소</dt>
      <dd>{formatMilliseconds(minDeltaIntervalMs)}</dd>
      <dt>delta 도착 간격 최대</dt>
      <dd>{formatMilliseconds(maxDeltaIntervalMs)}</dd>
      <dt>배칭 윈도우</dt>
      <dd>{batchWindowMs === null ? "—" : `${batchWindowMs} ms`}</dd>
      <dt>최종 배칭 윈도우</dt>
      <dd>{formatMilliseconds(effectiveBatchWindowMs)}</dd>
      <dt>배칭 윈도우 증가</dt>
      <dd>{formatMilliseconds(batchWindowIncreaseMs)}</dd>
      <dt>backpressure 관측</dt>
      <dd>
        {backpressureObserved === null
          ? "종료 후 판정"
          : backpressureObserved
            ? "관측됨"
            : "관측되지 않음"}
      </dd>
      <dt>최대 in-flight</dt>
      <dd>{maxInFlight ?? "—"}</dd>
      <dt>종료 이벤트</dt>
      <dd>{terminalSeen ? "수신함" : "미수신"}</dd>
      <dt>최종 스냅샷 조회</dt>
      <dd>{finalSnapshotPending ? "ACK 완료 대기 또는 조회 중" : snapshot === null ? "미완료" : "검증 완료"}</dd>
    </dl>
    {#if errorMessage !== null}
      <p class="error" role="alert">{errorMessage}</p>
    {/if}
  </section>

  <section aria-labelledby="partial-heading">
    <h2 id="partial-heading">부분 텍스트</h2>
    {#if partialText.length > 0}
      <pre aria-label="현재까지 수신한 부분 텍스트">{partialText}</pre>
    {:else}
      <p>아직 수신된 텍스트가 없습니다.</p>
    {/if}
  </section>

  <section aria-labelledby="snapshot-heading">
    <h2 id="snapshot-heading">최종 스냅샷</h2>
    {#if snapshot !== null}
      <dl>
        <dt>상태</dt>
        <dd>{backendStatusLabel[snapshot.status]}</dd>
        <dt>마지막 seq</dt>
        <dd>{snapshot.lastSeq}</dd>
        <dt>마지막 ACK seq</dt>
        <dd>{snapshot.lastAckedSeq}</dd>
        <dt>in-flight</dt>
        <dd>{snapshot.inFlight}</dd>
        <dt>기본 배칭 윈도우</dt>
        <dd>{snapshot.batchWindowMs} ms</dd>
        <dt>최종 배칭 윈도우</dt>
        <dd>{snapshot.effectiveBatchWindowMs} ms</dd>
      </dl>
      <h3>저장된 텍스트</h3>
      <pre aria-label="백엔드 최종 스냅샷 텍스트">{snapshot.text}</pre>
    {:else}
      <p>종료 이벤트를 받으면 ACK 완료 후 자동으로 조회합니다.</p>
    {/if}
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
    max-width: 56rem;
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

  .setting {
    margin-block-end: 1rem;
  }

  .setting label {
    display: block;
    font-weight: 600;
  }

  .setting p {
    margin-block: 0.25rem;
  }

  button,
  input {
    padding: 0.45rem 0.75rem;
    font: inherit;
  }

  input[type="number"] {
    width: 8rem;
  }

  dl {
    display: grid;
    grid-template-columns: max-content minmax(0, 1fr);
    gap: 0.25rem 1rem;
  }

  dt {
    font-weight: 600;
  }

  dd {
    margin: 0;
    overflow-wrap: anywhere;
  }

  pre {
    box-sizing: border-box;
    min-height: 4rem;
    margin: 0;
    padding: 0.75rem;
    overflow: auto;
    border: 1px solid currentcolor;
    font: inherit;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }

  .error {
    color: #b00020;
    font-weight: 600;
  }

  @media (prefers-color-scheme: dark) {
    .error {
      color: #ffb4ab;
    }
  }
</style>
