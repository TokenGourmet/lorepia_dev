<script lang="ts">
  import { goto } from "$app/navigation";
  import { page } from "$app/state";
  import { onMount } from "svelte";

  import "$lib/design/tokens.css";

  import {
    characterChatTitle,
    findSampleCharacter,
  } from "$lib/characters/sample";
  import { activeProviderProfile } from "$lib/providers/active-profile.svelte";
  import {
    LLM_PROVIDER_CATALOG,
    type LlmProviderId,
  } from "$lib/providers/catalog";
  import { appPreferences } from "$lib/storage/app-preferences.svelte";
  import {
    FIRST_CHAT_CHARACTER_ID,
    loadOrCreateCharacterChat,
  } from "$lib/storage/chat-history";
  import {
    storageClient,
    type ChatCursor,
    type StoredMessage,
  } from "$lib/storage/client";
  import {
    copySafetyArtifactJson,
    deliverSafetyArtifact,
    publicArtifactCopyError,
    publicArtifactDeliveryError,
  } from "$lib/system/artifact-delivery";
  import {
    MAX_REPORT_COMMENT_BYTES,
    MAX_REPORT_EXCERPT_BYTES,
    publicNativeSupportError,
    requestAiOutputReport,
    toSafetyProviderKind,
    utf8ByteLength,
    type AiReportCategory,
    type SafetyArtifact,
  } from "$lib/system/native-support";
  import LargeTitleHeader from "$lib/ui/LargeTitleHeader.svelte";
  import { activateBackSwipeSurface } from "$lib/ui/back-swipe-surface";
  import { edgeSwipeBack } from "$lib/ui/edge-back";

  const fallbackCharacter = findSampleCharacter(FIRST_CHAT_CHARACTER_ID)!;
  const character = $derived(
    findSampleCharacter(
      page.url.searchParams.get("character") ?? FIRST_CHAT_CHARACTER_ID,
    ) ?? fallbackCharacter,
  );

  const categoryOptions = [
    { value: "SAFETY_CONCERN", label: "안전 우려" },
    { value: "HARASSMENT_OR_HATE", label: "괴롭힘 또는 혐오" },
    { value: "SEXUAL_CONTENT", label: "성적 콘텐츠" },
    { value: "SELF_HARM", label: "자해 관련 콘텐츠" },
    { value: "ILLEGAL_OR_DANGEROUS", label: "불법 또는 위험한 콘텐츠" },
    { value: "PRIVACY_CONCERN", label: "개인정보 우려" },
    { value: "COPYRIGHT_CONCERN", label: "저작권 우려" },
    { value: "INCORRECT_OR_LOW_QUALITY", label: "부정확하거나 낮은 품질" },
    { value: "OTHER", label: "기타" },
  ] as const satisfies readonly {
    value: AiReportCategory;
    label: string;
  }[];

  const dateFormatter = new Intl.DateTimeFormat("ko-KR", {
    month: "numeric",
    day: "numeric",
    hour: "numeric",
    minute: "2-digit",
  });

  let chatId = $state<string | null>(null);
  let loading = $state(true);
  let loadError = $state<string | null>(null);
  let assistantMessages = $state<StoredMessage[]>([]);
  let selectedMessageId = $state("");
  let providerId = $state<LlmProviderId>("openai");
  let category = $state<AiReportCategory>("SAFETY_CONCERN");
  let userComment = $state("");
  let includeSelectedOutput = $state(false);
  let selectedOutputExcerpt = $state("");
  let creating = $state(false);
  let actionBusy = $state(false);
  let errorMessage = $state<string | null>(null);
  let successMessage = $state<string | null>(null);
  let artifact = $state<SafetyArtifact | null>(null);

  const selectedMessage = $derived(
    assistantMessages.find((message) => message.id === selectedMessageId) ??
      null,
  );
  const commentBytes = $derived(utf8ByteLength(userComment));
  const excerptBytes = $derived(utf8ByteLength(selectedOutputExcerpt));
  const selectedOutputNeedsTruncation = $derived(
    selectedMessage !== null &&
      utf8ByteLength(selectedMessage.text) > MAX_REPORT_EXCERPT_BYTES,
  );
  const infoHref = $derived(
    `/chat/info?character=${encodeURIComponent(character.id)}${
      chatId === null ? "" : `&chatId=${encodeURIComponent(chatId)}`
    }`,
  );
  const formError = $derived(
    selectedMessage === null
      ? "신고 초안을 만들 AI 응답을 선택해 주세요."
      : commentBytes > MAX_REPORT_COMMENT_BYTES
        ? "메모가 허용 길이를 초과했습니다."
        : includeSelectedOutput && selectedOutputExcerpt.trim().length === 0
          ? "포함할 응답 내용을 입력해 주세요."
          : excerptBytes > MAX_REPORT_EXCERPT_BYTES
            ? "포함할 응답 내용이 허용 길이를 초과했습니다."
            : null,
  );

  function optionLabel(message: StoredMessage): string {
    const excerpt = message.text.replace(/\s+/gu, " ").trim();
    const short = excerpt.length > 36 ? `${excerpt.slice(0, 36)}…` : excerpt;
    return `${dateFormatter.format(new Date(message.createdAtMs))} · ${
      short || "(내용 없는 응답)"
    }`;
  }

  function boundedExcerpt(value: string): string {
    const bytes = new TextEncoder().encode(value);
    if (bytes.byteLength <= MAX_REPORT_EXCERPT_BYTES) return value;
    let end = MAX_REPORT_EXCERPT_BYTES;
    while (end > 0 && (bytes[end] & 0xc0) === 0x80) {
      end -= 1;
    }
    return new TextDecoder("utf-8", { fatal: true }).decode(
      bytes.subarray(0, end),
    );
  }

  function clearGeneratedArtifact(): void {
    artifact = null;
    errorMessage = null;
    successMessage = null;
  }

  function returnHref(): string | null {
    const candidate = (page.state as { backHref?: unknown }).backHref;
    return typeof candidate === "string" &&
      candidate.startsWith("/") &&
      !candidate.startsWith("//")
      ? candidate
      : null;
  }

  function isMatchingInfoHref(candidate: string): boolean {
    const target = new URL(candidate, window.location.origin);
    return (
      target.pathname === "/chat/info" &&
      target.searchParams.get("character") === character.id
    );
  }

  function navigateBack(): void {
    const candidate = returnHref();
    if (
      candidate !== null &&
      isMatchingInfoHref(candidate) &&
      window.history.length > 1
    ) {
      window.history.back();
      return;
    }
    void goto(infoHref, { replaceState: true });
  }

  function handleBackClick(event: MouseEvent): void {
    event.preventDefault();
    navigateBack();
  }

  function isUserCancelled(error: unknown): boolean {
    return (
      typeof error === "object" &&
      error !== null &&
      "name" in error &&
      error.name === "AbortError"
    );
  }

  function selectMessage(event: Event): void {
    selectedMessageId = (event.currentTarget as HTMLSelectElement).value;
    if (includeSelectedOutput) {
      selectedOutputExcerpt = boundedExcerpt(selectedMessage?.text ?? "");
    }
    clearGeneratedArtifact();
  }

  function toggleOutputInclusion(event: Event): void {
    includeSelectedOutput = (event.currentTarget as HTMLInputElement).checked;
    selectedOutputExcerpt = includeSelectedOutput
      ? boundedExcerpt(selectedMessage?.text ?? "")
      : "";
    clearGeneratedArtifact();
  }

  async function resolveReportChatId(fallbackChatId: string): Promise<string> {
    const requestedChatId = page.url.searchParams.get("chatId");
    if (
      requestedChatId === null ||
      !/^[a-f0-9]{32}$/u.test(requestedChatId) ||
      requestedChatId === fallbackChatId
    ) {
      return fallbackChatId;
    }

    let before: ChatCursor | null = null;
    for (let pageIndex = 0; pageIndex < 100; pageIndex += 1) {
      const listed = await storageClient.listChats(100, before);
      const requested = listed.items.find(
        (candidate) => candidate.id === requestedChatId,
      );
      if (requested !== undefined) {
        return requested.characterId === character.id
          ? requested.id
          : fallbackChatId;
      }
      if (listed.nextCursor === null) break;
      before = listed.nextCursor;
    }
    return fallbackChatId;
  }

  async function loadAssistantMessages(): Promise<void> {
    loading = true;
    loadError = null;
    try {
      await appPreferences.hydrate();
      providerId = activeProviderProfile.selectedProviderId;
      const loaded = await loadOrCreateCharacterChat(
        character.id,
        characterChatTitle(character.name),
      );
      const targetChatId = await resolveReportChatId(loaded.chat.id);
      chatId = targetChatId;
      const messagePage = await storageClient.loadChatMessages(targetChatId);
      assistantMessages = messagePage.items.filter(
        (message) => message.role === "assistant",
      );
      selectedMessageId = assistantMessages.at(-1)?.id ?? "";
    } catch {
      chatId = null;
      assistantMessages = [];
      selectedMessageId = "";
      loadError = "저장된 AI 응답을 불러오지 못했습니다.";
    } finally {
      loading = false;
    }
  }

  async function createDraft(): Promise<void> {
    const message = selectedMessage;
    if (message === null || formError !== null || creating) return;
    creating = true;
    errorMessage = null;
    successMessage = null;
    artifact = null;
    try {
      artifact = await requestAiOutputReport({
        messageId: message.id,
        provider: toSafetyProviderKind(providerId),
        category,
        userComment: userComment.trim() || null,
        selectedOutputExcerpt: includeSelectedOutput
          ? selectedOutputExcerpt.trim()
          : null,
        includeSelectedOutput,
      });
      successMessage =
        "기기에서 검토용 JSON 초안을 만들었습니다. 어디에도 전송되지 않았습니다.";
    } catch (error) {
      errorMessage = publicNativeSupportError(error);
    } finally {
      creating = false;
    }
  }

  async function exportArtifact(): Promise<void> {
    const current = artifact;
    if (current === null || actionBusy) return;
    actionBusy = true;
    errorMessage = null;
    successMessage = null;
    try {
      const method = await deliverSafetyArtifact(current);
      successMessage =
        method === "shared"
          ? "공유 화면을 열었습니다."
          : "브라우저에 JSON 파일 저장을 요청했습니다.";
    } catch (error) {
      if (isUserCancelled(error)) return;
      errorMessage = publicArtifactDeliveryError();
    } finally {
      actionBusy = false;
    }
  }

  async function copyArtifact(): Promise<void> {
    const current = artifact;
    if (current === null || actionBusy) return;
    actionBusy = true;
    errorMessage = null;
    successMessage = null;
    try {
      await copySafetyArtifactJson(current);
      successMessage = "JSON을 클립보드에 복사했습니다.";
    } catch {
      errorMessage = publicArtifactCopyError();
    } finally {
      actionBusy = false;
    }
  }

  onMount(() => {
    void loadAssistantMessages();
  });
