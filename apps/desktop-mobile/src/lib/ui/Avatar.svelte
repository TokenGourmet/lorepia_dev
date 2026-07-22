<script lang="ts">
  let {
    initial,
    size = 36,
    inverted = false,
  }: {
    initial: string;
    size?: 32 | 36 | 48 | 72;
    inverted?: boolean;
  } = $props();
</script>

<span
  class="avatar"
  class:inverted
  class:size-32={size === 32}
  class:size-48={size === 48}
  class:size-72={size === 72}
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
    width: 36px;
    height: 36px;
    font-size: 12px;
  }

  .avatar.size-32 {
    width: 32px;
    height: 32px;
    font-size: 11px;
  }

  .avatar.size-48 {
    width: 48px;
    height: 48px;
    font-size: 16px;
  }

  .avatar.size-72 {
    width: 72px;
    height: 72px;
    font-size: 24px;
  }

  .avatar.inverted {
    background: var(--invert-surface);
    color: var(--invert-text);
  }
</style>
