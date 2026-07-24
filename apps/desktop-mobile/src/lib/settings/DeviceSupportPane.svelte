<script lang="ts">
  import { onMount } from "svelte";

  import "$lib/design/tokens.css";
  import {
    copySafetyArtifactJson,
    deliverSafetyArtifact,
    publicArtifactCopyError,
    publicArtifactDeliveryError,
  } from "$lib/system/artifact-delivery";
  import {
    publicNativeSupportError,
    requestAssetStoreStatus,
    requestProductSafetyContract,
    requestRedactedDiagnostics,
    type AssetStoreErrorCode,
    type AssetStoreStatus,
    type ProductSafetyContract,
    type SafetyArtifact,
  } from "$lib/system/native-support";
  import {
    storageClient,
    type StorageStatus,
  } from "$lib/storage/client";

  let storageStatus = $state<StorageStatus | null>(null);
  let assetStatus = $state<AssetStoreStatus | null>(null);
  let safetyContract = $state<ProductSafetyContract | null>(null);
  let statusBusy = $state(false);
  let statusError = $state<string | null>(null);

  let diagnosticArtifact = $state<SafetyArtifact | null>(null);
  let diagnosticBusy = $state(false);
  let diagnosticError = $state<string | null>(null);
  let deliveryBusy = $state(false);
  let deliveryError = $state<string | null>(null);
  let actionStatus = $state<string | null>(null);

  let requestGeneration = 0;
  let mounted = false;

  const storageNeedsReview = $derived(
    storageStatus?.available === true &&
      (!storageStatus.walMaintenance.schedulerStarted ||
        storageStatus.walMaintenance.thresholdExceeded === true ||
        storageStatus.walMaintenance.emergencyTruncateThresholdExceeded ===
          true ||
        storageStatus.walMaintenance.starvationObserved === true ||
        (storageStatus.walMaintenance.lastErrorAtMs !== null &&
          (storageStatus.walMaintenance.lastSuccessAtMs === null ||
            storageStatus.walMaintenance.lastErrorAtMs >
              storageStatus.walMaintenance.lastSuccessAtMs))),
  );
  const assetNeedsReview = $derived(
    assetStatus?.available === true &&
      assetStatus.stats !== null &&
      (BigInt(assetStatus.stats.missingCount) > 0n ||
        BigInt(assetStatus.stats.quarantinedCount) > 0n),
  );

  function formatCount(value: string): string {
    return BigInt(value).toLocaleString("ko-KR");
  }

  function formatBytes(value: string): string {
    const bytes = BigInt(value);
    const units = ["B", "KB", "MB", "GB", "TB"] as const;
    let unitIndex = 0;
    let divisor = 1n;
    while (
      unitIndex < units.length - 1 &&
      bytes >= divisor * 1024n
    ) {
      divisor *= 1024n;
      unitIndex += 1;
    }
    if (unitIndex === 0) return `${bytes.toLocaleString("ko-KR")} B`;
    const tenths = (bytes * 10n) / divisor;
    const whole = tenths / 10n;
    const decimal = tenths % 10n;
    return decimal === 0n
      ? `${whole.toLocaleString("ko-KR")} ${units[unitIndex]}`
      : `${whole.toLocaleString("ko-KR")}.${decimal} ${units[unitIndex]}`;
  }

  function assetErrorMessage(code: AssetStoreErrorCode | null): string {
    switch (code) {
      case "ASSET_PATH_UNAVAILABLE":
        return "기기 저장 경로를 사용할 수 없습니다.";
      case "ASSET_SCHEMA_INCOMPATIBLE":
        return "저장 형식이 현재 앱과 호환되지 않습니다.";
      case "ASSET_FILESYSTEM_UNSAFE":
        return "안전하지 않은 저장 경로가 감지되었습니다.";
      case "ASSET_STORE_UNAVAILABLE":
        return "카드·미디어 저장소를 열지 못했습니다.";
      case "ASSET_INTERNAL":
      case null:
        return "카드·미디어 저장소를 확인하지 못했습니다.";
    }
  }

  async function reloadStatuses(): Promise<void> {
    if (statusBusy) return;
    const generation = ++requestGeneration;
    statusBusy = true;
    statusError = null;
    const [storageResult, assetResult, safetyResult] =
      await Promise.allSettled([
        storageClient.getStorageStatus(),
        requestAssetStoreStatus(),
        requestProductSafetyContract(),
      ]);
    if (!mounted || generation !== requestGeneration) return;

    storageStatus =
      storageResult.status === "fulfilled" ? storageResult.value : null;
    assetStatus =
      assetResult.status === "fulfilled" ? assetResult.value : null;
    safetyContract =
      safetyResult.status === "fulfilled" ? safetyResult.value : null;
    if (
      storageResult.status === "rejected" ||
      assetResult.status === "rejected" ||
      safetyResult.status === "rejected"
    ) {
      statusError =
        "일부 기기 상태를 확인하지 못했습니다. 잠시 후 다시 확인해 주세요.";
    }
    statusBusy = false;
  }

  async function createDiagnostics(): Promise<void> {
    if (diagnosticBusy) return;
    diagnosticBusy = true;
    diagnosticError = null;
    deliveryError = null;
    actionStatus = null;
    try {
      diagnosticArtifact = await requestRedactedDiagnostics();
    } catch (error) {
      diagnosticArtifact = null;
      diagnosticError = publicNativeSupportError(error);
    } finally {
      diagnosticBusy = false;
    }
  }

  function isShareCancellation(error: unknown): boolean {
    return (
      typeof error === "object" &&
      error !== null &&
      "name" in error &&
      error.name === "AbortError"
    );
  }

  async function shareOrSave(): Promise<void> {
    if (!diagnosticArtifact || deliveryBusy) return;
    deliveryBusy = true;
    deliveryError = null;
    actionStatus = null;
    try {
      const method = await deliverSafetyArtifact(diagnosticArtifact);
      actionStatus =
        method === "shared"
          ? "기기 공유 화면을 열었습니다."
          : "브라우저에 파일 저장을 요청했습니다.";
    } catch (error) {
      if (isShareCancellation(error)) {
        actionStatus = "공유를 취소했습니다.";
      } else {
        deliveryError = publicArtifactDeliveryError();
      }
    } finally {
      deliveryBusy = false;
    }
  }

  async function copyJson(): Promise<void> {
    if (!diagnosticArtifact || deliveryBusy) return;
    deliveryBusy = true;
    deliveryError = null;
    actionStatus = null;
    try {
      await copySafetyArtifactJson(diagnosticArtifact);
      actionStatus = "진단 JSON을 복사했습니다.";
    } catch {
      deliveryError = publicArtifactCopyError();
    } finally {
      deliveryBusy = false;
    }
  }

  onMount(() => {
    mounted = true;
    void reloadStatuses();
    return () => {
      mounted = false;
      requestGeneration += 1;
    };
  });