</script>

<svelte:head>
  <title>LorePia — AI 응답 신고 초안</title>
</svelte:head>

<div
  class="screen"
  use:edgeSwipeBack={{
    onBack: navigateBack,
    getUnderlay: () => activateBackSwipeSurface(infoHref),
  }}
>
  <LargeTitleHeader title="신고 초안">
    {#snippet leading()}
      <a
        class="back"
        href={infoHref}
        aria-label="대화 설정으로 돌아가기"
        onclick={handleBackClick}
      >
        <svg
          viewBox="0 0 24 24"
          width="20"
          height="20"
          fill="none"
          stroke="currentColor"
          stroke-width="2"
          stroke-linecap="round"
          stroke-linejoin="round"
          aria-hidden="true"
        >
          <path d="m15 18-6-6 6-6" />
        </svg>
      </a>
    {/snippet}
  </LargeTitleHeader>

  <p class="disclosure">
    이 화면은 검토용 JSON 파일만 기기에서 만듭니다. 원격 제출이나 네트워크
    요청은 발생하지 않습니다.
  </p>

  {#if loading}
    <p class="page-status" role="status">저장된 AI 응답을 불러오는 중입니다.</p>
  {:else if loadError}
    <div class="page-status recovery" role="alert">
      <span>{loadError}</span>
      <button type="button" onclick={loadAssistantMessages}>다시 시도</button>
    </div>
  {:else if assistantMessages.length === 0}
    <div class="empty">
      <p>아직 저장된 AI 응답이 없습니다.</p>
      <a href={`/chat?character=${encodeURIComponent(character.id)}`}
        >대화로 돌아가기</a
      >
    </div>
  {:else}
    <form onsubmit={(event) => event.preventDefault()}>
      <section class="group" aria-labelledby="output-label">
        <h2 id="output-label">응답 선택</h2>
        <div class="card fields">
          <label>
            <span>저장된 AI 응답</span>
            <select value={selectedMessageId} onchange={selectMessage}>
              {#each assistantMessages as message (message.id)}
                <option value={message.id}>{optionLabel(message)}</option>
              {/each}
            </select>
          </label>
          <label>
            <span>응답 제공자</span>
            <select
              bind:value={providerId}
              onchange={clearGeneratedArtifact}
            >
              {#each LLM_PROVIDER_CATALOG as provider (provider.id)}
                <option value={provider.id}>{provider.label}</option>
              {/each}
            </select>
            <small>
              저장 기록만으로는 자동 판별할 수 없습니다. 실제 응답 제공자를
              확인해 선택해 주세요.
            </small>
          </label>
          <label>
            <span>분류</span>
            <select bind:value={category} onchange={clearGeneratedArtifact}>
              {#each categoryOptions as option (option.value)}
                <option value={option.value}>{option.label}</option>
              {/each}
            </select>
          </label>
        </div>
      </section>

      <section class="group" aria-labelledby="detail-label">
        <h2 id="detail-label">추가 내용</h2>
        <div class="card fields">
          <label>
            <span>메모 <small>선택 사항</small></span>
            <textarea
              bind:value={userComment}
              oninput={clearGeneratedArtifact}
              rows="4"
              placeholder="검토할 내용을 적어 주세요"
            ></textarea>
            <small class:limit={commentBytes > MAX_REPORT_COMMENT_BYTES}>
              {commentBytes.toLocaleString("ko-KR")} / {MAX_REPORT_COMMENT_BYTES.toLocaleString("ko-KR")}바이트
            </small>
          </label>

          <label class="consent">
            <input
              type="checkbox"
              checked={includeSelectedOutput}
              onchange={toggleOutputInclusion}
            />
            <span>선택한 응답 내용을 JSON에 포함</span>
          </label>

          {#if includeSelectedOutput}
            <label>
              <span>포함할 응답 내용</span>
              <textarea
                bind:value={selectedOutputExcerpt}
                oninput={clearGeneratedArtifact}
                rows="7"
              ></textarea>
              <small class:limit={excerptBytes > MAX_REPORT_EXCERPT_BYTES}>
                {excerptBytes.toLocaleString("ko-KR")} / {MAX_REPORT_EXCERPT_BYTES.toLocaleString("ko-KR")}바이트
              </small>
              {#if selectedOutputNeedsTruncation}
                <small>
                  원문이 허용 길이를 넘어 앞부분만 넣었습니다. 포함할 내용은 직접
                  편집할 수 있습니다.
                </small>
              {/if}
            </label>
          {/if}
        </div>
      </section>

      {#if formError}
        <p class="form-error">{formError}</p>
      {/if}
      <button
        class="create"
        type="button"
        disabled={formError !== null || creating}
        onclick={createDraft}
        >{creating ? "초안을 만드는 중…" : "검토용 JSON 초안 만들기"}</button
      >
    </form>

    {#if errorMessage}
      <p class="result error" role="alert">{errorMessage}</p>
    {:else if successMessage}
      <p class="result" role="status">{successMessage}</p>
    {/if}

    {#if artifact}
      <section class="artifact" aria-labelledby="artifact-label">
        <h2 id="artifact-label">만들어진 파일</h2>
        <div class="card">
          <p class="filename">{artifact.fileName}</p>
          <p class="filesize">{artifact.byteLength.toLocaleString("ko-KR")}바이트</p>
          <div class="artifact-actions">
            <button
              type="button"
              disabled={actionBusy}
              onclick={exportArtifact}>파일로 내보내기</button
            >
            <button
              type="button"
              disabled={actionBusy}
              onclick={copyArtifact}>JSON 복사</button
            >
          </div>
          <details>
            <summary>JSON 미리보기</summary>
            <pre>{artifact.json}</pre>
          </details>
        </div>
      </section>
    {/if}
  {/if}
</div>

<style>
  .screen {
    height: 100%;
    overflow-y: auto;
    overscroll-behavior: none;
    box-sizing: border-box;
    padding-bottom: calc(var(--sp-5) + var(--safe-bottom));
    background: var(--surface-page);
    color: var(--text-strong);
    font-family: var(--font-ui);
  }

  .back {
    width: var(--size-touch);
    height: var(--size-touch);
    display: inline-flex;
    align-items: center;
    justify-content: center;
    border-radius: var(--r-pill);
    color: var(--text-mid);
  }

  .back:active {
    background: var(--surface-bubble);
  }

  .disclosure,
  .page-status,
  .result,
  .form-error {
    margin: 0;
    padding: var(--sp-2) var(--sp-5);
    font-size: var(--fs-label);
    line-height: 1.5;
    color: var(--text-mid);
  }

  .page-status.recovery {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--sp-3);
  }

  .page-status button,
  .empty a {
    min-height: var(--size-touch);
    padding: 0 var(--sp-3);
    border: 0.5px solid var(--hairline);
    border-radius: var(--r-pill);
    display: inline-flex;
    align-items: center;
    background: var(--surface-card);
    color: var(--text-strong);
    font: inherit;
    text-decoration: none;
  }

  .empty {
    margin: var(--sp-4);
    padding: var(--sp-5);
    border-radius: var(--r-card);
    background: var(--surface-card);
    box-shadow: var(--shadow-card);
    text-align: center;
  }

  .empty p {
    margin: 0 0 var(--sp-3);
    color: var(--text-mid);
  }

  form,
  .artifact {
    display: flex;
    flex-direction: column;
  }

  .group,
  .artifact {
    padding: 0 var(--sp-4);
    margin-top: var(--sp-4);
  }

  .group h2,
  .artifact h2 {
    margin: 0 0 var(--sp-2) var(--sp-2);
    font-size: var(--fs-caption);
    font-weight: 500;
    color: var(--text-mid);
  }

  .card {
    padding: var(--sp-4);
    border-radius: var(--r-card);
    background: var(--surface-card);
    box-shadow: var(--shadow-card);
  }

  .fields {
    display: flex;
    flex-direction: column;
    gap: var(--sp-4);
  }

  label {
    display: flex;
    flex-direction: column;
    gap: var(--sp-2);
    font-size: var(--fs-label);
    color: var(--text-mid);
  }

  label > span {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: var(--sp-2);
  }

  small {
    font-size: var(--fs-caption);
    color: var(--text-faint);
  }

  small.limit,
  .form-error,
  .result.error {
    color: #c62828;
  }

  select,
  textarea {
    width: 100%;
    box-sizing: border-box;
    border: 0.5px solid var(--hairline);
    border-radius: 12px;
    background: var(--surface-page);
    color: var(--text-strong);
    font-family: var(--font-ui);
    font-size: var(--fs-ui);
  }

  select {
    min-height: var(--size-touch);
    padding: 0 var(--sp-3);
  }

  textarea {
    resize: vertical;
    padding: var(--sp-3);
    line-height: 1.5;
  }

  select:focus,
  textarea:focus {
    outline: 2px solid var(--tint-soft);
    outline-offset: 1px;
  }

  .consent {
    min-height: var(--size-touch);
    flex-direction: row;
    align-items: center;
    gap: var(--sp-3);
    color: var(--text-strong);
  }

  .consent input {
    width: 20px;
    height: 20px;
    accent-color: var(--tint);
  }

  .create {
    min-height: 50px;
    margin: var(--sp-4);
    border: none;
    border-radius: var(--r-pill);
    background: var(--tint);
    color: #fff;
    font-family: var(--font-ui);
    font-size: var(--fs-ui);
    font-weight: 600;
    cursor: pointer;
  }

  .create:disabled,
  .artifact-actions button:disabled {
    opacity: 0.45;
    cursor: default;
  }

  .artifact .card {
    display: grid;
    grid-template-columns: 1fr auto;
    gap: var(--sp-1) var(--sp-3);
  }

  .filename,
  .filesize {
    margin: 0;
  }

  .filename {
    font-size: var(--fs-ui);
  }

  .filesize {
    font-size: var(--fs-caption);
    color: var(--text-mid);
  }

  .artifact-actions,
  details {
    grid-column: 1 / -1;
  }

  .artifact-actions {
    display: flex;
    gap: var(--sp-2);
    margin-top: var(--sp-3);
  }

  .artifact-actions button {
    min-height: var(--size-touch);
    flex: 1;
    border: 0.5px solid var(--hairline);
    border-radius: var(--r-pill);
    background: var(--surface-bubble);
    color: var(--text-strong);
    font-family: var(--font-ui);
    font-size: var(--fs-label);
    cursor: pointer;
  }

  details {
    margin-top: var(--sp-3);
    border-top: 0.5px solid var(--hairline);
    padding-top: var(--sp-3);
  }

  summary {
    min-height: var(--size-touch);
    display: flex;
    align-items: center;
    cursor: pointer;
    color: var(--text-mid);
    font-size: var(--fs-label);
  }

  pre {
    max-height: 320px;
    overflow: auto;
    margin: 0;
    padding: var(--sp-3);
    border-radius: 12px;
    background: var(--surface-page);
    font: 11px/1.55 ui-monospace, SFMono-Regular, Menlo, monospace;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }

  @media (min-width: 700px) {
    .disclosure,
    .page-status,
    form,
    .artifact,
    .empty,
    .result,
    .form-error {
      width: min(100% - var(--sp-4) * 2, 680px);
      margin-left: auto;
      margin-right: auto;
      box-sizing: border-box;
    }
  }
</style>
