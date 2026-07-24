<script lang="ts">
  import { goto } from "$app/navigation";
  import { page } from "$app/state";

  import "$lib/design/tokens.css";

  import { findSampleCharacter } from "$lib/characters/sample";
  import Avatar from "$lib/ui/Avatar.svelte";
  import { activateBackSwipeSurface } from "$lib/ui/back-swipe-surface";
  import { edgeSwipeBack } from "$lib/ui/edge-back";
  import {
    completeNativeBack,
    prepareNativeBack,
  } from "$lib/ui/native-back";

  const character = $derived(findSampleCharacter(page.params.id ?? ""));
  const chatHref = $derived(
    character
      ? `/chat?character=${encodeURIComponent(character.id)}`
      : "/chat",
  );

  async function openChat(event: MouseEvent): Promise<void> {
    event.preventDefault();
    const nativeStatus = await prepareNativeBack();
    try {
      await goto(chatHref, {
        state: {
          backHref: `${window.location.pathname}${window.location.search}${window.location.hash}`,
        },
      });
    } catch {
      if (nativeStatus.active) {
        await completeNativeBack();
      }
    }
  }

  function navigateBack(event?: MouseEvent): void {
    event?.preventDefault();
    void goto("/", { replaceState: true });
  }
</script>

<svelte:head>
  <title>LorePia — {character ? character.name : "캐릭터"}</title>
</svelte:head>

<div
  class="screen"
  use:edgeSwipeBack={{
    onBack: navigateBack,
    getUnderlay: () => activateBackSwipeSurface("/"),
  }}
>
  <header class="top">
    <a
      class="back"
      href="/"
      aria-label="서재로 돌아가기"
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
      <a class="start" href={chatHref} onclick={openChat}>대화 시작</a>
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
    position: sticky;
    top: 0;
    z-index: 5;
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
    color: var(--text-strong);
    background: var(--bar-bg);
    -webkit-backdrop-filter: blur(20px) saturate(1.6);
    backdrop-filter: blur(20px) saturate(1.6);
    box-shadow: var(--shadow-card);
    transition: transform var(--dur-fast) var(--ease-spring);
  }

  .back:active {
    transform: scale(0.92);
  }

  .hero {
    position: relative;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--sp-2);
    padding: var(--sp-4) var(--sp-4) var(--sp-5);
    text-align: center;
    animation: lp-pop var(--dur-page) var(--ease-spring) backwards;
  }

  .hero::before {
    content: "";
    position: absolute;
    top: -40px;
    width: 300px;
    height: 300px;
    border-radius: var(--r-pill);
    background: radial-gradient(closest-side, var(--tint-soft), transparent);
    pointer-events: none;
  }

  .hero > :global(*) {
    position: relative;
    z-index: 1;
  }

  .hero h1 {
    margin: 0;
    font-size: 28px;
    font-weight: 800;
    letter-spacing: -0.03em;
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

  .about {
    animation: lp-rise var(--dur-page) var(--ease-out) backwards;
    animation-delay: 80ms;
  }

  .meta {
    margin: var(--sp-5) var(--sp-4) 0;
    padding: 0 var(--sp-4);
    background: var(--surface-card);
    border-radius: var(--r-card);
    box-shadow: var(--shadow-card);
    animation: lp-rise var(--dur-page) var(--ease-out) backwards;
    animation-delay: 140ms;
  }

  .meta-row {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: var(--sp-3);
    min-height: var(--size-touch);
    padding: var(--sp-3) 0;
  }

  .meta-row + .meta-row {
    border-top: 0.5px solid var(--hairline);
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
    min-height: 50px;
    box-sizing: border-box;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    border-radius: var(--r-pill);
    background: var(--tint);
    color: #fff;
    font-size: var(--fs-chat);
    font-weight: 600;
    text-decoration: none;
    box-shadow: var(--shadow-card);
    transition: transform var(--dur-fast) var(--ease-spring);
  }

  .start:active {
    transform: scale(0.97);
  }

  @media (min-width: 700px) {
    .meta,
    .actions {
      width: min(100% - var(--sp-4) * 2, 680px);
      margin-left: auto;
      margin-right: auto;
      box-sizing: border-box;
    }

    .actions {
      padding-left: 0;
      padding-right: 0;
    }
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
