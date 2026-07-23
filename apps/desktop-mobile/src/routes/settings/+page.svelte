<script lang="ts">
  import "$lib/design/tokens.css";

  import { theme, type ThemePreference } from "$lib/design/theme.svelte";
  import type { ThreadMode } from "$lib/chat/types";
  import {
    LLM_PROVIDER_CATALOG,
    getLlmProvider,
    type LlmProviderId,
    type LlmProviderDefinition,
  } from "$lib/providers/catalog";
  import {
    deleteProviderCredential,
    publicCredentialErrorMessage,
    requestCredentialStatus,
    saveProviderApiKey,
  } from "$lib/providers/credentials";
  import {
    MAX_MODEL_ID_BYTES,
    activeProviderProfile,
  } from "$lib/providers/active-profile.svelte";
  import { appPreferences } from "$lib/storage/app-preferences.svelte";

  const themeOptions: { value: ThemePreference; label: string }[] = [
    { value: "system", label: "시스템" },
    { value: "light", label: "라이트" },
    { value: "dark", label: "다크" },
  ];

  const modeOptions: { value: ThreadMode; label: string }[] = [
    { value: "chat", label: "채팅" },
    { value: "story", label: "스토리" },
  ];

  let defaultMode = $derived(appPreferences.current.defaultMode);
  let selectedProviderId = $derived(activeProviderProfile.selectedProviderId);
  let selectedProvider = $derived(getLlmProvider(selectedProviderId));

  function targetLabel(provider: LlmProviderDefinition): string {
    if (provider.target.kind === "fixed-origin") {
      return new URL(provider.target.origin).hostname;
    }

    return `${provider.target.serviceDomain} · 리전에 따라 지역 엔드포인트`;
  }

  const isApiKeyProvider = $derived(selectedProvider.authKind === "api-key");
  const credentialConfigured = $derived(
    activeProviderProfile.credentialConfigured,
  );
  const modelId = $derived(activeProviderProfile.modelId);
  const modelError = $derived(activeProviderProfile.modelError);
  const selectedProfileReady = $derived(
    activeProviderProfile.current?.providerId === selectedProviderId,
  );

  let credentialBusy = $state(false);
  let credentialError = $state<string | null>(null);
  let keyDraft = $state("");

  function selectProvider(providerId: LlmProviderId): void {
    activeProviderProfile.select(providerId);
    appPreferences.setProvider(providerId);
  }

  function updateModelId(value: string): void {
    if (selectedProviderId === "google-vertex-ai") return;
    activeProviderProfile.setModelId(value);
    appPreferences.setModelId(selectedProviderId, value);
  }

  async function refreshCredentialStatus(
    providerId: LlmProviderId,
  ): Promise<void> {
    const epoch = activeProviderProfile.beginCredentialOperation(providerId);
    try {
      const status = await requestCredentialStatus(providerId);
      if (
        activeProviderProfile.isCredentialOperationCurrent(providerId, epoch)
      ) {
        activeProviderProfile.setCredentialConfigured(
          providerId,
          status.configured,
        );
      }
    } catch (error) {
      if (
        activeProviderProfile.isCredentialOperationCurrent(providerId, epoch) &&
        providerId === selectedProviderId
      ) {
        activeProviderProfile.setCredentialConfigured(providerId, null);
        credentialError = publicCredentialErrorMessage(error);
      }
    }
  }

  $effect(() => {
    const providerId = selectedProviderId;
    keyDraft = "";
    credentialError = null;
    activeProviderProfile.setCredentialConfigured(providerId, null);
    if (getLlmProvider(providerId).authKind === "api-key") {
      void refreshCredentialStatus(providerId);
    } else {
      activeProviderProfile.setCredentialConfigured(providerId, false);
    }
  });

  async function saveKey(): Promise<void> {
    const secret = keyDraft.trim();
    if (!secret || credentialBusy) {
      return;
    }
    credentialBusy = true;
    credentialError = null;
    const providerId = selectedProviderId;
    activeProviderProfile.beginCredentialOperation(providerId);
    try {
      const status = await saveProviderApiKey(providerId, secret);
      activeProviderProfile.beginCredentialOperation(providerId);
      activeProviderProfile.setCredentialConfigured(
        providerId,
        status.configured,
      );
      keyDraft = "";
    } catch (error) {
      activeProviderProfile.beginCredentialOperation(providerId);
      if (providerId === selectedProviderId) {
        credentialError = publicCredentialErrorMessage(error);
      }
      void refreshCredentialStatus(providerId);
    } finally {
      credentialBusy = false;
    }
  }

  async function removeKey(): Promise<void> {
    if (credentialBusy) {
      return;
    }
    credentialBusy = true;
    credentialError = null;
    const providerId = selectedProviderId;
    activeProviderProfile.beginCredentialOperation(providerId);
    try {
      const status = await deleteProviderCredential(providerId);
      activeProviderProfile.beginCredentialOperation(providerId);
      activeProviderProfile.setCredentialConfigured(
        providerId,
        status.configured,
      );
    } catch (error) {
      activeProviderProfile.beginCredentialOperation(providerId);
      if (providerId === selectedProviderId) {
        credentialError = publicCredentialErrorMessage(error);
      }
      void refreshCredentialStatus(providerId);
    } finally {
      credentialBusy = false;
    }
  }
