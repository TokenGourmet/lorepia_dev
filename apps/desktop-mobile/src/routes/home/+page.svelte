<script lang="ts">
  import "$lib/design/tokens.css";

  import LargeTitleHeader from "$lib/ui/LargeTitleHeader.svelte";
</script>

<svelte:head>
  <title>LorePia — 홈</title>
</svelte:head>

<div class="screen">
  <LargeTitleHeader title="홈" />
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
</style>
