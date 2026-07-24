<script lang="ts">
  import "$lib/design/tokens.css";

  import LargeTitleHeader from "$lib/ui/LargeTitleHeader.svelte";
  import { minimizeDockOnScroll } from "$lib/ui/dock-chrome.svelte";
</script>

<svelte:head>
  <title>LorePia — 생성</title>
</svelte:head>

<div class="screen" use:minimizeDockOnScroll>
  <LargeTitleHeader title="생성" />

  <section class="coming-soon" aria-labelledby="create-status-title">
    <span class="create-icon" aria-hidden="true">
      <svg
        viewBox="0 0 24 24"
        width="28"
        height="28"
        fill="none"
        stroke="currentColor"
        stroke-width="1.8"
        stroke-linecap="round"
        stroke-linejoin="round"
      >
        <circle cx="12" cy="8" r="3.5" />
        <path d="M5.5 20c.7-4 3.1-6 6.5-6s5.8 2 6.5 6" />
        <path d="M19 3v4" />
        <path d="M17 5h4" />
      </svg>
    </span>
    <span class="status-label">기능 준비 중</span>
    <h1 id="create-status-title">캐릭터 생성은 아직 사용할 수 없습니다</h1>
    <p class="summary">
      이름과 설정을 입력받는 화면보다, 만든 캐릭터가 실제 기기 저장소와
      서재에 안전하게 남는 흐름을 먼저 연결하고 있습니다.
    </p>

    <div class="readiness">
      <strong>활성화 조건</strong>
      <p>
        캐릭터 정보 저장, 이미지 보관, 서재 반영이 하나의 흐름으로 검증된 뒤
        생성 화면을 엽니다. 입력 내용이 저장되지 않는 임시 생성 화면은
        제공하지 않습니다.
      </p>
    </div>
  </section>
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

  .coming-soon {
    width: min(calc(100% - var(--sp-4) * 2), 680px);
    margin: var(--sp-2) auto 0;
    box-sizing: border-box;
    padding: var(--sp-6) var(--sp-5);
    display: flex;
    flex-direction: column;
    align-items: center;
    text-align: center;
    animation: lp-rise var(--dur-page) var(--ease-out) backwards;
    animation-delay: 40ms;
  }

  .create-icon {
    width: 64px;
    height: 64px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    border-radius: 20px;
    background: var(--tint-soft);
    color: var(--tint);
  }

  .status-label {
    margin-top: var(--sp-4);
    padding: 3px var(--sp-2);
    border-radius: var(--r-pill);
    background: var(--warning-soft);
    color: var(--warning);
    font-size: var(--fs-caption);
    font-weight: 600;
  }

  h1 {
    margin: var(--sp-3) 0 0;
    color: var(--text-strong);
    font-size: var(--fs-bartitle);
    font-weight: 600;
    letter-spacing: -0.01em;
  }

  .summary {
    max-width: 40ch;
    margin: var(--sp-2) 0 0;
    color: var(--text-mid);
    font-size: var(--fs-ui);
    line-height: 1.6;
  }

  .readiness {
    width: 100%;
    max-width: 46ch;
    margin-top: var(--sp-5);
    box-sizing: border-box;
    padding-top: var(--sp-4);
    border-top: 0.5px solid var(--hairline);
    text-align: left;
  }

  .readiness strong {
    color: var(--text-strong);
    font-size: var(--fs-label);
    font-weight: 600;
  }

  .readiness p {
    margin: var(--sp-1) 0 0;
    color: var(--text-mid);
    font-size: var(--fs-label);
    line-height: 1.6;
  }

  .coming-soon {
    margin-bottom: var(--safe-bottom);
  }

  @media (max-width: 420px) {
    .coming-soon {
      padding: var(--sp-5) var(--sp-4);
    }
  }
</style>
