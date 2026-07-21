<script lang="ts">
  import { formatMessageTime } from "$lib/design/time-of-day";
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

  interface ThreadEntry {
    message: ChatMessage;
    time: string;
    showHeader: boolean;
    showTime: boolean;
    isTail: boolean;
  }

  const entries: ThreadEntry[] = $derived(
    messages.map((message, index) => {
      const prev = messages[index - 1];
      const next = messages[index + 1];
      const time = formatMessageTime(message.sentAt);
      const sameRunNext = next !== undefined && next.role === message.role;
      return {
        message,
        time,
        showHeader: prev === undefined || prev.role !== message.role,
        showTime:
          message.streaming !== true &&
          !(sameRunNext && formatMessageTime(next.sentAt) === time),
        isTail: !sameRunNext,
      };
    }),
  );
</script>

{#if mode === "chat"}
  <ol class="thread chat">
    {#each entries as entry (entry.message.id)}
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
              </div>
              {#if entry.showTime}
                <time class="stamp" datetime={entry.message.sentAt.toISOString()}
                  >{entry.time}</time
                >
              {/if}
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
            {#if entry.showTime}
              <time class="stamp" datetime={entry.message.sentAt.toISOString()}
                >{entry.time}</time
              >
            {/if}
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
      <li class="passage" class:mine={entry.message.role === "user"}>
        {#if entry.showHeader}
          <span class="speaker"
            >{entry.message.role === "character" ? characterName : "나"}</span
          >
        {/if}
        {#if entry.message.role === "character"}
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
        {:else}
          <div class="note">
            <p>{entry.message.text}</p>
          </div>
        {/if}
        {#if entry.showTime}
          <time class="stamp" datetime={entry.message.sentAt.toISOString()}
            >{entry.time}</time
          >
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

  .chat .row.grouped {
    margin-top: calc(var(--sp-1) - var(--sp-4));
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
    background: var(--surface-bubble);
    color: var(--text-strong);
    font-family: var(--font-ui);
    font-size: var(--fs-label);
    font-weight: 500;
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

  .bubble {
    background: var(--surface-bubble);
    border-radius: var(--r-bubble);
    padding: 10px 14px;
    min-width: 0;
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
    background: var(--invert-surface);
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
    color: var(--invert-text);
  }

  .bubble p.voice {
    font-family: var(--font-voice);
  }

  .narration {
    font-style: italic;
    color: var(--text-mid);
  }

  .stamp {
    font-family: var(--font-ui);
    font-size: var(--fs-caption);
    color: var(--text-faint);
    flex-shrink: 0;
    white-space: nowrap;
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
    background: var(--invert-surface);
    border-radius: var(--r-block);
    padding: var(--sp-3) var(--sp-4);
  }

  .note p {
    margin: 0;
    font-family: var(--font-ui);
    font-size: var(--fs-chat);
    line-height: var(--lh-chat);
    color: var(--invert-text);
    overflow-wrap: anywhere;
  }

  .story .stamp {
    align-self: flex-start;
  }

  .passage.mine .stamp {
    align-self: flex-end;
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