</script>

<div class="support-pane">
  <div class="pane-heading">
    <div>
      <h3>기기 및 개인정보</h3>
      <p>앱 코어가 실제로 제공하는 저장·보호 상태입니다.</p>
    </div>
    <button
      class="quiet-button lp-state-layer"
      type="button"
      onclick={reloadStatuses}
      disabled={statusBusy}
    >
      {statusBusy ? "확인 중" : "다시 확인"}
    </button>
  </div>

  <div class="support-card" aria-busy={statusBusy}>
    <div class="status-row">
      <div class="status-copy">
        <strong>대화 저장소</strong>
        {#if storageStatus}
          <small>
            {storageStatus.available
              ? `기기 내부 저장 · 형식 ${storageStatus.schemaVersion ?? "확인 중"}${
                  storageNeedsReview ? " · 유지 관리 확인 필요" : ""
                }`
              : "대화를 기기에 저장할 수 없습니다."}
          </small>
        {:else}
          <small>{statusBusy ? "상태를 확인하고 있습니다." : "상태 확인 실패"}</small>
        {/if}
      </div>
      <span
        class:ok={storageStatus?.available === true && !storageNeedsReview}
        class:warning={storageStatus?.available === false ||
          storageNeedsReview}
        class="state"
      >
        {storageStatus
          ? storageStatus.available
            ? storageNeedsReview
              ? "확인 필요"
              : "정상"
            : "확인 필요"
          : "미확인"}
      </span>
    </div>

    <div class="status-row">
      <div class="status-copy">
        <strong>카드·미디어 저장소</strong>
        {#if assetStatus?.available && assetStatus.stats}
          <small>
            {formatCount(assetStatus.stats.objectCount)}개 ·
            {formatBytes(assetStatus.stats.activeBytes)}
            {#if assetNeedsReview}
              · 누락 {formatCount(assetStatus.stats.missingCount)}개 · 격리
              {formatCount(assetStatus.stats.quarantinedCount)}개
            {/if}
          </small>
        {:else if assetStatus}
          <small>{assetErrorMessage(assetStatus.errorCode)}</small>
        {:else}
          <small>{statusBusy ? "상태를 확인하고 있습니다." : "상태 확인 실패"}</small>
        {/if}
      </div>
      <span
        class:ok={assetStatus?.available === true && !assetNeedsReview}
        class:warning={assetStatus?.available === false || assetNeedsReview}
        class="state"
      >
        {assetStatus
          ? assetStatus.available
            ? assetNeedsReview
              ? "정리 필요"
              : "정상"
            : "확인 필요"
          : "미확인"}
      </span>
    </div>
  </div>

  {#if statusError}
    <p class="inline-error" role="status">{statusError}</p>
  {/if}

  <div class="privacy-card">
    <div class="privacy-heading">
      <strong>보호 원칙</strong>
      <span class:ok={safetyContract !== null} class="state">
        {safetyContract ? "코어 확인됨" : "미확인"}
      </span>
    </div>
    {#if safetyContract}
      <ul>
        <li>대화 요청은 사용자가 선택한 LLM 제공자에게만 보냅니다.</li>
        <li>API 키는 기기 보안 저장소에만 두고 진단 정보에 넣지 않습니다.</li>
        <li>가져온 JavaScript·Lua는 보안 정책에 따라 실행하지 않습니다.</li>
      </ul>
    {:else}
      <p>
        {statusBusy
          ? "제품 보호 계약을 확인하고 있습니다."
          : "제품 보호 계약을 확인하지 못했습니다."}
      </p>
    {/if}
  </div>

  <div class="diagnostic-card">
    <div class="diagnostic-heading">
      <div>
        <strong>진단 정보</strong>
        <p>
          사용자 요청으로 기기에서만 초안을 만듭니다. 자동 전송이나 원격 신고는
          하지 않습니다.
        </p>
      </div>
      <button
        class="primary-button lp-state-layer"
        type="button"
        onclick={createDiagnostics}
        disabled={diagnosticBusy}
      >
        {diagnosticBusy
          ? "만드는 중"
          : diagnosticArtifact
            ? "다시 만들기"
            : "초안 만들기"}
      </button>
    </div>

    {#if diagnosticError}
      <p class="inline-error" role="alert">{diagnosticError}</p>
    {/if}

    {#if diagnosticArtifact}
      <div class="artifact-summary">
        <span>{diagnosticArtifact.fileName}</span>
        <small>{formatBytes(String(diagnosticArtifact.byteLength))}</small>
      </div>
      <p class="redaction-proof">
        API 키, 대화·페르소나, 프롬프트·로어, 파일 경로, 원시 제공자 오류가
        없음을 코어 응답에서 확인했습니다.
      </p>
      <details>
        <summary>생성된 JSON 검토</summary>
        <pre>{diagnosticArtifact.json}</pre>
      </details>
      <div class="artifact-actions">
        <button
          class="secondary-button lp-state-layer"
          type="button"
          onclick={shareOrSave}
          disabled={deliveryBusy}
        >
          공유 또는 파일 저장
        </button>
        <button
          class="secondary-button lp-state-layer"
          type="button"
          onclick={copyJson}
          disabled={deliveryBusy}
        >
          JSON 복사
        </button>
      </div>
    {/if}

    {#if deliveryError}
      <p class="inline-error" role="alert">{deliveryError}</p>
    {:else if actionStatus}
      <p class="action-status" role="status">{actionStatus}</p>
    {/if}
  </div>
</div>

<style>
  .support-pane {
    display: grid;
    gap: var(--sp-3);
    color: var(--text-strong);
  }

  .pane-heading,
  .diagnostic-heading,
  .privacy-heading,
  .artifact-summary,
  .artifact-actions {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--sp-3);
  }

  h3,
  p {
    margin: 0;
  }

  h3,
  .privacy-heading strong,
  .diagnostic-heading strong {
    font-size: var(--fs-ui);
    font-weight: 600;
  }

  .pane-heading p,
  .diagnostic-heading p,
  .privacy-card > p {
    margin-top: 2px;
    color: var(--text-mid);
    font-size: var(--fs-caption);
    line-height: 1.45;
  }

  .support-card,
  .privacy-card,
  .diagnostic-card {
    padding: 0 var(--sp-4);
    background: var(--surface-card);
    border-radius: var(--r-card);
    box-shadow: var(--shadow-card);
  }

  .status-row {
    min-height: 58px;
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--sp-3);
    padding: var(--sp-1) 0;
  }

  .status-row + .status-row {
    border-top: 0.5px solid var(--hairline);
  }

  .status-copy {
    min-width: 0;
    display: grid;
    gap: 2px;
  }

  .status-copy strong {
    font-size: var(--fs-ui);
    font-weight: 500;
  }

  .status-copy small,
  .artifact-summary small {
    color: var(--text-mid);
    font-size: var(--fs-caption);
    line-height: 1.4;
  }

  .state {
    min-height: 24px;
    display: inline-flex;
    align-items: center;
    flex-shrink: 0;
    padding: 0 var(--sp-2);
    border-radius: var(--r-pill);
    background: var(--surface-field);
    color: var(--text-mid);
    font-size: var(--fs-caption);
    white-space: nowrap;
  }

  .state.ok {
    background: var(--success-soft);
    color: var(--success);
  }

  .state.warning {
    background: var(--warning-soft);
    color: var(--warning);
  }

  .privacy-card,
  .diagnostic-card {
    padding-top: var(--sp-4);
    padding-bottom: var(--sp-4);
  }

  .privacy-card ul {
    margin: var(--sp-3) 0 0;
    padding-left: 18px;
    color: var(--text-mid);
    font-size: var(--fs-label);
    line-height: 1.65;
  }

  .diagnostic-heading {
    align-items: flex-start;
  }

  .diagnostic-heading > div {
    min-width: 0;
  }

  .diagnostic-heading p {
    max-width: 48ch;
  }

  button {
    min-height: var(--size-touch);
    border: 0;
    border-radius: var(--r-pill);
    font-family: var(--font-ui);
    font-size: var(--fs-label);
    cursor: pointer;
  }

  button:disabled {
    opacity: 0.45;
    cursor: default;
  }

  .quiet-button {
    padding: 0 var(--sp-3);
    background: transparent;
    color: var(--tint);
  }

  .primary-button {
    flex-shrink: 0;
    padding: 0 var(--sp-4);
    background: var(--invert-surface);
    color: var(--invert-text);
  }

  .secondary-button {
    flex: 1 1 0;
    padding: 0 var(--sp-3);
    background: var(--surface-field);
    color: var(--text-strong);
  }

  .artifact-summary {
    margin-top: var(--sp-4);
    padding-top: var(--sp-3);
    border-top: 0.5px solid var(--hairline);
    font-size: var(--fs-label);
  }

  .redaction-proof {
    margin-top: var(--sp-2);
    color: var(--success);
    font-size: var(--fs-caption);
    line-height: 1.5;
  }

  details {
    margin-top: var(--sp-3);
    border: 0.5px solid var(--hairline);
    border-radius: var(--r-block);
    overflow: hidden;
  }

  summary {
    min-height: var(--size-touch);
    display: flex;
    align-items: center;
    padding: 0 var(--sp-3);
    color: var(--text-strong);
    font-size: var(--fs-label);
    cursor: pointer;
  }

  pre {
    max-height: 240px;
    margin: 0;
    padding: var(--sp-3);
    overflow: auto;
    border-top: 0.5px solid var(--hairline);
    background: var(--surface-field);
    color: var(--text-mid);
    font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
    font-size: 10px;
    line-height: 1.5;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    user-select: text;
  }

  .artifact-actions {
    margin-top: var(--sp-3);
  }

  .inline-error,
  .action-status {
    padding: 0 var(--sp-2);
    font-size: var(--fs-caption);
    line-height: 1.5;
  }

  .inline-error {
    color: var(--danger);
  }

  .action-status {
    margin-top: var(--sp-2);
    color: var(--text-mid);
  }

  @media (max-width: 380px) {
    .diagnostic-heading {
      display: grid;
    }

    .primary-button {
      justify-self: stretch;
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .lp-state-layer::after {
      transition: none;
    }
  }
</style>
