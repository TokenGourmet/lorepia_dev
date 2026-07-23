<script lang="ts">
  import { onMount } from "svelte";

  import "$lib/design/tokens.css";

  import { SAMPLE_CHARACTERS } from "$lib/characters/sample";
  import { librarySearch } from "$lib/characters/library-search.svelte";
  import { matchesQuery } from "$lib/characters/search";
  import { formatMessageTime } from "$lib/design/time-of-day";
  import Avatar from "$lib/ui/Avatar.svelte";
  import LargeTitleHeader from "$lib/ui/LargeTitleHeader.svelte";
  import { minimizeDockOnScroll } from "$lib/ui/dock-chrome.svelte";
  import {
    publicBootstrapError,
    requestProductBootstrap,
    type ProductBootstrap,
  } from "$lib/product-bootstrap";

  let bootstrap = $state<ProductBootstrap | null>(null);
  let errorMessage = $state<string | null>(null);
  let loading = $state(true);

  const characters = SAMPLE_CHARACTERS;

  /* The query lives in librarySearch: the mobile bottom search bar (owned by
     the layout) and this screen's desktop field both drive the same filter. */
  let searchFocused = $state(false);
  let searchInput = $state<HTMLInputElement | null>(null);

  const matches = $derived(
    characters.filter((character) =>
      matchesQuery(character, librarySearch.query),
    ),
  );

  // iOS keeps Cancel up while editing or while a query is in effect.
  const showCancel = $derived(searchFocused || librarySearch.query !== "");

  function clearQuery(): void {
    librarySearch.query = "";
  }

  function cancelSearch(): void {
    librarySearch.query = "";
    searchInput?.blur();
  }

  async function loadBootstrap(): Promise<void> {
    loading = true;
    errorMessage = null;

    try {
      bootstrap = await requestProductBootstrap();
    } catch {
      bootstrap = null;
      errorMessage = publicBootstrapError();
    } finally {
      loading = false;
    }
  }

  onMount(() => {
    void loadBootstrap();
  });
</script>

<svelte:head>
  <title>LorePia — 서재</title>
</svelte:head>

