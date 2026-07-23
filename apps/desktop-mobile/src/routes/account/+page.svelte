<script lang="ts">
  import "$lib/design/tokens.css";

  import SettingsPane from "$lib/settings/SettingsPane.svelte";
  import LargeTitleHeader from "$lib/ui/LargeTitleHeader.svelte";
  import { minimizeDockOnScroll } from "$lib/ui/dock-chrome.svelte";
</script>

<svelte:head>
  <title>LorePia — 계정</title>
</svelte:head>

<!-- The "my" tab: account card up top, settings below — one pane serves
     both this tab and any future compact presentations. -->
<div class="screen" use:minimizeDockOnScroll>
  <LargeTitleHeader title="계정" />
  <SettingsPane />
</div>

<style>
  .screen {
    height: 100%;
    overflow-y: auto;
    overscroll-behavior: none;
    width: min(100%, 720px);
    margin: 0 auto;
    display: flex;
    flex-direction: column;
    background: var(--surface-page);
    font-family: var(--font-ui);
    box-sizing: border-box;
    padding-bottom: calc(var(--sp-5) + var(--safe-bottom));
  }
</style>
