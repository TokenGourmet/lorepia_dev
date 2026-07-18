<script lang="ts">
  import {
    LuaProbeCommandError,
    LuaProbeProtocolError,
    runLuaLimitsM1Probe,
    type LuaProbeSuccess,
  } from "$lib/lua-probe";

  type UiPhase = "idle" | "running" | "passed" | "failed";

  let phase = $state<UiPhase>("idle");
  let receipt = $state("아직 실행하지 않았습니다.");

  function successReceipt(result: LuaProbeSuccess): string {
    const interrupted = result.cases.filter(
      (entry) => entry.outcome === "INTERRUPTED",
    ).length;
    const verified = result.cases.length - interrupted;
    return [
      "Lua 실행 한도 검증 통과",
      `정책: ${result.policyVersion}`,
      `fixture catalog SHA-256: ${result.fixtureCatalogSha256}`,
      `런타임: ${result.luaVersion} / mlua ${result.mluaVersion}`,
      `검증 케이스: 완료·검증 ${verified} / 제한 중단 ${interrupted}`,
      `시간/명령 한도: ${result.limits.deadlineMs}ms / ${result.limits.instructionCap}`,
      `메모리 한도: ${result.limits.memoryCeilingBytes} bytes`,
      "위험 stdlib·보호호출·coroutine 우회: 제거됨",
      "적대 fixture 이후 host 회복: 확인",
    ].join("\n");
  }

  async function runProbe(): Promise<void> {
    if (phase === "running") return;
    phase = "running";
    receipt = "고정 Lua 5.4 한도 fixture를 검증 중입니다.";
    try {
      const result = await runLuaLimitsM1Probe();
      phase = "passed";
      receipt = successReceipt(result);
    } catch (error) {
      phase = "failed";
      if (error instanceof LuaProbeCommandError) {
        receipt = error.failure.code === "PROBE_BUSY"
          ? "다른 Lua 한도 검증이 실행 중입니다."
          : "Lua 한도 검증이 제한된 내부 오류로 중단되었습니다.";
      } else if (error instanceof LuaProbeProtocolError) {
        receipt = "네이티브 응답 형식이 제한된 계약과 다릅니다.";
      } else {
        receipt = "Lua 한도 검증 호출을 완료하지 못했습니다.";
      }
    }
  }
</script>

<svelte:head>
  <title>LorePia M-1 Lua Limits 실증</title>
</svelte:head>

<main>
  <h1>M-1 Lua Limits 실증</h1>
  <button type="button" onclick={runProbe} disabled={phase === "running"}>
    {phase === "running" ? "검증 중" : "Lua 한도 검증 실행"}
  </button>
  <pre aria-live="polite">{receipt}</pre>
</main>