<div class="screen" use:minimizeDockOnScroll>
  <LargeTitleHeader title="서재" />

  {#if loading}
    <p class="status" role="status">제품 코어에 연결하는 중입니다.</p>
  {:else if errorMessage}
    <div class="status error" role="alert">
      <span>{errorMessage}</span>
      <button type="button" onclick={loadBootstrap}>다시 시도</button>
    </div>
  {/if}

  {#if characters.length === 0}
    <section class="empty">
      <h2>첫 캐릭터를 데려오세요</h2>
      <p>카드 파일을 가져오면 이곳에 이야기가 쌓입니다.</p>
      <a class="cta" href="/import">캐릭터 가져오기</a>
    </section>
  {:else}
    <div class="searchrow">
      <div class="search">
        <svg
          viewBox="0 0 24 24"
          width="16"
          height="16"
          fill="none"
          stroke="currentColor"
          stroke-width="2"
          stroke-linecap="round"
          aria-hidden="true"
        >
          <circle cx="11" cy="11" r="7" />
          <path d="m20 20-3.5-3.5" />
        </svg>
        <input
          type="search"
          bind:value={librarySearch.query}
          bind:this={searchInput}
          onfocus={() => (searchFocused = true)}
          onblur={() => (searchFocused = false)}
          placeholder="검색"
          aria-label="캐릭터 검색"
          aria-controls="library-list"
        />
        {#if librarySearch.query !== ""}
          <button
            class="clear"
            type="button"
            onclick={clearQuery}
            aria-label="검색어 지우기"
          >
            <svg
              viewBox="0 0 24 24"
              width="12"
              height="12"
              fill="none"
              stroke="currentColor"
              stroke-width="3"
              stroke-linecap="round"
              aria-hidden="true"
            >
              <path d="M6 6l12 12M18 6 6 18" />
            </svg>
          </button>
        {/if}
      </div>
      {#if showCancel}
        <!-- pointerdown runs before the input's blur, so the tap still lands
             when losing focus is what hides this button. -->
        <button
          class="cancel"
          type="button"
          onpointerdown={(event) => {
            event.preventDefault();
            cancelSearch();
          }}
        >
          취소
        </button>
      {/if}
    </div>
  {/if}

  {#if characters.length > 0 && matches.length === 0}
    <p class="noresult" role="status">일치하는 캐릭터가 없습니다.</p>
  {:else if characters.length > 0}
    <ol class="list" id="library-list">
      {#each matches as character (character.id)}
        <li>
          <a class="row" href="/chat">
            <Avatar initial={character.initial} size={48} />
            <span class="body">
              <span class="line">
                <span class="name">{character.name}</span>
                <span class="meta">
                  <time class="when" datetime={character.lastAt.toISOString()}
                    >{formatMessageTime(character.lastAt)}</time
                  >
                  <svg
                    class="chev"
                    viewBox="0 0 24 24"
                    width="14"
                    height="14"
                    fill="none"
                    stroke="currentColor"
                    stroke-width="2.2"
                    stroke-linecap="round"
                    stroke-linejoin="round"
                    aria-hidden="true"
                  >
                    <path d="m9 18 6-6-6-6" />
                  </svg>
                </span>
              </span>
              <span class="preview">{character.lastMessage}</span>
            </span>
          </a>
        </li>
      {/each}
    </ol>
  {/if}

  {#if bootstrap && characters.length > 0}
    <p class="core">코어 v{bootstrap.coreVersion} · 기기 로컬 저장</p>
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

  .status {
    margin: 0;
    padding: var(--sp-2) var(--sp-4);
    box-sizing: border-box;
    font-size: var(--fs-label);
    color: var(--text-mid);
  }

  .status.error {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--sp-3);
    margin: var(--sp-2) var(--sp-4) 0;
    padding: var(--sp-3) var(--sp-4);
    background: var(--surface-card);
    border-radius: var(--r-card);
    box-shadow: var(--shadow-card);
    color: var(--text-strong);
  }

  .status.error button {
    min-height: 32px;
    padding: 0 var(--sp-3);
    border: 0.5px solid var(--hairline);
    border-radius: var(--r-pill);
    background: transparent;
    color: var(--text-strong);
    font-family: var(--font-ui);
    font-size: var(--fs-label);
    cursor: pointer;
  }

  /* On phones search lives at the bottom next to the dock (iOS 26), so the
     top field only exists on the wide layout. */
  .searchrow {
    flex-shrink: 0;
    display: none;
    align-items: center;
    gap: var(--sp-2);
    margin: 0 var(--sp-4);
  }

  .search {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: center;
    gap: var(--sp-2);
    box-sizing: border-box;
    height: 36px;
    padding: 0 var(--sp-3);
    border-radius: var(--r-block);
    background: var(--surface-bubble);
    color: var(--text-faint);
  }

  .search:focus-within {
    box-shadow: inset 0 0 0 1.5px var(--tint);
  }

  .search input {
    flex: 1;
    min-width: 0;
    border: 0;
    padding: 0;
    background: transparent;
    color: var(--text-strong);
    font-family: var(--font-ui);
    font-size: var(--fs-chat);
    caret-color: var(--cursor-color);
  }

  .search input:focus {
    outline: none;
  }

  .search input::placeholder {
    color: var(--text-faint);
  }

  /* The custom clear button below replaces the platform decoration. */
  .search input::-webkit-search-cancel-button {
    -webkit-appearance: none;
    appearance: none;
  }

  .clear {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    flex-shrink: 0;
    width: 18px;
    height: 18px;
    padding: 0;
    border: 0;
    border-radius: var(--r-pill);
    background: var(--text-faint);
    color: var(--surface-page);
    cursor: pointer;
  }

  .cancel {
    flex-shrink: 0;
    min-height: 36px;
    padding: 0;
    border: 0;
    background: transparent;
    color: var(--tint);
    font-family: var(--font-ui);
    font-size: var(--fs-chat);
    cursor: pointer;
    animation: lp-cancel-in var(--dur-base) var(--ease-out);
  }

  @keyframes lp-cancel-in {
    from {
      opacity: 0;
      transform: translateX(8px);
    }
  }

  .noresult {
    margin: var(--sp-6) 0 0;
    padding: 0 var(--sp-4);
    font-size: var(--fs-ui);
    color: var(--text-mid);
    text-align: center;
  }

  .list {
    margin: var(--sp-2) 0 0;
    padding: 0;
    list-style: none;
    display: flex;
    flex-direction: column;
    gap: 0;
  }

  .list li {
    position: relative;
    animation: lp-pop var(--dur-page) var(--ease-spring) backwards;
  }

  .list li:not(:last-child)::after {
    content: "";
    position: absolute;
    right: 0;
    bottom: 0;
    left: calc(var(--sp-4) + 48px + var(--sp-3));
    height: 0.5px;
    background: var(--hairline);
    pointer-events: none;
  }

  .list li:nth-child(1) {
    animation-delay: 40ms;
  }
  .list li:nth-child(2) {
    animation-delay: 90ms;
  }
  .list li:nth-child(3) {
    animation-delay: 140ms;
  }
  .list li:nth-child(4) {
    animation-delay: 190ms;
  }
  .list li:nth-child(5) {
    animation-delay: 240ms;
  }
  .list li:nth-child(n + 6) {
    animation-delay: 290ms;
  }

  .row {
    display: flex;
    align-items: center;
    gap: var(--sp-3);
    width: 100%;
    min-height: 72px;
    padding: var(--sp-3) var(--sp-4);
    box-sizing: border-box;
    text-decoration: none;
    min-width: 0;
    transition: background var(--dur-fast) var(--ease-out);
  }

  /* Rows run edge to edge, so the press state is what marks the tap target. */
  .row:active {
    background: var(--surface-bubble);
  }

  @media (hover: hover) {
    .row:hover {
      background: var(--surface-bubble);
    }
  }

  .body {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .line {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--sp-2);
  }

  .name {
    font-size: var(--fs-chat);
    font-weight: 500;
    letter-spacing: -0.02em;
    color: var(--text-strong);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .meta {
    display: inline-flex;
    align-items: center;
    gap: 1px;
    flex-shrink: 0;
    color: var(--text-faint);
  }

  .when {
    font-size: var(--fs-caption);
  }

  /* Two-line preview with a trailing chevron on the timestamp: the Messages
     row anatomy. */
  .preview {
    font-size: 13px;
    line-height: 1.45;
    color: var(--text-mid);
    display: -webkit-box;
    -webkit-box-orient: vertical;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    overflow: hidden;
  }

  .empty {
    position: relative;
    flex: 1;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: var(--sp-2);
    padding: var(--sp-6);
    text-align: center;
    animation: lp-pop var(--dur-page) var(--ease-spring) backwards;
  }

  .empty::before {
    content: "";
    position: absolute;
    width: 280px;
    height: 280px;
    border-radius: var(--r-pill);
    background: radial-gradient(closest-side, var(--tint-soft), transparent);
    pointer-events: none;
  }

  .empty > :global(*) {
    position: relative;
    z-index: 1;
  }

  .empty h2 {
    margin: 0;
    font-size: 18px;
    font-weight: 500;
    color: var(--text-strong);
  }

  .empty p {
    margin: 0;
    font-size: var(--fs-ui);
    color: var(--text-mid);
  }

  .cta {
    margin-top: var(--sp-3);
    min-height: var(--size-touch);
    display: inline-flex;
    align-items: center;
    padding: 0 var(--sp-5);
    border-radius: var(--r-pill);
    background: var(--tint);
    color: #fff;
    font-size: var(--fs-ui);
    font-weight: 600;
    text-decoration: none;
    box-shadow: var(--shadow-card);
    transition: transform var(--dur-fast) var(--ease-out);
  }

  .cta:active {
    transform: scale(0.96);
  }

  .core {
    margin: auto 0 0;
    padding: var(--sp-3) var(--sp-4)
      calc(var(--sp-3) + var(--safe-bottom));
    font-size: var(--fs-caption);
    color: var(--text-faint);
    text-align: center;
  }

  @media (min-width: 700px) {
    /* Rows carry the page gutter in their own padding, so this column is
       wider by exactly that much and avatars stay flush with the title. */
    .status,
    .list {
      width: min(100%, calc(680px + var(--sp-4) * 2));
      margin-inline: auto;
    }

    /* Card-shaped, so their edges land on the gutter like the title. */
    .status.error,
    .searchrow {
      width: min(100% - var(--sp-4) * 2, 680px);
      margin-inline: auto;
    }

    .searchrow {
      display: flex;
    }
  }
</style>
