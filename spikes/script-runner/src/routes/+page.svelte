<script lang="ts">
  import { scriptRunner } from "$lib/runner-controller";
  import type { ProbeSuiteReceipt } from "$lib/runner-contract";

  let running = $state(false);
  let receipt = $state<ProbeSuiteReceipt | null>(null);
  let errorCode = $state<string | null>(null);

  async function runProbe(): Promise<void> {
    running = true;
    receipt = null;
    errorCode = null;
    try {
      receipt = await scriptRunner.runProbeSuite();
    } catch (error) {
      errorCode = error instanceof Error ? error.message : "PROBE_FAILED";
    } finally {
      running = false;
    }
  }
</script>

<svelte:head>
  <title>LorePia M-1 Script Runner</title>
</svelte:head>

<main>
  <h1>종료 가능한 Script Runner</h1>
  <p>
    임포트 코드는 Tauri IPC나 호스트 WebView에서 실행하지 않습니다. 각 케이스는
    고정 최대 메모리의 새 QuickJS-WASM Worker에서 실행되고, 엔진 인터럽트가
    실패해도 호스트가 Worker를 외부 종료합니다.
  </p>

  <button type="button" disabled={running} onclick={runProbe}>
    {running ? "15개 경계 케이스 실행 중…" : "전체 경계 실증 실행"}
  </button>

  {#if errorCode}
    <p role="alert">실증 실패: {errorCode}</p>
  {:else if receipt}
    <p role="status">
      결과: {receipt.passed}/{receipt.total}
      {receipt.passed === receipt.total ? "PASS" : "FAIL"}
    </p>
    <table>
      <thead>
        <tr>
          <th>케이스</th>
          <th>코드</th>
          <th>결과</th>
          <th>ms</th>
          <th>호스트 heartbeat</th>
        </tr>
      </thead>
      <tbody>
        {#each receipt.cases as item}
          <tr>
            <td>{item.caseId}</td>
            <td>{item.code}</td>
            <td>{item.passed ? "PASS" : "FAIL"}</td>
            <td>{item.elapsedMs.toFixed(1)}</td>
            <td>{item.hostHeartbeatTicks}</td>
          </tr>
        {/each}
      </tbody>
    </table>
    <details>
      <summary>bounded receipt</summary>
      <pre>{JSON.stringify(receipt, null, 2)}</pre>
    </details>
  {/if}
</main>

<style>
  :global(body) {
    margin: 0;
    background: #10131a;
    color: #f4f6fb;
    font-family: ui-sans-serif, system-ui, sans-serif;
  }
  main {
    max-width: 960px;
    margin: 0 auto;
    padding: 2rem;
  }
  p {
    line-height: 1.6;
  }
  button {
    margin: 1rem 0;
    padding: 0.75rem 1rem;
  }
  table {
    width: 100%;
    border-collapse: collapse;
    font-size: 0.85rem;
  }
  th,
  td {
    border: 1px solid #3a4354;
    padding: 0.5rem;
    text-align: left;
  }
  pre {
    max-height: 20rem;
    overflow: auto;
    white-space: pre-wrap;
  }
</style>