</script>

<svelte:head>
  <title>LorePia — 설정</title>
</svelte:head>

<div class="screen">
  <header class="top">
    <h1>설정</h1>
  </header>

  <section>
    <h2>화면</h2>
    <div class="card">
      <div class="row">
        <span class="lead">
          <span class="ric" aria-hidden="true">
            <svg
              viewBox="0 0 24 24"
              width="16"
              height="16"
              fill="none"
              stroke="currentColor"
              stroke-width="2"
              stroke-linecap="round"
              stroke-linejoin="round"
            >
              <circle cx="12" cy="12" r="4" />
              <path d="M12 2v2" />
              <path d="M12 20v2" />
              <path d="m4.9 4.9 1.4 1.4" />
              <path d="m17.7 17.7 1.4 1.4" />
              <path d="M2 12h2" />
              <path d="M20 12h2" />
              <path d="m6.3 17.7-1.4 1.4" />
              <path d="m19.1 4.9-1.4 1.4" />
            </svg>
          </span>
          <span class="label">테마</span>
        </span>
        <div class="segment" role="group" aria-label="테마 선택">
          {#each themeOptions as option (option.value)}
            <button
              type="button"
              class:active={theme.preference === option.value}
              onclick={() => appPreferences.setTheme(option.value)}
              >{option.label}</button
            >
          {/each}
        </div>
      </div>
      <div class="row">
        <span class="lead">
          <span class="ric" aria-hidden="true">
            <svg
              viewBox="0 0 24 24"
              width="16"
              height="16"
              fill="none"
              stroke="currentColor"
              stroke-width="2"
              stroke-linecap="round"
              stroke-linejoin="round"
            >
              <path d="M12 7v14" />
              <path
                d="M3 18a1 1 0 0 1-1-1V4a1 1 0 0 1 1-1h5a4 4 0 0 1 4 4 4 4 0 0 1 4-4h5a1 1 0 0 1 1 1v13a1 1 0 0 1-1 1h-6a3 3 0 0 0-3 3 3 3 0 0 0-3-3z"
              />
            </svg>
          </span>
          <span class="label">기본 표시 모드</span>
        </span>
        <div class="segment" role="group" aria-label="기본 표시 모드 선택">
          {#each modeOptions as option (option.value)}
            <button
              type="button"
              class:active={defaultMode === option.value}
              onclick={() => appPreferences.setDefaultMode(option.value)}
              >{option.label}</button
            >
          {/each}
        </div>
      </div>
    </div>
  </section>

  <section>
    <h2>연결</h2>
    <div class="connection-heading">
      <span class="label">LLM 제공자</span>
      <span class="status" class:configured={selectedProfileReady}
        >{selectedProfileReady
          ? "사용 준비됨"
          : credentialConfigured === true
            ? "모델 선택 필요"
            : "연결 전"}</span
      >
    </div>

    <fieldset class="provider-picker">
      <legend>LLM 제공자 선택</legend>
      <div class="provider-grid">
        {#each LLM_PROVIDER_CATALOG as provider (provider.id)}
          <label class:selected={selectedProviderId === provider.id}>
            <input
              type="radio"
              name="llm-provider"
              value={provider.id}
              checked={selectedProviderId === provider.id}
              disabled={credentialBusy}
              onchange={() => selectProvider(provider.id)}
            />
            <span>{provider.label}</span>
            {#if provider.id === "ollama-cloud"}
              <small aria-hidden="true">Cloud</small>
            {:else if provider.id === "google-gemini"}
              <small aria-hidden="true">Developer API</small>
            {:else if provider.id === "google-vertex-ai"}
              <small aria-hidden="true">Google Cloud</small>
            {/if}
          </label>
        {/each}
      </div>
    </fieldset>

    <div class="provider-detail" aria-live="polite">
      <div class="provider-title">
        <div>
          <strong>{selectedProvider.label}</strong>
          <p>{selectedProvider.description}</p>
        </div>
        <span class="candidate">{selectedProfileReady ? "첫 대화 연결" : "구성 중"}</span>
      </div>

      <dl>
        <div>
          <dt>인증</dt>
          <dd>{selectedProvider.authLabel}</dd>
        </div>
        <div>
          <dt>고정 대상</dt>
          <dd>{targetLabel(selectedProvider)}</dd>
        </div>
        <div>
          <dt>모델</dt>
          <dd>{modelId.trim() || "공식 모델 ID를 직접 입력"}</dd>
        </div>
      </dl>

      <div class="required-settings">
        <h3>필요한 설정</h3>
        {#each selectedProvider.setupFields as field (field.id)}
          {#if field.id === "modelId" && isApiKeyProvider}
            <label class="setting-input" for="provider-model-id">
              <span>{field.label}</span>
              <input
                id="provider-model-id"
                type="text"
                value={modelId}
                placeholder="제공자의 공식 모델 ID"
                autocomplete="off"
                spellcheck="false"
                maxlength={MAX_MODEL_ID_BYTES}
                aria-invalid={modelId.length > 0 && modelError !== null}
                oninput={(event) => updateModelId(event.currentTarget.value)}
              />
              <small class:error={modelId.length > 0 && modelError !== null}
                >{modelId.length > 0 && modelError !== null
                  ? modelError
                  : "키는 포함하지 말고 모델 식별자만 입력하세요."}</small
              >
            </label>
          {:else}
            <div class="field-preview">
              <span>{field.label}</span>
              <small>{field.placeholder}</small>
            </div>
          {/if}
        {/each}
      </div>

      {#if isApiKeyProvider}
        <div class="key-entry">
          <label class="key-label" for="provider-key"
            >{selectedProvider.authLabel}</label
          >
          <div class="key-line">
            <input
              id="provider-key"
              type="password"
              placeholder="입력 후 저장하면 기기 보안 저장소로만 이동"
              autocomplete="off"
              bind:value={keyDraft}
              disabled={credentialBusy}
            />
            <button
              class="save"
              type="button"
              onclick={saveKey}
              disabled={credentialBusy || keyDraft.trim().length === 0}
              >저장</button
            >
          </div>
          <div class="key-status">
            {#if credentialConfigured === true}
              <span class="chip ok">저장됨 · 값은 다시 표시되지 않음</span>
              <button
                class="remove"
                type="button"
                onclick={removeKey}
                disabled={credentialBusy}>키 삭제</button
              >
            {:else if credentialConfigured === false}
              <span class="chip">저장된 키 없음</span>
            {:else}
              <span class="chip">상태 확인 중</span>
            {/if}
          </div>
          {#if credentialError}
            <p class="key-error" role="alert">{credentialError}</p>
          {/if}
        </div>
      {:else}
        <div class="field-preview protected">
          <span>{selectedProvider.authLabel}</span>
          <small>Google OAuth 흐름이 구현되기 전까지 연결할 수 없습니다</small>
        </div>
      {/if}
    </div>

    <p class="note security-note">
      키는 이 기기의 OS 자격증명 저장소에만 저장되며, 앱 화면으로 다시 읽어오는
      경로 자체가 없습니다. 외부 요청은 선택한 제공자의 고정 호스트로만
      나갑니다.
    </p>
  </section>

  <section>
    <h2>데이터</h2>
    <p class="note">
      모든 대화와 캐릭터는 기기에만 저장됩니다. 외부로 나가는 요청은 사용자가
      선택한 LLM 호출뿐입니다.
    </p>
    <p class="note">
      가져온 카드의 스크립트는 보존만 되며, 검증된 실행 경계가 준비되기 전까지
      실행되지 않습니다.
    </p>
  </section>

  <section>
    <h2>정보</h2>
    <div class="card">
      <div class="row">
        <span class="lead">
          <span class="ric" aria-hidden="true">
            <svg
              viewBox="0 0 24 24"
              width="16"
              height="16"
              fill="none"
              stroke="currentColor"
              stroke-width="2"
              stroke-linecap="round"
              stroke-linejoin="round"
            >
              <circle cx="12" cy="12" r="9" />
              <path d="M12 16v-5" />
              <path d="M12 8h.01" />
            </svg>
          </span>
          <span class="label">버전</span>
        </span>
        <span class="value">0.1.0</span>
      </div>
      <div class="row">
        <span class="lead">
          <span class="ric" aria-hidden="true">
            <svg
              viewBox="0 0 24 24"
              width="16"
              height="16"
              fill="none"
              stroke="currentColor"
              stroke-width="2"
              stroke-linecap="round"
              stroke-linejoin="round"
            >
              <path d="M15 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V7z" />
              <path d="M14 2v4a2 2 0 0 0 2 2h4" />
              <path d="M8 13h8" />
              <path d="M8 17h5" />
            </svg>
          </span>
          <span class="label">라이선스</span>
        </span>
        <span class="value">Apache-2.0</span>
      </div>
    </div>
  </section>
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

  .top {
    position: sticky;
    top: 0;
    z-index: 5;
    display: flex;
    align-items: center;
    padding: var(--sp-3) var(--sp-4);
    padding-top: calc(var(--sp-3) + var(--safe-top));
    background: var(--bar-bg);
    -webkit-backdrop-filter: blur(20px) saturate(1.6);
    backdrop-filter: blur(20px) saturate(1.6);
  }

  .top h1 {
    margin: 0;
    font-size: var(--fs-title);
    font-weight: 700;
    letter-spacing: -0.03em;
    color: var(--text-strong);
  }

  section {
    padding: 0 var(--sp-4);
    margin-top: var(--sp-4);
    animation: lp-rise var(--dur-page) var(--ease-out) backwards;
  }

  section:nth-of-type(1) {
    animation-delay: 40ms;
  }
  section:nth-of-type(2) {
    animation-delay: 90ms;
  }
  section:nth-of-type(3) {
    animation-delay: 140ms;
  }
  section:nth-of-type(4) {
    animation-delay: 190ms;
  }

  h2 {
    margin: 0 0 var(--sp-2);
    padding-left: var(--sp-2);
    font-size: var(--fs-label);
    font-weight: 600;
    letter-spacing: 0.02em;
    color: var(--text-mid);
  }

  .card {
    padding: 0 var(--sp-4);
    background: var(--surface-card);
    border-radius: var(--r-card);
    box-shadow: var(--shadow-card);
  }

  .row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--sp-3);
    min-height: 52px;
    padding: var(--sp-1) 0;
  }

  .row + .row {
    border-top: 0.5px solid var(--hairline);
  }

  .lead {
    display: inline-flex;
    align-items: center;
    gap: var(--sp-3);
    min-width: 0;
  }

  .ric {
    width: 30px;
    height: 30px;
    flex-shrink: 0;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    border-radius: 9px;
    background: var(--tint-soft);
    color: var(--tint);
  }

  .label {
    font-size: var(--fs-ui);
    color: var(--text-strong);
  }

  .value {
    font-size: var(--fs-ui);
    color: var(--text-strong);
  }

  .connection-heading {
    min-height: var(--size-touch);
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--sp-3);
  }

  .status,
  .candidate {
    display: inline-flex;
    align-items: center;
    min-height: 24px;
    padding: 0 var(--sp-2);
    border: 0.5px solid var(--hairline);
    border-radius: var(--r-pill);
    color: var(--text-mid);
    font-size: var(--fs-caption);
    white-space: nowrap;
  }

  .provider-picker {
    min-width: 0;
    margin: 0;
    padding: 0;
    border: 0;
  }

  .provider-picker legend {
    position: absolute;
    width: 1px;
    height: 1px;
    padding: 0;
    margin: -1px;
    overflow: hidden;
    clip: rect(0, 0, 0, 0);
    white-space: nowrap;
    border: 0;
  }

  .provider-grid {
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: var(--sp-2);
  }

  .provider-grid label {
    position: relative;
    min-height: 52px;
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    justify-content: center;
    gap: 2px;
    padding: var(--sp-2) var(--sp-3);
    border: 0.5px solid var(--hairline);
    border-radius: var(--r-block);
    background: var(--surface-field);
    color: var(--text-strong);
    font-family: var(--font-ui);
    font-size: var(--fs-ui);
    text-align: left;
    cursor: pointer;
    transition:
      border-color var(--dur-fast) var(--ease-out),
      background var(--dur-fast) var(--ease-out),
      color var(--dur-fast) var(--ease-out);
  }

  .provider-grid input {
    position: absolute;
    width: 1px;
    height: 1px;
    margin: -1px;
    overflow: hidden;
    clip: rect(0, 0, 0, 0);
  }

  .provider-grid label:focus-within {
    outline: 2px solid var(--cursor-color);
    outline-offset: 2px;
  }

  .provider-grid label small {
    color: var(--text-mid);
    font-size: var(--fs-caption);
  }

  .provider-grid label.selected {
    border-color: var(--tint);
    box-shadow: inset 0 0 0 1px var(--tint);
    background: var(--tint-soft);
    color: var(--text-strong);
  }

  .provider-grid label.selected small {
    color: var(--tint);
  }

  .provider-detail {
    margin-top: var(--sp-3);
    padding: var(--sp-4);
    border-radius: var(--r-card);
    background: var(--surface-card);
    box-shadow: var(--shadow-card);
  }

  .provider-title {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: var(--sp-3);
  }

  .provider-title strong {
    color: var(--text-strong);
    font-size: var(--fs-chat);
    font-weight: 600;
  }

  .provider-title p {
    margin: var(--sp-1) 0 0;
    color: var(--text-mid);
    font-size: var(--fs-label);
    line-height: 1.5;
  }

  dl {
    margin: var(--sp-3) 0 0;
  }

  dl > div {
    display: grid;
    grid-template-columns: 72px minmax(0, 1fr);
    gap: var(--sp-2);
    padding: var(--sp-2) 0;
    border-top: 0.5px solid var(--hairline);
  }

  dt,
  dd {
    margin: 0;
    font-size: var(--fs-label);
    line-height: 1.5;
  }

  dt {
    color: var(--text-mid);
  }

  dd {
    min-width: 0;
    color: var(--text-strong);
    overflow-wrap: anywhere;
  }

  .required-settings {
    margin-top: var(--sp-3);
  }

  .required-settings h3 {
    margin: 0 0 var(--sp-2);
    color: var(--text-mid);
    font-size: var(--fs-caption);
    font-weight: 500;
  }

  .field-preview,
  .setting-input {
    min-height: var(--size-touch);
    display: flex;
    flex-direction: column;
    justify-content: center;
    gap: 2px;
    padding: var(--sp-2) var(--sp-3);
    box-sizing: border-box;
    border: 0.5px solid var(--field-border);
    border-radius: var(--r-block);
    background: var(--surface-page);
    color: var(--text-strong);
    font-size: var(--fs-label);
  }

  .field-preview + .field-preview,
  .setting-input + .field-preview,
  .field-preview + .setting-input {
    margin-top: var(--sp-2);
  }

  .field-preview small,
  .setting-input small {
    color: var(--text-faint);
    font-size: var(--fs-caption);
  }

  .setting-input input {
    min-height: 36px;
    margin: var(--sp-1) 0;
    box-sizing: border-box;
    padding: 0 var(--sp-2);
    border: 0.5px solid var(--field-border);
    border-radius: var(--r-block);
    background: var(--surface-field);
    color: var(--text-strong);
    font-family: var(--font-ui);
    font-size: 16px;
    outline: none;
  }

  .setting-input input:focus-visible {
    border-color: var(--text-mid);
  }

  .setting-input small.error {
    color: var(--danger, #a33);
  }

  .field-preview.protected {
    border-style: dashed;
  }

  .key-entry {
    margin-top: var(--sp-3);
    display: flex;
    flex-direction: column;
    gap: var(--sp-2);
  }

  .key-label {
    font-size: var(--fs-caption);
    font-weight: 500;
    color: var(--text-mid);
  }

  .key-line {
    display: flex;
    gap: var(--sp-2);
  }

  .key-line input {
    flex: 1;
    min-width: 0;
    min-height: var(--size-touch);
    box-sizing: border-box;
    padding: 0 var(--sp-3);
    border: 0.5px solid var(--field-border);
    border-radius: var(--r-block);
    background: var(--surface-page);
    color: var(--text-strong);
    font-family: var(--font-ui);
    font-size: 16px;
    outline: none;
    transition: border-color var(--dur-fast) var(--ease-out);
  }

  .key-line input::placeholder {
    color: var(--text-faint);
    font-size: var(--fs-label);
  }

  .key-line input:focus-visible {
    border-color: var(--text-mid);
  }

  .key-line .save {
    min-height: var(--size-touch);
    padding: 0 var(--sp-4);
    border: none;
    border-radius: var(--r-pill);
    background: var(--tint);
    color: #fff;
    font-family: var(--font-ui);
    font-size: var(--fs-ui);
    font-weight: 600;
    cursor: pointer;
    flex-shrink: 0;
    transition:
      opacity var(--dur-fast) var(--ease-out),
      transform var(--dur-fast) var(--ease-spring);
  }

  .key-line .save:not(:disabled):active {
    transform: scale(0.95);
  }

  .key-line .save:disabled {
    opacity: 0.35;
    cursor: default;
  }

  .key-status {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--sp-3);
  }

  .chip {
    display: inline-flex;
    align-items: center;
    min-height: 24px;
    padding: 0 var(--sp-2);
    border: 0.5px solid var(--hairline);
    border-radius: var(--r-pill);
    color: var(--text-mid);
    font-size: var(--fs-caption);
  }

  .chip.ok {
    border-color: transparent;
    background: var(--success-soft);
    color: var(--success);
    font-weight: 500;
  }

  .remove {
    min-height: 32px;
    padding: 0 var(--sp-2);
    border: none;
    background: transparent;
    color: var(--text-mid);
    font-family: var(--font-ui);
    font-size: var(--fs-label);
    text-decoration: underline;
    cursor: pointer;
  }

  .remove:disabled {
    opacity: 0.5;
    cursor: default;
  }

  .key-error {
    margin: 0;
    font-size: var(--fs-label);
    line-height: 1.5;
    color: var(--danger);
  }

  .status.configured {
    border-color: transparent;
    background: var(--tint-soft);
    color: var(--tint);
    font-weight: 500;
  }

  .security-note {
    padding-left: var(--sp-3);
    border-left: 2px solid var(--hairline);
  }

  .segment {
    display: flex;
    background: var(--surface-bubble);
    border-radius: 10px;
    padding: 2px;
    gap: 2px;
  }

  .segment button {
    min-height: 30px;
    padding: 0 var(--sp-3);
    border: none;
    border-radius: 8px;
    background: transparent;
    color: var(--text-mid);
    font-family: var(--font-ui);
    font-size: var(--fs-label);
    font-weight: 500;
    cursor: pointer;
    transition:
      background var(--dur-base) var(--ease-out),
      color var(--dur-base) var(--ease-out),
      box-shadow var(--dur-base) var(--ease-out),
      transform var(--dur-fast) var(--ease-spring);
  }

  .segment button:active {
    transform: scale(0.95);
  }

  .segment button.active {
    background: var(--segment-thumb);
    color: var(--text-strong);
    font-weight: 600;
    box-shadow: 0 1px 4px rgba(0, 0, 0, 0.12);
  }

  .note {
    margin: var(--sp-2) 0 0;
    font-size: var(--fs-label);
    line-height: 1.6;
    color: var(--text-mid);
  }

  @media (min-width: 640px) {
    .provider-grid {
      grid-template-columns: repeat(3, minmax(0, 1fr));
    }
  }
</style>
