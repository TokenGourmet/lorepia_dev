<script lang="ts">
  import { formatThreadStamp } from "$lib/design/time-of-day";
  import type { ChatMessage, ThreadMode } from "./types";

  let {
    messages,
    mode = "chat",
    characterName,
    characterInitial,
  }: {
    messages: ChatMessage[];
    mode?: ThreadMode;
    characterName: string;
    characterInitial: string;
  } = $props();

  /* Time reads the iMessage way: no per-bubble stamps — a centered separator
     marks the thread start and any lull longer than this. */
  const SEPARATOR_GAP_MS = 60 * 60 * 1000;

  interface ThreadEntry {
    message: ChatMessage;
    separator: string | null;
    showHeader: boolean;
    isTail: boolean;
    isTyping: boolean;
  }

  function gapBefore(index: number): boolean {
    if (index === 0) return true;
    return (
      messages[index].sentAt.getTime() - messages[index - 1].sentAt.getTime() >
      SEPARATOR_GAP_MS
    );
  }

  const entries: ThreadEntry[] = $derived(
    messages.map((message, index) => {
      const prev = messages[index - 1];
      const next = messages[index + 1];
      const separated = gapBefore(index);
      // A separator breaks the run: the next message regroups under a fresh
      // header, and the previous one keeps its tail.
      const sameRunNext =
        next !== undefined && next.role === message.role && !gapBefore(index + 1);
      return {
        message,
        separator: separated ? formatThreadStamp(message.sentAt) : null,
        showHeader: separated || prev === undefined || prev.role !== message.role,
        isTail: !sameRunNext,
        isTyping:
          message.streaming === true &&
          message.text.length === 0 &&
          (message.streamingChunks ?? []).every((chunk) => chunk.length === 0),
      };
    }),
  );
</script>

