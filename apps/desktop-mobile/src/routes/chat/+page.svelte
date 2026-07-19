<script lang="ts">
  import { onMount } from "svelte";
  import { goto } from "$app/navigation";

  import "$lib/design/tokens.css";

  import Composer from "$lib/chat/Composer.svelte";
  import MessageThread from "$lib/chat/MessageThread.svelte";
  import type { ChatMessage, ThreadMode } from "$lib/chat/types";
  import { keyboardInset } from "$lib/design/keyboard-inset.svelte";
  import Avatar from "$lib/ui/Avatar.svelte";
  import { horizontalSwipe, type SwipeCommit } from "$lib/ui/horizontal-swipe";

  const characterName = "세라핀";
  const characterInitial = "세";

  let mode = $state<ThreadMode>("chat");
  let scrollRegion = $state<HTMLDivElement | null>(null);
  let panelElement = $state<HTMLElement | null>(null);

  let panelOpen = $state(false);
  let panelShift = $state(0);
  let backDrag = $state(0);
  let dragging = $state(false);

  let messages = $state<ChatMessage[]>([
    {
      id: "m1",
      role: "character",
      narration: "낡은 책장 사이로 촛불이 흔들린다. 세라핀은 읽던 책을 덮고 천천히 고개를 든다.",
      text: "“이 시간에 서고를 찾는 손님은 오랜만이네요. 찾는 책이 있나요, 아니면… 잠이 오지 않는 밤인가요?”",
      sentAt: new Date(2026, 6, 18, 23, 42),
    },
    {
      id: "m2",
      role: "user",
      text: "잠이 안 와서. 아무 책이나 추천해줄래?",
      sentAt: new Date(2026, 6, 18, 23, 43),
    },
    {
      id: "m3",
      role: "character",
      narration: "그녀는 사다리를 밀며 높은 선반으로 손을 뻗는다.",
      text: "“그렇다면 이야기가 긴 책이 좋겠어요. 끝이 궁금해서 잠들지 못하게.”",
      sentAt: new Date(2026, 6, 18, 23, 44),
    },
    {
      id: "m4",
      role: "character",
      text: "“짧은 우화집도 함께 챙겨드릴게요. 어느 쪽이든, 오늘 밤은 혼자가 아니에요.”",
      sentAt: new Date(2026, 6, 18, 23, 44),
    },
  ]);

  function panelWidth(): number {
    return panelElement?.offsetWidth ?? 320;
  }

  function openPanel(): void {
    panelOpen = true;
    panelShift = panelWidth();
  }

  function closePanel(): void {
    panelOpen = false;
    panelShift = 0;
  }

  function clamp(value: number, min: number, max: number): number {
    return Math.min(Math.max(value, min), max);
  }

  function handleSwipeMove(dx: number): void {
    dragging = true;
    if (panelOpen) {
      panelShift = clamp(panelWidth() - dx, 0, panelWidth());
      return;
    }
    if (dx < 0) {
      backDrag = 0;
      panelShift = clamp(-dx, 0, panelWidth());
    } else {
      panelShift = 0;
      backDrag = dx;
    }
  }

  function handleSwipeEnd(commit: SwipeCommit): void {
    dragging = false;
    if (panelOpen) {
      if (commit === "right") {
        closePanel();
      } else {
        panelShift = panelWidth();
      }
      return;
    }
    if (panelShift > 0) {
      if (commit === "left") {
        openPanel();
      } else {
        panelShift = 0;
      }
      return;
    }
    if (backDrag > 0 && commit === "right") {
      void goto("/");
    }
    backDrag = 0;
  }

  function handleSend(text: string): void {
    messages = [
      ...messages,
      {
        id: `m${messages.length + 1}-${Date.now()}`,
        role: "user",
        text,
        sentAt: new Date(),
      },
    ];
  }

  onMount(() => keyboardInset.start());

  $effect(() => {
    void messages.length;
    void keyboardInset.value;
    const region = scrollRegion;
    if (region) {
      requestAnimationFrame(() => {
        region.scrollTop = region.scrollHeight;
      });
    }
  });
</script>

<svelte:head>
  <title>LorePia — 대화</title>
</svelte:head>

<div
  class="screen"
  class:animate={!dragging}
  style:transform={`translateX(${backDrag}px)`}
  use:horizontalSwipe={{ onMove: handleSwipeMove, onEnd: handleSwipeEnd }}
>
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
    <button class="identity" type="button" onclick={openPanel}>
      <Avatar initial={characterInitial} size={32} />
      <span class="titles">
        <span class="name">{characterName}</span>
        <span class="tagline">달빛 서고의 사서</span>
      </span>
    </button>
    <span class="header-spacer"></span>
  </header>

  <div class="scroll" bind:this={scrollRegion}>
    <MessageThread {messages} {mode} {characterName} {characterInitial} />
  </div>

  <div class="composer-slot" style:padding-bottom={`${keyboardInset.value}px`}>
    <Composer onSend={handleSend} />
  </div>

  <button
    class="scrim"
    class:animate={!dragging}
    type="button"
    aria-label="방 설정 닫기"
    aria-hidden={panelShift === 0}
    tabindex={panelShift === 0 ? -1 : 0}
    style:opacity={panelShift / panelWidth()}
    style:visibility={panelShift === 0 ? "hidden" : "visible"}
    onclick={closePanel}
  ></button>

  <aside
    class="panel"
    class:animate={!dragging}
    bind:this={panelElement}
    style:transform={`translateX(calc(100% - ${panelShift}px))`}
    aria-label="방 설정"
  >
    <div class="panel-hero">
      <Avatar initial={characterInitial} size={48} />
      <div>
        <p class="panel-name">{characterName}</p>
        <p class="panel-tagline">달빛 서고의 사서</p>
      </div>
    </div>

    <div class="panel-row">
      <span class="panel-label">표시 모드</span>
      <div class="segment" role="group" aria-label="표시 모드 선택">
        <button
          type="button"
          class:active={mode === "chat"}
          onclick={() => (mode = "chat")}>채팅</button
        >
        <button
          type="button"
          class:active={mode === "story"}
          onclick={() => (mode = "story")}>스토리</button
        >
      </div>
    </div>

    <a class="panel-link" href="/character/seraphine">캐릭터 정보 보기</a>
    <a class="panel-link" href="/settings">앱 설정</a>
  </aside>
