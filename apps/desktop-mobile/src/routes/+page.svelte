<script lang="ts">
  import { onMount } from "svelte";

  import {
    publicBootstrapError,
    requestProductBootstrap,
    type ProductBootstrap,
  } from "$lib/product-bootstrap";

  let bootstrap = $state<ProductBootstrap | null>(null);
  let errorMessage = $state<string | null>(null);
  let loading = $state(true);

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
  <title>LorePia</title>
</svelte:head>

<main>
  <h1>LorePia</h1>

  {#if loading}
    <p role="status">제품 코어에 연결하는 중입니다.</p>
  {:else if errorMessage}
    <p role="alert">{errorMessage}</p>
    <button type="button" onclick={loadBootstrap}>다시 시도</button>
  {:else if bootstrap}
    <p role="status">제품 코어에 연결되었습니다.</p>
    <dl>
      <dt>부트스트랩 계약</dt>
      <dd>v{bootstrap.contractVersion}</dd>
      <dt>코어 버전</dt>
      <dd>{bootstrap.coreVersion}</dd>
      <dt>데이터 경계</dt>
      <dd>기기 로컬, 사용자가 선택한 LLM 요청만 외부 전송</dd>
      <dt>가져온 실행 콘텐츠</dt>
      <dd>M-1 검증 완료 전 비활성</dd>
    </dl>
  {/if}
</main>