{#if mode === "chat"}
  <ol class="thread chat">
    {#each entries as entry (entry.message.id)}
      {#if entry.separator}
        <li class="sep">
          <time datetime={entry.message.sentAt.toISOString()}
            >{entry.separator}</time
          >
        </li>
      {/if}
      {#if entry.message.role === "character"}
        <li
          class="row character"
          class:tail={entry.isTail}
          class:grouped={!entry.showHeader}
        >
          {#if entry.showHeader}
            <span class="avatar" aria-hidden="true">{characterInitial}</span>
          {:else}
            <span class="avatar-spacer" aria-hidden="true"></span>
          {/if}
          <div class="stack">
            {#if entry.showHeader}
              <span class="name">{characterName}</span>
            {/if}
            <div class="bubble-line">
              <div class="bubble">
                {#if entry.isTyping}
                  <span class="typing" role="status" aria-label="응답 작성 중"
                    ><i></i><i></i><i></i></span
                  >
                {:else}
                <p class="voice">
                  {#if entry.message.narration}
                    <span class="narration">{entry.message.narration}</span>
                    {" "}
                  {/if}{entry.message.text}{#each entry.message.streamingChunks ?? [] as chunk}{chunk}{/each}{#if entry.message.streaming}<span
                      class="cursor"
                      aria-hidden="true"
                    ></span>{/if}{#if !entry.message.streaming && entry.message.deliveryState === "partial"}<span
                      class="delivery-state"
                      aria-label="응답이 중단됨"
                    >중단됨</span>{:else if !entry.message.streaming && entry.message.deliveryState === "failed"}<span
                      class="delivery-state failed"
                      aria-label="응답 실패"
                    >실패</span>{/if}
                </p>
                {/if}
              </div>
            </div>
          </div>
        </li>
      {:else}
        <li
          class="row user"
          class:tail={entry.isTail}
          class:grouped={!entry.showHeader}
        >
          <div class="bubble-line mine">
            <div class="bubble inverted">
              <p>{entry.message.text}</p>
            </div>
          </div>
        </li>
      {/if}
    {/each}
  </ol>
{:else}
  <ol class="thread story">
    {#each entries as entry (entry.message.id)}
      {#if entry.separator}
        <li class="sep">
          <time datetime={entry.message.sentAt.toISOString()}
            >{entry.separator}</time
          >
        </li>
      {/if}
      <li class="passage" class:mine={entry.message.role === "user"}>
        {#if entry.showHeader}
          <span class="speaker"
            >{entry.message.role === "character" ? characterName : "나"}</span
          >
        {/if}
        {#if entry.message.role === "character"}
          {#if entry.isTyping}
            <span class="typing" role="status" aria-label="응답 작성 중"
              ><i></i><i></i><i></i></span
            >
          {:else}
          <p class="voice prose">
            {#if entry.message.narration}
              <span class="narration">{entry.message.narration}</span>
              {" "}
            {/if}{entry.message.text}{#each entry.message.streamingChunks ?? [] as chunk}{chunk}{/each}{#if entry.message.streaming}<span
                class="cursor"
                aria-hidden="true"
              ></span>{/if}{#if !entry.message.streaming && entry.message.deliveryState === "partial"}<span
                class="delivery-state"
                aria-label="응답이 중단됨"
              >중단됨</span>{:else if !entry.message.streaming && entry.message.deliveryState === "failed"}<span
                class="delivery-state failed"
                aria-label="응답 실패"
              >실패</span>{/if}
          </p>
          {/if}
        {:else}
          <div class="note">
            <p>{entry.message.text}</p>
          </div>
        {/if}
      </li>
    {/each}
  </ol>
{/if}

<style>
  .thread {
    margin: 0;
    padding: var(--sp-4);
    list-style: none;
    display: flex;
    flex-direction: column;
    background: var(--surface-page);
  }

  .chat {
    gap: var(--sp-4);
  }

  @media (min-width: 700px) {
    .thread.chat {
      padding-inline: max(var(--sp-4), calc((100% - 760px) / 2));
    }
  }

  .chat .row.grouped {
    margin-top: calc(var(--sp-1) - var(--sp-4));
  }

  /* Centered thread separator, the iMessage idiom: the only place time
     appears in the transcript. */
  .sep {
    align-self: center;
    padding-top: var(--sp-2);
    font-family: var(--font-ui);
    font-size: var(--fs-caption);
    font-weight: 500;
    color: var(--text-faint);
  }

  .sep:first-child {
    padding-top: 0;
  }

  .row {
    display: flex;
  }

  .row.character {
    justify-content: flex-start;
    gap: var(--sp-2);
  }

  .row.user {
    justify-content: flex-end;
  }

  .avatar {
    width: var(--size-avatar);
    height: var(--size-avatar);
    border-radius: var(--r-pill);
    background: var(--tint-soft);
    color: var(--tint);
    font-family: var(--font-ui);
    font-size: var(--fs-label);
    font-weight: 600;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    flex-shrink: 0;
  }

  .avatar-spacer {
    width: var(--size-avatar);
    flex-shrink: 0;
  }

  .stack {
    display: flex;
    flex-direction: column;
    gap: var(--sp-1);
    max-width: var(--bubble-max);
    min-width: 0;
  }

  .name {
    font-family: var(--font-ui);
    font-size: var(--fs-label);
    color: var(--text-mid);
    padding-left: var(--sp-1);
  }

  .bubble-line {
    display: flex;
    align-items: flex-end;
    gap: 6px;
    min-width: 0;
  }

  .bubble-line.mine {
    max-width: var(--bubble-max);
    justify-content: flex-end;
  }

  /* Received bubbles sit flat in tonal gray, the iMessage way — only the
     user's tinted bubble carries depth. */
  .bubble {
    background: var(--surface-bubble);
    border-radius: var(--r-bubble);
    padding: 10px 14px;
    min-width: 0;
  }

  .chat .row:last-child {
    animation: lp-pop var(--dur-slow) var(--ease-spring) backwards;
  }

  .chat .row.user:last-child {
    transform-origin: bottom right;
  }

  .chat .row.character:last-child {
    transform-origin: bottom left;
  }

  .delivery-state {
    display: inline-block;
    margin-left: var(--sp-2);
    color: var(--text-mid);
    font-family: var(--font-ui);
    font-size: var(--fs-caption);
  }

  .delivery-state.failed {
    color: var(--text-danger, var(--text-mid));
  }

  .row.character.tail .bubble {
    border-bottom-left-radius: var(--r-tail);
  }

  .row.user.tail .bubble {
    border-bottom-right-radius: var(--r-tail);
  }

  .bubble.inverted {
    background: linear-gradient(
      180deg,
      color-mix(in srgb, var(--tint) 82%, #ffffff) 0%,
      var(--tint) 100%
    );
  }

  .bubble p {
    margin: 0;
    font-family: var(--font-ui);
    font-size: var(--fs-chat);
    line-height: var(--lh-chat);
    color: var(--text-strong);
    overflow-wrap: anywhere;
  }

  .bubble.inverted p {
    color: #fff;
  }

  .bubble p.voice {
    font-family: var(--font-voice);
  }

  .narration {
    font-style: italic;
    color: var(--text-mid);
  }

  .story {
    gap: var(--sp-5);
    max-width: var(--measure-story);
    margin-inline: auto;
    width: 100%;
    box-sizing: border-box;
  }

  .passage {
    display: flex;
    flex-direction: column;
    gap: var(--sp-1);
  }

  .speaker {
    font-family: var(--font-ui);
    font-size: var(--fs-caption);
    letter-spacing: 0.4px;
    color: var(--text-mid);
  }

  .prose {
    margin: 0;
    font-family: var(--font-voice);
    font-size: var(--fs-story);
    line-height: var(--lh-story);
    color: var(--text-strong);
    overflow-wrap: anywhere;
  }

  .note {
    background: linear-gradient(
      180deg,
      color-mix(in srgb, var(--tint) 82%, #ffffff) 0%,
      var(--tint) 100%
    );
    border-radius: var(--r-block);
    padding: var(--sp-3) var(--sp-4);
    box-shadow: 0 1px 2px rgba(0, 0, 0, 0.05);
  }

  .note p {
    margin: 0;
    font-family: var(--font-ui);
    font-size: var(--fs-chat);
    line-height: var(--lh-chat);
    color: #fff;
    overflow-wrap: anywhere;
  }

  .typing {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    padding: 6px 2px;
  }

  .typing i {
    width: 7px;
    height: 7px;
    border-radius: var(--r-pill);
    background: var(--text-faint);
    animation: typing-bounce 1.2s var(--ease-out) infinite;
  }

  .typing i:nth-child(2) {
    animation-delay: 150ms;
  }

  .typing i:nth-child(3) {
    animation-delay: 300ms;
  }

  @keyframes typing-bounce {
    0%,
    60%,
    100% {
      transform: none;
      opacity: 0.5;
    }
    30% {
      transform: translateY(-4px);
      opacity: 1;
    }
  }

  .cursor {
    display: inline-block;
    width: 7px;
    height: 1em;
    margin-left: 2px;
    vertical-align: -0.15em;
    background: var(--cursor-color);
    animation: blink 1.1s steps(2, start) infinite;
  }

  @keyframes blink {
    to {
      visibility: hidden;
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .cursor {
      animation: none;
    }
  }
</style>