</div>

<style>
  .screen {
    height: 100%;
    display: flex;
    flex-direction: column;
    background: var(--surface-page);
    font-family: var(--font-ui);
    position: relative;
    overflow: hidden;
    touch-action: pan-y;
  }

  .screen.animate {
    transition: transform var(--dur-base) var(--ease-out);
  }

  .top {
    display: flex;
    align-items: center;
    gap: var(--sp-2);
    min-height: calc(var(--size-touch) + var(--sp-2));
    padding: var(--sp-2) var(--sp-3);
    padding-top: calc(var(--sp-2) + var(--safe-top));
    background: var(--surface-header);
    border-bottom: 0.5px solid var(--hairline);
  }

  .back,
  .header-spacer {
    width: var(--size-touch);
    height: var(--size-touch);
    flex-shrink: 0;
  }

  .back {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    border-radius: var(--r-pill);
    color: var(--text-mid);
  }

  .identity {
    flex: 1;
    display: flex;
    align-items: center;
    justify-content: center;
    gap: var(--sp-3);
    min-width: 0;
    min-height: var(--size-touch);
    border: none;
    background: transparent;
    cursor: pointer;
    font-family: var(--font-ui);
    padding: 0;
  }

  .titles {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    min-width: 0;
  }

  .name {
    font-size: var(--fs-ui);
    font-weight: 500;
    color: var(--text-strong);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .tagline {
    font-size: var(--fs-caption);
    color: var(--text-mid);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .scroll {
    flex: 1;
    overflow-y: auto;
    overscroll-behavior: none;
    scroll-behavior: smooth;
  }

  .composer-slot {
    background: var(--surface-page);
    transition: padding-bottom var(--dur-base) var(--ease-out);
  }

  .scrim {
    position: absolute;
    inset: 0;
    border: none;
    padding: 0;
    background: rgba(0, 0, 0, 0.35);
    cursor: pointer;
  }

  .scrim.animate {
    transition:
      opacity var(--dur-base) var(--ease-out),
      visibility var(--dur-base) var(--ease-out);
  }

  .panel {
    position: absolute;
    top: 0;
    right: 0;
    bottom: 0;
    width: min(320px, 84vw);
    background: var(--surface-header);
    border-left: 0.5px solid var(--hairline);
    padding: calc(var(--sp-4) + var(--safe-top)) var(--sp-4)
      calc(var(--sp-4) + var(--safe-bottom));
    display: flex;
    flex-direction: column;
    gap: var(--sp-3);
    box-sizing: border-box;
  }

  .panel.animate {
    transition: transform var(--dur-base) var(--ease-out);
  }

  .panel-hero {
    display: flex;
    align-items: center;
    gap: var(--sp-3);
    padding-bottom: var(--sp-3);
    border-bottom: 0.5px solid var(--hairline);
  }

  .panel-name {
    margin: 0;
    font-size: var(--fs-chat);
    font-weight: 500;
    color: var(--text-strong);
  }

  .panel-tagline {
    margin: 0;
    font-size: var(--fs-label);
    color: var(--text-mid);
  }

  .panel-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--sp-3);
    min-height: var(--size-touch);
  }

  .panel-label {
    font-size: var(--fs-ui);
    color: var(--text-strong);
  }

  .segment {
    display: flex;
    border: 0.5px solid var(--hairline);
    border-radius: var(--r-pill);
    padding: 2px;
    gap: 2px;
  }

  .segment button {
    min-height: 32px;
    padding: 0 var(--sp-3);
    border: none;
    border-radius: var(--r-pill);
    background: transparent;
    color: var(--text-mid);
    font-family: var(--font-ui);
    font-size: var(--fs-label);
    cursor: pointer;
    transition:
      background var(--dur-fast) var(--ease-out),
      color var(--dur-fast) var(--ease-out);
  }

  .segment button.active {
    background: var(--invert-surface);
    color: var(--invert-text);
  }

  .panel-link {
    display: flex;
    align-items: center;
    min-height: var(--size-touch);
    font-size: var(--fs-ui);
    color: var(--text-strong);
    text-decoration: none;
    border-bottom: 0.5px solid var(--hairline);
  }

  @media (prefers-reduced-motion: reduce) {
    .screen.animate,
    .scrim.animate,
    .panel.animate,
    .composer-slot {
      transition: none;
    }

    .scroll {
      scroll-behavior: auto;
    }
  }
</style>
