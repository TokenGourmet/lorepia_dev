<script lang="ts">
  import { onMount } from "svelte";

  import "$lib/design/tokens.css";

  import { SAMPLE_CHARACTERS } from "$lib/characters/sample";
  import { formatMessageTime } from "$lib/design/time-of-day";
  import Avatar from "$lib/ui/Avatar.svelte";
  import {
    publicBootstrapError,
    requestProductBootstrap,
    type ProductBootstrap,
  } from "$lib/product-bootstrap";

  let bootstrap = $state<ProductBootstrap | null>(null);
  let errorMessage = $state<string | null>(null);
  let loading = $state(true);

  const characters = SAMPLE_CHARACTERS;

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

<div class="screen">
  <header class="top">
    <h1>서재</h1>
    <nav aria-label="주요 메뉴">
      <a href="/import" aria-label="캐릭터 가져오기">
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
          <path d="M12 5v14" />
          <path d="M5 12h14" />
        </svg>
      </a>
      <a href="/settings" aria-label="설정">
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
          <circle cx="12" cy="12" r="3" />
          <path
            d="M19.4 15a1.7 1.7 0 0 0 .34 1.87l.06.06a2 2 0 1 1-2.83 2.83l-.06-.06a1.7 1.7 0 0 0-1.87-.34 1.7 1.7 0 0 0-1.03 1.56V21a2 2 0 1 1-4 0v-.09a1.7 1.7 0 0 0-1.11-1.56 1.7 1.7 0 0 0-1.87.34l-.06.06a2 2 0 1 1-2.83-2.83l.06-.06a1.7 1.7 0 0 0 .34-1.87 1.7 1.7 0 0 0-1.56-1.03H3a2 2 0 1 1 0-4h.09A1.7 1.7 0 0 0 4.65 8.9a1.7 1.7 0 0 0-.34-1.87l-.06-.06a2 2 0 1 1 2.83-2.83l.06.06a1.7 1.7 0 0 0 1.87.34h.09A1.7 1.7 0 0 0 10.13 3V3a2 2 0 1 1 4 0v.09a1.7 1.7 0 0 0 1.03 1.56 1.7 1.7 0 0 0 1.87-.34l.06-.06a2 2 0 1 1 2.83 2.83l-.06.06a1.7 1.7 0 0 0-.34 1.87v.09a1.7 1.7 0 0 0 1.56 1.03H21a2 2 0 1 1 0 4h-.09a1.7 1.7 0 0 0-1.51 1.03Z"
          />
        </svg>
      </a>
    </nav>
  </header>

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
    <ol class="list">
      {#each characters as character (character.id)}
        <li>
          <a class="row" href="/chat">
            <Avatar initial={character.initial} size={48} />
            <span class="body">
              <span class="line">
                <span class="name">{character.name}</span>
                <time class="when" datetime={character.lastAt.toISOString()}
                  >{formatMessageTime(character.lastAt)}</time
                >
              </span>
              <span class="preview">{character.lastMessage}</span>
            </span>
          </a>
          <a
            class="info"
            href={`/character/${character.id}`}
            aria-label={`${character.name} 정보`}
          >
            <svg
              viewBox="0 0 24 24"
              width="18"
              height="18"
              fill="none"
              stroke="currentColor"
              stroke-width="2"
              stroke-linecap="round"
              stroke-linejoin="round"
              aria-hidden="true"
            >
              <circle cx="12" cy="12" r="9" />
              <path d="M12 16v-5" />
              <path d="M12 8h.01" />
            </svg>
          </a>
        </li>
      {/each}
    </ol>
    {#if bootstrap}
      <p class="core">코어 v{bootstrap.coreVersion} · 기기 로컬 저장</p>
    {/if}
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
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: var(--sp-3) var(--sp-4);
    padding-top: calc(var(--sp-3) + var(--safe-top));
  }

  .top h1 {
    margin: 0;
    font-size: 20px;
    font-weight: 500;
    color: var(--text-strong);
  }

  nav {
    display: flex;
    gap: var(--sp-1);
  }

  nav a {
    width: var(--size-touch);
    height: var(--size-touch);
    display: inline-flex;
    align-items: center;
    justify-content: center;
    border-radius: var(--r-pill);
    color: var(--text-mid);
    transition: background var(--dur-fast) var(--ease-out);
  }

  nav a:active {
    background: var(--surface-bubble);
  }

  .status {
    margin: 0;
    padding: var(--sp-2) var(--sp-4);
    font-size: var(--fs-label);
    color: var(--text-mid);
  }

  .status.error {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--sp-3);
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

  .list {
    margin: 0;
    padding: 0;
    list-style: none;
  }

  .list li {
    display: flex;
    align-items: center;
    border-bottom: 0.5px solid var(--hairline);
  }

  .row {
    flex: 1;
    display: flex;
    align-items: center;
    gap: var(--sp-3);
    min-height: 72px;
    padding: var(--sp-3) 0 var(--sp-3) var(--sp-4);
    text-decoration: none;
    min-width: 0;
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
    align-items: baseline;
    justify-content: space-between;
    gap: var(--sp-2);
  }

  .name {
    font-size: var(--fs-chat);
    font-weight: 500;
    color: var(--text-strong);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .when {
    font-size: var(--fs-caption);
    color: var(--text-faint);
    flex-shrink: 0;
  }

  .preview {
    font-size: 13px;
    line-height: 1.45;
    color: var(--text-mid);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .info {
    width: var(--size-touch);
    height: var(--size-touch);
    margin-right: var(--sp-2);
    display: inline-flex;
    align-items: center;
    justify-content: center;
    border-radius: var(--r-pill);
    color: var(--text-faint);
    flex-shrink: 0;
  }

  .empty {
    flex: 1;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: var(--sp-2);
    padding: var(--sp-6);
    text-align: center;
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
    background: var(--invert-surface);
    color: var(--invert-text);
    font-size: var(--fs-ui);
    font-weight: 500;
    text-decoration: none;
  }

  .core {
    margin: auto 0 0;
    padding: var(--sp-3) var(--sp-4)
      calc(var(--sp-3) + var(--safe-bottom));
    font-size: var(--fs-caption);
    color: var(--text-faint);
    text-align: center;
  }
</style>
