<script lang="ts">
  import { goto } from "$app/navigation";

  import "$lib/design/tokens.css";

  import LargeTitleHeader from "$lib/ui/LargeTitleHeader.svelte";
  import { activateBackSwipeSurface } from "$lib/ui/back-swipe-surface";
  import { edgeSwipeBack } from "$lib/ui/edge-back";

  interface PreviewResult {
    id: string;
    file: string;
    verdict: "accepted" | "quarantined" | "rejected";
    detail: string;
  }

  const previewResults: PreviewResult[] = [
    {
      id: "r1",
      file: "seraphine.card.png",
      verdict: "accepted",
      detail: "카드 1장 · 스크립트 1개 보존됨, 실행되지 않음",
    },
    {
      id: "r2",
      file: "starlight-pack.zip",
      verdict: "quarantined",
      detail: "압축 경로가 허용 범위를 벗어나 격리됨",
    },
    {
      id: "r3",
      file: "unknown.bin",
      verdict: "rejected",
      detail: "지원하지 않는 형식",
    },
  ];

  const verdictLabel = {
    accepted: "가져옴",
    quarantined: "격리됨",
    rejected: "거부됨",
  } as const;

  function navigateBack(event?: MouseEvent): void {
    event?.preventDefault();
    void goto("/home", { replaceState: true });
  }
</script>

<svelte:head>
  <title>LorePia — 가져오기</title>
</svelte:head>

<div
  class="screen"
  use:edgeSwipeBack={{
    onBack: navigateBack,
    getUnderlay: () => activateBackSwipeSurface("/home"),
  }}
