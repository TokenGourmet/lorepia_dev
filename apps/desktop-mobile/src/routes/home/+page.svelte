<script lang="ts">
  import "$lib/design/tokens.css";

  import LargeTitleHeader from "$lib/ui/LargeTitleHeader.svelte";
  import { minimizeDockOnScroll } from "$lib/ui/dock-chrome.svelte";
</script>

<svelte:head>
  <title>LorePia — 홈</title>
</svelte:head>

<div class="screen" use:minimizeDockOnScroll>
  <LargeTitleHeader title="홈" />

  <section class="quick" aria-label="빠른 작업">
    <a class="entry" href="/import">
      <span class="ic" aria-hidden="true">
        <svg
          viewBox="0 0 24 24"
          width="22"
          height="22"
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
      <span class="txt">
        <span class="t">캐릭터 가져오기</span>
        <span class="s">PNG 카드 · ZIP 아카이브</span>
      </span>
      <svg
        class="chev"
        viewBox="0 0 24 24"
        width="16"
        height="16"
        fill="none"
        stroke="currentColor"
        stroke-width="2.2"
        stroke-linecap="round"
        stroke-linejoin="round"
        aria-hidden="true"
      >
        <path d="m9 18 6-6-6-6" />
      </svg>
    </a>

    <a class="entry" href="/community">
      <span class="ic" aria-hidden="true">
        <svg
          viewBox="0 0 24 24"
          width="22"
          height="22"
          fill="none"
          stroke="currentColor"
          stroke-width="2"
          stroke-linecap="round"
          stroke-linejoin="round"
        >
          <path d="M12 3a9 9 0 1 0 0 18 9 9 0 0 0 0-18" />
          <path d="M3.5 12h17" />
          <path d="M12 3a13.5 13.5 0 0 1 0 18" />
          <path d="M12 3a13.5 13.5 0 0 0 0 18" />
        </svg>
      </span>
      <span class="txt">
        <span class="t">커뮤니티</span>
        <span class="s">사이트 연동 준비 중 · 카드 공유</span>
      </span>
      <svg
        class="chev"
        viewBox="0 0 24 24"
        width="16"
        height="16"
        fill="none"
        stroke="currentColor"
        stroke-width="2.2"
        stroke-linecap="round"
        stroke-linejoin="round"
        aria-hidden="true"
      >
        <path d="m9 18 6-6-6-6" />
      </svg>
    </a>
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

  .quick {
    display: flex;
    flex-direction: column;
    gap: var(--sp-3);
    margin: var(--sp-2) var(--sp-4) 0;
    padding-bottom: var(--safe-bottom);
  }

  .entry {
    display: flex;
    align-items: center;
    gap: var(--sp-3);
    padding: var(--sp-3) var(--sp-4);
    background: var(--surface-card);
    border-radius: var(--r-card);
    box-shadow: var(--shadow-card);
    text-decoration: none;
    animation: lp-rise var(--dur-page) var(--ease-out) backwards;
    transition: transform var(--dur-base) var(--ease-spring);
  }

  .entry:nth-child(1) {
    animation-delay: 40ms;
  }

  .entry:nth-child(2) {
    animation-delay: 90ms;
  }

  .entry:active {
    transform: scale(0.97);
  }

  .ic {
    width: 44px;
    height: 44px;
    flex-shrink: 0;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    border-radius: 14px;
    background: var(--tint-soft);
    color: var(--tint);
  }

  .txt {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .t {
    font-size: var(--fs-chat);
    font-weight: 600;
    letter-spacing: -0.01em;
    color: var(--text-strong);
  }

  .s {
    font-size: var(--fs-label);
    color: var(--text-mid);
  }

  .chev {
    flex-shrink: 0;
    color: var(--text-faint);
  }

  @media (min-width: 700px) {
    .quick {
      width: min(100% - var(--sp-4) * 2, 680px);
      margin-inline: auto;
      box-sizing: border-box;
    }
  }
</style>
