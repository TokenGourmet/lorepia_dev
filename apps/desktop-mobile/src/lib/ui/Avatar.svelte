<script lang="ts">
  let {
    initial,
    size = 36,
    inverted = false,
  }: {
    initial: string;
    size?: number;
    inverted?: boolean;
  } = $props();
</script>

<span
  class="avatar"
  class:inverted
  style:width="{size}px"
  style:height="{size}px"
  style:font-size="{Math.round(size / 3)}px"
  aria-hidden="true">{initial}</span
>

<style>
  .avatar {
    border-radius: var(--r-pill);
    background: var(--surface-bubble);
    color: var(--text-strong);
    font-family: var(--font-ui);
    font-weight: 500;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    flex-shrink: 0;
  }

  .avatar.inverted {
    background: var(--invert-surface);
    color: var(--invert-text);
  }
</style>
