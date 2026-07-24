<script lang="ts">
  import { goto } from "$app/navigation";

  import "$lib/design/tokens.css";

  import { SAMPLE_CHARACTERS } from "$lib/characters/sample";
  import Avatar from "$lib/ui/Avatar.svelte";
  import { activateBackSwipeSurface } from "$lib/ui/back-swipe-surface";
  import LargeTitleHeader from "$lib/ui/LargeTitleHeader.svelte";
  import { edgeSwipeBack } from "$lib/ui/edge-back";

  // Placeholder for the site-backed storefront; swaps to live community
  // cards once the sharing service is wired.
  const featured = SAMPLE_CHARACTERS.slice(0, 5);

  function navigateBack(event?: MouseEvent): void {
    event?.preventDefault();
    void goto("/home", { replaceState: true });
  }
</script>

<svelte:head>
  <title>LorePia — 커뮤니티</title>
</svelte:head>

<div
  class="screen"
  use:edgeSwipeBack={{
    onBack: navigateBack,
    getUnderlay: () => activateBackSwipeSurface("/home"),
  }}
>
  <LargeTitleHeader title="커뮤니티">
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

  <section class="feature" aria-label="추천 캐릭터">
    <h2>추천 캐릭터 <span class="preview-tag">예시 미리보기</span></h2>
    <div class="rail">
      {#each featured as character (character.id)}
        <article class="card">
          <Avatar initial={character.initial} size={48} />
          <span class="name">{character.name}</span>
          <span class="tagline">{character.tagline}</span>
          <button class="get" type="button" disabled>받기</button>
        </article>
      {/each}
    </div>
  </section>

  <p class="note">
    사이트 연동 후 커뮤니티 카드가 여기에 흐릅니다. 계정으로 로그인하면 내
    캐릭터 카드를 올리고 받을 수 있습니다.
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

  .feature {
    margin-top: var(--sp-2);
  }

  .feature h2 {
    display: flex;
    align-items: center;
    gap: var(--sp-2);
    margin: 0 var(--sp-4) var(--sp-2);
    font-size: var(--fs-chat);
    font-weight: 600;
    letter-spacing: -0.01em;
    color: var(--text-strong);
  }

  .preview-tag {
    font-size: var(--fs-caption);
    font-weight: 400;
    color: var(--text-faint);
    border: 0.5px solid var(--hairline);
    border-radius: var(--r-pill);
    padding: 1px var(--sp-2);
  }

  /* The storefront rail: horizontally scrolling cards with the page gutter
     as scroll padding, App Store style. */
  .rail {
    display: flex;
    gap: var(--sp-3);
    overflow-x: auto;
    padding: 2px var(--sp-4) var(--sp-2);
    scroll-snap-type: x proximity;
    /* Snap positions must honor the page gutter, or the snapped card pins
       flush to the screen edge. */
    scroll-padding-inline: var(--sp-4);
    scrollbar-width: none;
  }

  .rail::-webkit-scrollbar {
    display: none;
  }

  .card {
    flex: none;
    scroll-snap-align: start;
    width: 128px;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--sp-1);
    padding: var(--sp-4) var(--sp-3);
    box-sizing: border-box;
    background: var(--surface-card);
    border-radius: var(--r-card);
    box-shadow: var(--shadow-card);
    text-align: center;
    animation: lp-rise var(--dur-page) var(--ease-out) backwards;
    animation-delay: 40ms;
  }

  .card .name {
    margin-top: var(--sp-1);
    font-size: var(--fs-ui);
    font-weight: 600;
    color: var(--text-strong);
    max-width: 100%;
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
  }

  .card .tagline {
    font-size: var(--fs-caption);
    line-height: 1.35;
    color: var(--text-mid);
    display: -webkit-box;
    -webkit-box-orient: vertical;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    overflow: hidden;
  }

  /* The App Store "GET" pill; enabled once cards can actually be pulled
     from the site. */
  .get {
    margin-top: var(--sp-2);
    min-height: 28px;
    padding: 0 var(--sp-3);
    border: 0;
    border-radius: var(--r-pill);
    background: var(--tint-soft);
    color: var(--tint);
    font-family: var(--font-ui);
    font-size: var(--fs-label);
    font-weight: 700;
    cursor: pointer;
  }

  .get:disabled {
    opacity: 0.45;
    cursor: default;
  }

  .note {
    margin: var(--sp-2) var(--sp-4) 0;
    padding-bottom: calc(var(--sp-5) + var(--safe-bottom));
    font-size: var(--fs-label);
    line-height: 1.6;
    color: var(--text-mid);
  }

  @media (min-width: 700px) {
    .feature {
      width: min(100%, calc(680px + var(--sp-4) * 2));
      margin-inline: auto;
      box-sizing: border-box;
    }

    .note {
      width: min(100% - var(--sp-4) * 2, 680px);
      margin-inline: auto;
      box-sizing: border-box;
    }
  }
</style>
