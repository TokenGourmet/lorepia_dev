<script lang="ts">
  import {
    KeychainProbeCommandError,
    KeychainProbeProtocolError,
    runKeychainM1Probe,
    type KeychainProbeErrorCode,
    type KeychainProbeSuccess,
  } from "$lib/keychain-probe";

  type UiPhase = "idle" | "running" | "passed" | "failed";

  const errorLabel: Record<KeychainProbeErrorCode, string> = {
    PROBE_BUSY: "다른 키체인 검증이 실행 중입니다.",
    STORE_UNAVAILABLE: "자격 증명 저장소를 사용할 수 없습니다.",
    STORE_LOCKED: "자격 증명 저장소가 잠겨 있습니다.",
    STORE_FAILURE: "자격 증명 저장소 검증에 실패했습니다.",
    CLEANUP_FAILED: "검증 항목 정리에 실패했습니다.",
    COLLISION: "검증용 항목 충돌이 발생했습니다.",
    RANDOM_FAILURE: "검증용 난수 생성에 실패했습니다.",
    INTERNAL_STATE: "키체인 검증 상태가 올바르지 않습니다.",
  };

  let phase = $state<UiPhase>("idle");
  let result = $state<KeychainProbeSuccess | null>(null);
  let statusText = $state("아직 실행하지 않았습니다.");

  async function runProbe(): Promise<void> {
    if (phase === "running") return;
    phase = "running";
    result = null;
    statusText = "키체인 생성·읽기·수정·삭제를 검증 중입니다.";

    try {
      result = await runKeychainM1Probe();
      phase = "passed";
      statusText = "키체인 수명주기 검증을 통과했습니다.";
    } catch (error) {
      phase = "failed";
      if (error instanceof KeychainProbeCommandError) {
        const cleanup = error.failure.cleanupPending
          ? " 임시 검증 항목 정리가 필요합니다."
          : "";
        statusText = `${errorLabel[error.failure.code]}${cleanup}`;
      } else if (error instanceof KeychainProbeProtocolError) {
        statusText = "네이티브 응답 형식이 계약과 다릅니다.";
      } else {
        statusText = "키체인 검증 호출을 완료하지 못했습니다.";
      }
    }
  }
</script>

<svelte:head>
  <title>LorePia M-1 Keychain 실증</title>
</svelte:head>

<main>
  <h1>M-1 Keychain 실증</h1>
  <button type="button" onclick={runProbe} disabled={phase === "running"}>
    {phase === "running" ? "검증 중" : "수명주기 검증 실행"}
  </button>
  <p aria-live="polite">{statusText}</p>

  {#if result !== null}
    <dl>
      <dt>백엔드</dt>
      <dd>{result.backend}</dd>
      <dt>실행 ID</dt>
      <dd>{result.runId}</dd>
      <dt>검증 참조 지문</dt>
      <dd>{result.referenceFingerprint}</dd>
      <dt>이전 미정리 항목 복구</dt>
      <dd>{result.staleCleanupRecovered ? "예" : "해당 없음"}</dd>
      <dt>정리 대기</dt>
      <dd>아니요</dd>
    </dl>
  {/if}
</main>
