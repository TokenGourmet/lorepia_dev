<script lang="ts">
  import {
    ImportProbeCommandError,
    ImportProbeProtocolError,
    runImportHardeningM1Probe,
    type ImportProbeErrorCode,
    type ImportProbeSuccess,
  } from "$lib/import-probe";

  type UiPhase = "idle" | "running" | "passed" | "failed";

  const errorLabel: Record<ImportProbeErrorCode, string> = {
    PATH_UNAVAILABLE: "검증용 가져오기 경로를 준비할 수 없습니다.",
    SOURCE_TOO_LARGE: "원본 파일이 검증 한도를 초과했습니다.",
    UNSUPPORTED_FORMAT: "지원하지 않는 원본 형식입니다.",
    ARCHIVE_MALFORMED: "아카이브 구조가 올바르지 않습니다.",
    UNSUPPORTED_COMPRESSION: "허용하지 않는 압축 방식입니다.",
    ENTRY_COUNT_LIMIT: "아카이브 항목 수 한도를 초과했습니다.",
    ENTRY_SIZE_LIMIT: "아카이브 단일 항목 크기 한도를 초과했습니다.",
    TOTAL_SIZE_LIMIT: "아카이브 전체 해제 크기 한도를 초과했습니다.",
    COMPRESSION_RATIO_LIMIT: "압축률 방어 한도를 초과했습니다.",
    UNSAFE_PATH: "안전하지 않은 아카이브 경로를 거부했습니다.",
    DUPLICATE_PATH: "충돌하거나 중복된 아카이브 경로를 거부했습니다.",
    UNSAFE_ENTRY_TYPE: "안전하지 않은 아카이브 항목 유형을 거부했습니다.",
    PNG_MALFORMED: "PNG 구조 검증에 실패했습니다.",
    UNSUPPORTED_FILE_TYPE: "허용하지 않는 내부 파일 유형입니다.",
    STAGING_FAILURE: "격리된 임시 영역을 준비할 수 없습니다.",
    PUBLISH_CONFLICT: "검증 결과 게시 대상이 충돌했습니다.",
    PUBLISH_FAILURE: "검증 결과를 원자적으로 게시하지 못했습니다.",
    CLEANUP_FAILURE: "검증용 임시 파일 정리에 실패했습니다.",
    PROBE_BUSY: "다른 가져오기 검증이 실행 중입니다.",
    INTERNAL_STATE: "가져오기 검증 상태가 올바르지 않습니다.",
  };

  let phase = $state<UiPhase>("idle");
  let receipt = $state("아직 실행하지 않았습니다.");

  function successReceipt(result: ImportProbeSuccess): string {
    const acceptedCases = result.cases.filter((entry) => entry.outcome === "ACCEPTED").length;
    const rejectedCases = result.cases.filter((entry) => entry.outcome === "REJECTED").length;
    return [
      "Import hardening 검증 통과",
      `정책: ${result.policyVersion}`,
      `fixture catalog SHA-256: ${result.fixtureCatalogSha256}`,
      `검증 케이스: ACCEPTED ${acceptedCases} / REJECTED ${rejectedCases}`,
      `정상 archive SHA-256: ${result.validArchive.sourceSha256}`,
      `정상 archive bytes: source ${result.validArchive.sourceBytes} / uncompressed ${result.validArchive.totalUncompressedBytes}`,
      `정상 archive 항목: ${result.validArchive.entryCount}개, 스크립트 ${result.validArchive.scriptEntries}개 중 실행 0개`,
      `정상 PNG SHA-256: ${result.validDirectPng.sourceSha256}`,
      `정상 PNG bytes: ${result.validDirectPng.sourceBytes}, dimensions ${result.validDirectPng.width}x${result.validDirectPng.height}`,
      `ZIP/PNG parser: ${result.zipVersion} / ${result.pngVersion}`,
      "고정 외부 sentinel: 변경 없음",
      "스크립트: inert quarantine",
      "정리 대기: 아니요",
    ].join("\n");
  }

  async function runProbe(): Promise<void> {
    if (phase === "running") return;
    phase = "running";
    receipt = "고정 archive/PNG 방어 fixture를 검증 중입니다.";
    try {
      const result = await runImportHardeningM1Probe();
      phase = "passed";
      receipt = successReceipt(result);
    } catch (error) {
      phase = "failed";
      if (error instanceof ImportProbeCommandError) {
        const cleanup = error.failure.cleanupPending
          ? " 임시 검증 데이터 정리가 필요합니다."
          : "";
        receipt = `${errorLabel[error.failure.code]}${cleanup}`;
      } else if (error instanceof ImportProbeProtocolError) {
        receipt = "네이티브 응답 형식이 제한된 계약과 다릅니다.";
      } else {
        receipt = "가져오기 방어 검증 호출을 완료하지 못했습니다.";
      }
    }
  }
</script>

<svelte:head>
  <title>LorePia M-1 Import Hardening 실증</title>
</svelte:head>

<main>
  <h1>M-1 Import Hardening 실증</h1>
  <button type="button" onclick={runProbe} disabled={phase === "running"}>
    {phase === "running" ? "검증 중" : "가져오기 방어 검증 실행"}
  </button>
  <pre aria-live="polite">{receipt}</pre>
</main>
