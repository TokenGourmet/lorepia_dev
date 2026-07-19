<script lang="ts">
  import { onMount } from "svelte";

  import type { AudioReceipt, AudioState } from "$lib/audio-contract";
  import { AudioM1Controller } from "$lib/audio-controller";
  import { AudioM1Error } from "$lib/audio-error";
  import { installAudioLifecycleHooks } from "$lib/audio-lifecycle";

  type UiPhase = "idle" | "running" | "ready" | "failed";

  let controller = $state<AudioM1Controller | null>(null);
  let currentReceipt = $state<AudioReceipt | null>(null);
  let phase = $state<UiPhase>("idle");
  let receiptText = $state("초기화 중입니다.");

  function publish(receipt: AudioReceipt): void {
    currentReceipt = receipt;
    if (receipt.transition.action === "MEDIA_ERROR") {
      phase = "failed";
      receiptText = `${JSON.stringify(receipt, null, 2)}\n\n오류 코드: MEDIA_FAILURE`;
      return;
    }
    phase = "ready";
    receiptText = JSON.stringify(receipt, null, 2);
  }

  function boundedFailure(error: unknown): string {
    if (!(error instanceof AudioM1Error)) {
      return "오디오 검증 작업을 제한된 오류로 완료하지 못했습니다.";
    }
    const messages = {
      BUSY: "다른 오디오 검증 작업이 진행 중입니다.",
      INVALID_TRANSITION: "현재 상태에서는 그 오디오 동작을 실행할 수 없습니다.",
      APP_BACKGROUND: "백그라운드에서는 재생 동작을 실행할 수 없습니다.",
      FIXTURE_LOAD_FAILED: "고정 오디오 fixture를 읽지 못했습니다.",
      FIXTURE_TOO_LARGE: "고정 오디오 fixture가 크기 한도를 넘었습니다.",
      FIXTURE_MISMATCH: "고정 오디오 fixture의 길이 또는 SHA-256이 다릅니다.",
      FIXTURE_UNSUPPORTED: "고정 오디오 fixture 또는 WebView 디코더를 사용할 수 없습니다.",
      METADATA_MISMATCH: "WebView가 읽은 재생 시간이 고정 계약과 다릅니다.",
      MEDIA_EVENT_TIMEOUT: "필수 WebView 오디오 이벤트가 시간 안에 오지 않았습니다.",
      PLAY_REJECTED: "WebView가 사용자 시작 재생을 거부했습니다.",
      MEDIA_FAILURE: "WebView가 재생 중 미디어 오류를 보고했습니다.",
      PLAYBACK_NO_PROGRESS: "재생을 시작했지만 고정 시간 안에 media-time이 진행되지 않았습니다.",
      PLAYBACK_NON_MONOTONIC: "재생 관측 중 media-time이 허용 범위를 넘어 뒤로 움직였습니다.",
      PAUSE_UNSTABLE: "일시정지 관측 창에서 media-time이 허용 범위를 넘어 움직였습니다.",
      SEEK_FAILED: "고정 seek checkpoint에 도달하지 못했습니다.",
      LIFECYCLE_INTERRUPTED: "앱 생명주기 전환으로 진행 중 작업이 취소되었습니다.",
      RECEIPT_INVALID: "오디오 영수증이 제한된 계약과 다릅니다.",
    } as const;
    return `${messages[error.code]}\n오류 코드: ${error.code}`;
  }

  async function execute(
    operation: (active: AudioM1Controller) => AudioReceipt | Promise<AudioReceipt>,
  ): Promise<void> {
    if (controller === null || phase === "running") return;
    phase = "running";
    receiptText = "고정 오디오 fixture 검증을 실행 중입니다.";
    try {
      publish(await operation(controller));
    } catch (error) {
      phase = "failed";
      receiptText = boundedFailure(error);
    }
  }

  function stateIs(...states: AudioState[]): boolean {
    return currentReceipt !== null && states.includes(currentReceipt.state);
  }

  function disabledUnless(...states: AudioState[]): boolean {
    return (
      phase === "running" ||
      currentReceipt?.appPhase !== "FOREGROUND" ||
      !stateIs(...states)
    );
  }

  onMount(() => {
    controller = new AudioM1Controller({ onChange: publish });
    publish(controller.snapshot());
    const removeLifecycleHooks = installAudioLifecycleHooks(
      controller,
      document,
      window,
      publish,
    );
    return () => {
      removeLifecycleHooks();
      controller?.dispose();
      controller = null;
    };
  });
</script>

<svelte:head>
  <title>LorePia M-1 Audio Playback 실증</title>
</svelte:head>

<main>
  <h1>M-1 Audio Playback 실증</h1>
  <p>고정 로컬 WAV와 trusted WebView 재생 경로만 검증합니다.</p>
  <button type="button" onclick={() => execute((active) => active.load())} disabled={disabledUnless("RELEASED")}>Load</button>
  <button type="button" onclick={() => execute((active) => active.play())} disabled={disabledUnless("LOADED")}>Play</button>
  <button type="button" onclick={() => execute((active) => active.pause())} disabled={disabledUnless("PLAYING") || (currentReceipt?.actionCounts.PAUSE ?? 0) > 0}>Pause</button>
  <button type="button" onclick={() => execute((active) => active.seekCheckpoint())} disabled={disabledUnless("PAUSED") || (currentReceipt?.actionCounts.SEEK_CHECKPOINT ?? 0) > 0}>Seek 6000 ms</button>
  <button type="button" onclick={() => execute((active) => active.resume())} disabled={disabledUnless("PAUSED") || (currentReceipt?.actionCounts.RESUME ?? 0) > 0}>Resume</button>
  <button type="button" onclick={() => execute((active) => active.stop())} disabled={disabledUnless("PLAYING", "PAUSED", "ENDED") || (currentReceipt?.actionCounts.STOP ?? 0) > 0}>Stop</button>
  <button type="button" onclick={() => execute((active) => active.release())} disabled={disabledUnless("LOADED", "PLAYING", "PAUSED", "STOPPED", "ENDED")}>Release</button>
  <button type="button" onclick={() => execute((active) => active.snapshot())} disabled={phase === "running" || controller === null}>Snapshot</button>
  <pre aria-live="polite">{receiptText}</pre>
</main>