>
  <LargeTitleHeader title="가져오기">
    {#snippet leading()}
      <a
        class="back"
        href="/home"
        aria-label="홈으로 돌아가기"
        onclick={navigateBack}
      >
        <svg
          viewBox="0 0 24 24"
          width="20"
          height="20"
          fill="none"
          stroke="currentColor"
          stroke-width="2"
          stroke-linecap="round"
          stroke-linejoin="round"
          aria-hidden="true"
        >
          <path d="m15 18-6-6 6-6" />
        </svg>
      </a>
    {/snippet}
  </LargeTitleHeader>

  <section class="drop">
    <span class="drop-ic" aria-hidden="true">
      <svg
        viewBox="0 0 24 24"
        width="26"
        height="26"
        fill="none"
        stroke="currentColor"
        stroke-width="2"
        stroke-linecap="round"
        stroke-linejoin="round"
      >
        <path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4" />
        <path d="M7 10l5 5 5-5" />
        <path d="M12 15V3" />
      </svg>
    </span>
    <p class="drop-title">카드 파일을 선택하세요</p>
    <p class="drop-sub">PNG 카드와 ZIP 아카이브를 지원합니다</p>
    <button type="button" class="pick" disabled>파일 선택</button>
    <p class="drop-note">가져오기 기능은 임포터 연결 후 활성화됩니다</p>
  </section>

  <section class="results">
    <h2>검사 결과 <span class="preview-tag">예시 미리보기</span></h2>
    <ol>
      {#each previewResults as result (result.id)}
        <li>
          <div class="result-line">
            <span class="file">{result.file}</span>
            <span class="verdict {result.verdict}"
              >{verdictLabel[result.verdict]}</span
            >
          </div>
          <p class="detail">{result.detail}</p>
        </li>
      {/each}
    </ol>
  </section>

  <p class="policy">
    카드에 포함된 스크립트는 항상 보존만 되며, 검증된 실행 경계가 준비되기
    전까지 실행되지 않습니다.
  </p>
</div>

<style>
  .screen {
    height: 100%;
    overflow-y: auto;
    overscroll-behavior: none;
    display: flex;
    flex-direction: column;
    background: var(--surface-page);
    font-family: var(--font-ui);
  }

  .back {
    width: var(--size-touch);
    height: var(--size-touch);
    flex-shrink: 0;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    border-radius: var(--r-pill);
    color: var(--text-mid);
    transition:
      background var(--dur-fast) var(--ease-out),
      transform var(--dur-base) var(--ease-spring);
  }

  .back:active {
    background: var(--surface-bubble);
    transform: scale(0.9);
  }

  .drop {
    margin: var(--sp-2) var(--sp-4) 0;
    padding: var(--sp-6) var(--sp-4);
    background: var(--surface-card);
    border: 1.5px dashed var(--field-border);
    border-radius: var(--r-card);
    box-shadow: var(--shadow-card);
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--sp-1);
    text-align: center;
    animation: lp-rise var(--dur-page) var(--ease-out) backwards;
    animation-delay: 40ms;
  }

  .drop-ic {
    width: 56px;
    height: 56px;
    margin-bottom: var(--sp-2);
    display: inline-flex;
    align-items: center;
    justify-content: center;
    border-radius: 18px;
    background: var(--tint-soft);
    color: var(--tint);
  }

  .drop-title {
    margin: 0;
    font-size: 17px;
    font-weight: 700;
    letter-spacing: -0.01em;
    color: var(--text-strong);
  }

  .drop-sub {
    margin: 0;
    font-size: var(--fs-label);
    color: var(--text-mid);
  }

  .pick {
    margin-top: var(--sp-3);
    min-height: var(--size-touch);
    padding: 0 var(--sp-5);
    border: none;
    border-radius: var(--r-pill);
    background: var(--tint);
    color: #fff;
    font-family: var(--font-ui);
    font-size: var(--fs-ui);
    font-weight: 600;
    cursor: pointer;
    transition: transform var(--dur-fast) var(--ease-spring);
  }

  .pick:not(:disabled):active {
    transform: scale(0.95);
  }

  .pick:disabled {
    opacity: 0.35;
    cursor: default;
  }

  .drop-note {
    margin: var(--sp-1) 0 0;
    font-size: var(--fs-caption);
    color: var(--text-faint);
  }

  .results {
    padding: 0 var(--sp-4);
  }

  .results h2 {
    margin: var(--sp-3) 0 0;
    font-size: var(--fs-label);
    font-weight: 500;
    color: var(--text-mid);
    display: flex;
    align-items: center;
    gap: var(--sp-2);
  }

  .preview-tag {
    font-size: var(--fs-caption);
    font-weight: 400;
    color: var(--text-faint);
    border: 0.5px solid var(--hairline);
    border-radius: var(--r-pill);
    padding: 1px var(--sp-2);
  }

  .results ol {
    margin: var(--sp-2) 0 0;
    padding: 0;
    list-style: none;
    background: var(--surface-card);
    border-radius: var(--r-card);
    box-shadow: var(--shadow-card);
    overflow: hidden;
    animation: lp-rise var(--dur-page) var(--ease-out) backwards;
    animation-delay: 100ms;
  }

  .results li {
    padding: var(--sp-3) var(--sp-4);
  }

  .results li + li {
    border-top: 0.5px solid var(--hairline);
  }

  .result-line {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: var(--sp-3);
  }

  .file {
    font-size: var(--fs-ui);
    color: var(--text-strong);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .verdict {
    font-size: var(--fs-caption);
    flex-shrink: 0;
    border-radius: var(--r-pill);
    padding: 2px var(--sp-2);
  }

  .verdict {
    font-weight: 600;
  }

  .verdict.accepted {
    background: var(--success-soft);
    color: var(--success);
  }

  .verdict.quarantined {
    background: var(--warning-soft);
    color: var(--warning);
  }

  .verdict.rejected {
    background: var(--danger-soft);
    color: var(--danger);
  }

  .detail {
    margin: var(--sp-1) 0 0;
    font-size: var(--fs-label);
    color: var(--text-mid);
  }

  @media (min-width: 700px) {
    .drop {
      width: min(100% - var(--sp-4) * 2, 680px);
      margin-left: auto;
      margin-right: auto;
      box-sizing: border-box;
    }

    .results {
      width: min(100%, 712px);
      margin-inline: auto;
      box-sizing: border-box;
    }
  }

  .policy {
    margin: auto var(--sp-4) 0;
    padding: var(--sp-4) 0 calc(var(--sp-4) + var(--safe-bottom));
    font-size: var(--fs-caption);
    line-height: 1.6;
    color: var(--text-faint);
    text-align: center;
  }
</style>
