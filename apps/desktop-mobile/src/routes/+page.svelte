<script lang="ts">
  import { goto } from "$app/navigation";
  import { onMount, tick } from "svelte";

  import "$lib/design/tokens.css";

  import { SAMPLE_CHARACTERS } from "$lib/characters/sample";
  import { librarySearch } from "$lib/characters/library-search.svelte";
  import { matchesQuery } from "$lib/characters/search";
  import { formatMessageTime } from "$lib/design/time-of-day";
  import Avatar from "$lib/ui/Avatar.svelte";
  import LargeTitleHeader from "$lib/ui/LargeTitleHeader.svelte";
  import { minimizeDockOnScroll } from "$lib/ui/dock-chrome.svelte";
  import {
    completeNativeBack,
    prepareNativeBack,
  } from "$lib/ui/native-back";
  import {
    publicBootstrapError,
    requestProductBootstrap,
    type ProductBootstrap,
  } from "$lib/product-bootstrap";

  let bootstrap = $state<ProductBootstrap | null>(null);
  let errorMessage = $state<string | null>(null);
  let loading = $state(true);

  const characters = SAMPLE_CHARACTERS;

  /* Search is local to the library. On phones a trailing toolbar button
     becomes an integrated toolbar field; wide layouts keep their field
     visible below the title. */
  let searchFocused = $state(false);
  let mobileSearchInput = $state<HTMLInputElement | null>(null);
  let desktopSearchInput = $state<HTMLInputElement | null>(null);
  let narrowLayout = $state(true);

  const matches = $derived(
    characters.filter((character) =>
      matchesQuery(character, librarySearch.query),
    ),
  );

  // iOS keeps Cancel up while editing or while a query is in effect.
  const showCancel = $derived(
    librarySearch.open || searchFocused || librarySearch.query !== "",
  );

  async function openSearch(): Promise<void> {
    librarySearch.openSearch();
    await tick();
    (narrowLayout ? mobileSearchInput : desktopSearchInput)?.focus();
  }

  async function openChat(event: MouseEvent): Promise<void> {
    event.preventDefault();
    const nativeStatus = await prepareNativeBack();
    try {
      await goto("/chat", {
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

  async function clearQuery(): Promise<void> {
    librarySearch.query = "";
    await tick();
    (narrowLayout ? mobileSearchInput : desktopSearchInput)?.focus({
      preventScroll: true,
    });
  }

  function cancelSearch(): void {
    librarySearch.close();
    searchFocused = false;
    mobileSearchInput?.blur();
    desktopSearchInput?.blur();
  }

  function closeSearchOnEscape(event: KeyboardEvent): void {
    if (event.key === "Escape" && librarySearch.open) {
      cancelSearch();
    }
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
    const narrowQuery = window.matchMedia("(max-width: 699px)");
    const syncLayout = (): void => {
      narrowLayout = narrowQuery.matches;
    };

    syncLayout();
    narrowQuery.addEventListener("change", syncLayout);
    void loadBootstrap();

    return () => {
      narrowQuery.removeEventListener("change", syncLayout);
    };
  });
</script>

<svelte:head>
  <title>LorePia — 서재</title>
</svelte:head>

<svelte:window onkeydown={closeSearchOnEscape} />

<div class="screen" use:minimizeDockOnScroll>
  <LargeTitleHeader title="서재">
    {#snippet trailing()}
      {#if characters.length > 0}
        {#if librarySearch.open}
          <div
            class="integratedsearch"
            id="library-mobile-search"
            role="search"
          >
            <div class="integratedfield">
              <svg
                class="searchicon"
                viewBox="0 0 22 22"
                width="22"
                height="22"
                fill="none"
                stroke="currentColor"
                stroke-width="2"
                stroke-linecap="round"
                aria-hidden="true"
              >
                <circle cx="8.5" cy="8.5" r="7" />
                <path d="m13.5 13.5 7.5 7.5" />
              </svg>
              <input
                type="search"
                bind:value={librarySearch.query}
                bind:this={mobileSearchInput}
                onfocus={() => (searchFocused = true)}
                onblur={() => (searchFocused = false)}
                placeholder="캐릭터 검색"
                aria-label="캐릭터 검색"
                aria-controls="library-list"
              />
              {#if librarySearch.query !== ""}
                <button
                  class="clear lp-state-layer"
                  type="button"
                  onpointerdown={(event) => event.preventDefault()}
                  onclick={clearQuery}
                  aria-label="검색어 지우기"
                >
                  <svg
                    viewBox="0 0 24 24"
                    width="10"
                    height="10"
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
            <button
              class="integratedcancel lp-state-layer"
              type="button"
              aria-label="검색 닫기"
              onclick={cancelSearch}
            >
              <svg
                viewBox="0 0 22 22"
                width="22"
                height="22"
                fill="none"
                stroke="currentColor"
                stroke-width="2"
                stroke-linecap="round"
                aria-hidden="true"
              >
                <path d="M3.5 3.5l15 15M18.5 3.5l-15 15" />
              </svg>
            </button>
          </div>
        {:else}
          <button
            class="searchtrigger lp-state-layer"
            type="button"
            aria-label="캐릭터 검색"
            aria-controls="library-mobile-search"
            aria-expanded="false"
            onclick={openSearch}
          >
            <svg
              viewBox="0 0 22 22"
              width="22"
              height="22"
              fill="none"
              stroke="currentColor"
              stroke-width="2"
              stroke-linecap="round"
              aria-hidden="true"
            >
              <circle cx="8.5" cy="8.5" r="7" />
              <path d="m13.5 13.5 7.5 7.5" />
            </svg>
          </button>
        {/if}
      {/if}
    {/snippet}
  </LargeTitleHeader>

  {#if characters.length > 0}
    <div class="desktopsearchrow" id="library-desktop-search" role="search">
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
          bind:this={desktopSearchInput}
          onfocus={() => {
            searchFocused = true;
            librarySearch.openSearch();
          }}
          onblur={() => (searchFocused = false)}
          placeholder="캐릭터 검색"
          aria-label="캐릭터 검색"
          aria-controls="library-list"
        />
        {#if librarySearch.query !== ""}
          <button
            class="clear lp-state-layer"
            type="button"
            onpointerdown={(event) => event.preventDefault()}
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
          class="cancel lp-state-layer"
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

  {#if loading}
    <p class="status" role="status">제품 코어에 연결하는 중입니다.</p>
  {:else if errorMessage}
    <div class="status error" role="alert">
      <span>{errorMessage}</span>
      <button class="lp-state-layer" type="button" onclick={loadBootstrap}
        >다시 시도</button
      >
    </div>
  {/if}

  {#if characters.length === 0}
    <section class="empty">
      <h2>첫 캐릭터를 데려오세요</h2>
      <p>카드 파일을 가져오면 이곳에 이야기가 쌓입니다.</p>
      <a class="cta" href="/import">캐릭터 가져오기</a>
    </section>
  {/if}

  {#if characters.length > 0 && matches.length === 0}
    <p class="noresult" role="status">일치하는 캐릭터가 없습니다.</p>
  {:else if characters.length > 0}
    <ol class="list" id="library-list">
      {#each matches as character (character.id)}
        <li>
          <a class="row" href="/chat" onclick={openChat}>
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

  /* iOS 26 integratedButton: a 44pt circular toolbar item, 16pt from the
     trailing edge. The material remains adaptive; its geometry is fixed. */
  .searchtrigger {
    position: relative;
    width: var(--size-touch);
    height: var(--size-touch);
    display: inline-flex;
    align-items: center;
    justify-content: center;
    padding: 0;
    border: 0;
    border-radius: var(--r-pill);
    background: transparent;
    color: var(--text-mid);
    cursor: pointer;
  }

  .searchtrigger::before {
    content: "";
    position: absolute;
    inset: 0;
    border: 0.5px solid var(--hairline);
    border-radius: var(--r-pill);
    background: var(--bar-bg);
    -webkit-backdrop-filter: blur(20px) saturate(1.6);
    backdrop-filter: blur(20px) saturate(1.6);
    box-shadow: var(--shadow-float);
  }

  .searchtrigger svg {
    position: relative;
  }

  /* UIKit 26 measured at runtime: the active platter is 16pt from both
     screen edges and 44pt high. Its search-bar item sits 4pt inside; the
     44pt close control follows the field with an 11pt gap. */
  .integratedsearch {
    width: calc(100vw - var(--sp-4) * 2);
    height: var(--size-touch);
    box-sizing: border-box;
    padding: 0 var(--sp-1);
    border: 0.5px solid var(--hairline);
    border-radius: 22px;
    display: flex;
    align-items: center;
    background: var(--bar-bg);
    -webkit-backdrop-filter: blur(20px) saturate(1.6);
    backdrop-filter: blur(20px) saturate(1.6);
    box-shadow: var(--shadow-float);
    color: var(--text-mid);
  }

  .integratedfield {
    flex: 1;
    min-width: 0;
    height: var(--size-touch);
    box-sizing: border-box;
    display: flex;
    align-items: center;
    gap: 6px;
    padding-left: var(--sp-3);
  }

  .integratedfield .searchicon {
    flex-shrink: 0;
  }

  .integratedfield input {
    flex: 1;
    min-width: 0;
    height: 40px;
    border: 0;
    padding: 0;
    background: transparent;
    color: var(--text-strong);
    font-family: var(--font-ui);
    font-size: var(--fs-bartitle);
    line-height: 22px;
    caret-color: var(--cursor-color);
  }

  .integratedfield input:focus {
    outline: none;
  }

  .integratedfield input::placeholder {
    color: var(--text-faint);
  }

  .integratedfield input::-webkit-search-cancel-button {
    -webkit-appearance: none;
    appearance: none;
  }

  .integratedcancel {
    flex: 0 0 var(--size-touch);
    width: var(--size-touch);
    height: var(--size-touch);
    margin-left: 11px;
    padding: 0;
    border: 0;
    border-radius: 22px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    background: transparent;
    color: var(--text-strong);
    cursor: pointer;
  }

  .desktopsearchrow {
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

  .integratedfield .clear {
    position: relative;
    width: 36px;
    height: 36px;
    background: transparent;
  }

  .integratedfield .clear::before {
    content: "";
    position: absolute;
    width: 18px;
    height: 18px;
    border-radius: var(--r-pill);
    background: var(--text-faint);
  }

  .integratedfield .clear svg {
    position: relative;
  }

  .cancel {
    flex-shrink: 0;
    min-height: var(--size-touch);
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
    .searchtrigger,
    .integratedsearch {
      display: none;
    }

    /* Rows carry the page gutter in their own padding, so this column is
       wider by exactly that much and avatars stay flush with the title. */
    .status,
    .list {
      width: min(100%, calc(680px + var(--sp-4) * 2));
      margin-inline: auto;
    }

    /* Card-shaped, so their edges land on the gutter like the title. */
    .status.error,
    .desktopsearchrow {
      width: min(100% - var(--sp-4) * 2, 680px);
      margin-inline: auto;
    }

    .desktopsearchrow {
      display: flex;
    }
  }
</style>
