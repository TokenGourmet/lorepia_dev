<script lang="ts">
  import { onMount } from "svelte";

  let { title }: { title: string } = $props();

  let barElement = $state<HTMLElement | null>(null);
  let largeTitleElement = $state<HTMLElement | null>(null);
  let collapsed = $state(false);

  /* The bar title takes over once the large title has scrolled fully behind
     the bar. Comparing rects keeps this correct under any safe-area inset. */
  function syncTitleCollapse(): void {
    if (!barElement || !largeTitleElement) return;
    collapsed =
      largeTitleElement.getBoundingClientRect().bottom <=
      barElement.getBoundingClientRect().bottom;
  }

  /* Render this as a direct child of the screen's scroll container: the
     header watches that element to drive the collapse. */
  onMount(() => {
    const scroller = barElement?.parentElement;
    if (!scroller) return;
    scroller.addEventListener("scroll", syncTitleCollapse, { passive: true });
    return () => {
      scroller.removeEventListener("scroll", syncTitleCollapse);
    };
  });
</script>

<header class="bar" class:collapsed bind:this={barElement}>
  <span class="bartitle" aria-hidden="true">{title}</span>
</header>

<h1 class="title" bind:this={largeTitleElement}>{title}</h1>

<style>
  .bar {
    position: sticky;
    top: 0;
    z-index: 5;
    display: flex;
    align-items: center;
    justify-content: center;
    /* The screen is a column flex container, so the bar and the large title
       must opt out of shrinking or overflowing content squashes them. */
    flex-shrink: 0;
    height: var(--size-touch);
    padding: var(--safe-top) var(--sp-4) 0;
    background: var(--bar-bg);
    -webkit-backdrop-filter: blur(20px) saturate(1.6);
    backdrop-filter: blur(20px) saturate(1.6);
  }

  /* Separator only once content sits under the bar, as on iOS. */
  .bar::after {
    content: "";
    position: absolute;
    right: 0;
    bottom: 0;
    left: 0;
    height: 0.5px;
    background: var(--hairline);
    opacity: 0;
    transition: opacity var(--dur-base) var(--ease-out);
  }

  .bar.collapsed::after {
    opacity: 1;
  }

  .bartitle {
    font-size: var(--fs-bartitle);
    font-weight: 600;
    letter-spacing: -0.02em;
    color: var(--text-strong);
    white-space: nowrap;
    opacity: 0;
    transform: translateY(10px);
    transition:
      opacity var(--dur-base) var(--ease-out),
      transform var(--dur-base) var(--ease-out);
  }

  .bar.collapsed .bartitle {
    opacity: 1;
    transform: none;
  }

  .title {
    flex-shrink: 0;
    margin: 0;
    padding: var(--sp-2) var(--sp-4) var(--sp-3);
    font-size: var(--fs-title);
    font-weight: 700;
    letter-spacing: -0.03em;
    color: var(--text-strong);
  }

  @media (min-width: 700px) {
    /* The sidebar layout has no centred-title convention to honour, so the
       bar title tracks the same column edge as the large title. */
    .bar,
    .title {
      padding-left: max(var(--sp-4), calc((100% - 680px) / 2));
    }

    .bar {
      justify-content: flex-start;
    }
  }
</style>
