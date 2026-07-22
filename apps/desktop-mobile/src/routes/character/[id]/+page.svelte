<script lang="ts">
  import { page } from "$app/state";

  import "$lib/design/tokens.css";

  import { findSampleCharacter } from "$lib/characters/sample";
  import Avatar from "$lib/ui/Avatar.svelte";

  const character = $derived(findSampleCharacter(page.params.id ?? ""));
</script>

<svelte:head>
  <title>LorePia — {character ? character.name : "캐릭터"}</title>
</svelte:head>

<div class="screen">
  <header class="top">
    <a class="back" href="/" aria-label="서재로 돌아가기">
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
  </header>

  {#if character}
    <section class="hero">
      <Avatar initial={character.initial} size={72} />
      <h1>{character.name}</h1>
      <p class="tagline">{character.tagline}</p>
    </section>

    <section class="about">
      <p class="voice">{character.description}</p>
    </section>

    <dl class="meta">
      <div class="meta-row">
        <dt>카드 테마</dt>
        <dd>기본 (종이·목탄)</dd>
      </div>
      <div class="meta-row">
        <dt>스크립트</dt>
        <dd>
          {#if character.scriptCount > 0}
            {character.scriptCount}개 · 보존됨, 실행되지 않음
          {:else}
            없음
          {/if}
        </dd>
      </div>
    </dl>

    <div class="actions">
      <a class="start" href="/chat">대화 시작</a>
    </div>
  {:else}
    <section class="missing">
      <h1>캐릭터를 찾을 수 없어요</h1>
      <p>서재로 돌아가 다시 선택해 주세요.</p>
      <a class="start" href="/">서재로</a>
    </section>
  {/if}
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

  .top {
    padding: var(--sp-2) var(--sp-3);
    padding-top: calc(var(--sp-2) + var(--safe-top));
  }

  .back {
    width: var(--size-touch);
    height: var(--size-touch);
    display: inline-flex;
    align-items: center;
    justify-content: center;
    border-radius: var(--r-pill);
    color: var(--text-mid);
  }

  .hero {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--sp-2);
    padding: var(--sp-4) var(--sp-4) var(--sp-5);
    text-align: center;
  }

  .hero h1 {
    margin: 0;
    font-size: 22px;
    font-weight: 500;
    color: var(--text-strong);
  }

  .tagline {
    margin: 0;
    font-size: var(--fs-ui);
    color: var(--text-mid);
  }

  .about {
    padding: 0 var(--sp-5);
    max-width: var(--measure-story);
    margin-inline: auto;
  }

  .voice {
    margin: 0;
    font-family: var(--font-voice);
    font-size: var(--fs-story);
    line-height: var(--lh-story);
    color: var(--text-strong);
  }

  .meta {
    margin: var(--sp-5) var(--sp-4) 0;
    border-top: 0.5px solid var(--hairline);
  }

  .meta-row {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: var(--sp-3);
    min-height: var(--size-touch);
    padding: var(--sp-3) var(--sp-1);
    border-bottom: 0.5px solid var(--hairline);
  }

  dt {
    font-size: var(--fs-label);
    color: var(--text-mid);
    flex-shrink: 0;
  }

  dd {
    margin: 0;
    font-size: var(--fs-ui);
    color: var(--text-strong);
    text-align: right;
  }

  .actions {
    margin-top: auto;
    padding: var(--sp-4);
    padding-bottom: calc(var(--sp-4) + var(--safe-bottom));
  }

  .start {
    width: 100%;
    min-height: 48px;
    box-sizing: border-box;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    border-radius: var(--r-pill);
    background: var(--invert-surface);
    color: var(--invert-text);
    font-size: var(--fs-chat);
    font-weight: 500;
    text-decoration: none;
    transition: transform var(--dur-fast) var(--ease-out);
  }

  .start:active {
    transform: scale(0.98);
  }

  .missing {
    flex: 1;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: var(--sp-2);
    padding: var(--sp-6);
    text-align: center;
  }

  .missing h1 {
    margin: 0;
    font-size: 18px;
    font-weight: 500;
    color: var(--text-strong);
  }

  .missing p {
    margin: 0;
    font-size: var(--fs-ui);
    color: var(--text-mid);
  }

  .missing .start {
    width: auto;
    padding: 0 var(--sp-5);
    margin-top: var(--sp-3);
  }
</style>
