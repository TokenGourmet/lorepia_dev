<script lang="ts">
  import {
    SQLiteProbeCommandError,
    SQLiteProbeProtocolError,
    runSQLiteM1Probe,
    type SQLiteProbeErrorCode,
    type SQLiteProbeSuccess,
  } from "$lib/sqlite-probe";

  type UiPhase = "idle" | "running" | "passed" | "failed";

  const errorLabel: Record<SQLiteProbeErrorCode, string> = {
    PATH_UNAVAILABLE: "검증용 데이터베이스 경로를 준비할 수 없습니다.",
    OPEN_FAILURE: "파일 데이터베이스를 열 수 없습니다.",
    MIGRATION_FAILURE: "데이터베이스 마이그레이션 검증에 실패했습니다.",
    PERSISTENCE_FAILURE: "데이터베이스 재실행 보존 검증에 실패했습니다.",
    CONCURRENCY_FAILURE: "WAL 읽기·쓰기 동시성 검증에 실패했습니다.",
    FTS_UNAVAILABLE: "이 SQLite 빌드에서 FTS5를 사용할 수 없습니다.",
    FTS_GOLDEN_MISMATCH: "한국어 검색 결과가 고정 fixture와 다릅니다.",
    CLEANUP_FAILURE: "검증용 데이터베이스 정리에 실패했습니다.",
    PROBE_BUSY: "다른 SQLite 검증이 실행 중입니다.",
    INTERNAL_STATE: "SQLite 검증 상태가 올바르지 않습니다.",
  };

  let phase = $state<UiPhase>("idle");
  let receipt = $state("아직 실행하지 않았습니다.");

  function successReceipt(result: SQLiteProbeSuccess): string {
    return [
      "SQLite/FTS5 검증 통과",
      `SQLite 버전: ${result.sqliteVersion}`,
      `스키마 버전: ${result.schemaVersion}`,
      `적용 마이그레이션: ${result.appliedMigrations.join(", ")}`,
      "미래 스키마 거부: 예",
      `저널 모드: ${result.concurrency.journalMode}`,
      `busy timeout: ${result.concurrency.busyTimeoutMs}ms`,
      `토크나이저: ${result.search.tokenizer}`,
      `짧은 질의 결과 한도: ${result.search.shortQueryLimit}개`,
      `golden 질의: ${result.search.golden.length}개`,
      `fixture SHA-256: ${result.fixtureSha256}`,
      `컴파일 옵션: ${result.compileOptions.join(", ")}`,
      "정리 대기: 아니요",
    ].join("\n");
  }

  async function runProbe(): Promise<void> {
    if (phase === "running") return;
    phase = "running";
    receipt = "파일 재실행, WAL 동시성, 한국어 FTS5를 검증 중입니다.";

    try {
      const result = await runSQLiteM1Probe();
      phase = "passed";
      receipt = successReceipt(result);
    } catch (error) {
      phase = "failed";
      if (error instanceof SQLiteProbeCommandError) {
        const cleanup = error.failure.cleanupPending
          ? " 임시 데이터베이스 정리가 필요합니다."
          : "";
        receipt = `${errorLabel[error.failure.code]}${cleanup}`;
      } else if (error instanceof SQLiteProbeProtocolError) {
        receipt = "네이티브 응답 형식이 계약과 다릅니다.";
      } else {
        receipt = "SQLite 검증 호출을 완료하지 못했습니다.";
      }
    }
  }
</script>

<svelte:head>
  <title>LorePia M-1 SQLite/FTS5 실증</title>
</svelte:head>

<main>
  <h1>M-1 SQLite/FTS5 실증</h1>
  <button type="button" onclick={runProbe} disabled={phase === "running"}>
    {phase === "running" ? "검증 중" : "SQLite/FTS5 검증 실행"}
  </button>
  <pre aria-live="polite">{receipt}</pre>
</main>
