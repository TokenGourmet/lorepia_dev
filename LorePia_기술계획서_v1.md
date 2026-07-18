# LorePia (로어피아) — 기술 계획서 v1

> 크로스플랫폼(Windows / macOS / Linux / iOS / Android) AI 캐릭터 채팅 클라이언트.
> 창작자가 HTML/CSS/JS/Lua로 UI·로어북·미디어 트리거를 직접 설계하고, 그것이 AI 채팅 파이프라인에 네이티브급 성능으로 연동되는 앱.
> 서버에 콘텐츠를 저장·공유하지 않는 **로컬 우선(local-first) 클라이언트**. 유저는 파일을 직접 임포트한다.

---

## 0. 핵심 원칙 (모든 결정의 기준)

1. **성능 > 개발 편의.** 0.1초 싸움. 무거운 연산은 전부 Rust, 웹뷰는 뷰포트만 그린다.
2. **단일 코드베이스, 5 OS.** 창작자는 OS를 모른다. PC/모바일 반응형만 고려.
3. **창작자 콘텐츠 = 웹 표준.** HTML/CSS/JS + Lua. 채팅 서피스는 반드시 웹이어야 한다.
4. **클린룸 아님, 클론 아님.** 기존 코드 0줄 사용. 기능은 관찰·명세 기반으로 새로 설계.
5. **로컬 전용.** UGC 서버 없음 → 스토어 모더레이션 의무 최소화, 유저 데이터는 유저 기기에.

---

## 1. 기술스택 (확정)

| 레이어 | 선택 | 이유 |
|---|---|---|
| 코어 엔진 | **Rust** (workspace, 멀티 크레이트) | 파싱/트리거/DB/스트리밍의 성능·안정성. 5 OS 전부 크로스컴파일 |
| 앱 셸 | **Tauri 2** | 단일 코드베이스로 데스크톱 3 + 모바일 2. 경량 번들. 시스템 웹뷰 |
| 프론트엔드 | **Svelte 5 + TypeScript + Vite** | 컴파일 타임 반응성 → 런타임 오버헤드 최소. 가상 스크롤 궁합 좋음 |
| 로컬 DB | **SQLite** (rusqlite) + **FTS5** | 단일 파일, 임베디드, 전문검색. 대화 10만+ 스케일 검증됨 |
| 스크립팅 | **Lua 5.4** (mlua, vendored) | iOS JIT 금지 제약 회피(인터프리터), 샌드박스 쉬움, 카드 생태계 표준 |
| 마크다운 | **comrak** (Rust) | GFM 호환, 코어에서 파싱 → HTML 캐시 |
| 정규식 | **fancy-regex** (Rust) | 창작자 regex 스크립트는 JS 플레이버(lookbehind/backreference) 필요 |
| HTML 새니타이즈 | **ammonia** (Rust) | 창작자 HTML 렌더 전 XSS 방어 |
| LLM 통신 | **reqwest** + SSE 스트리밍 (Rust) | API 키가 웹뷰에 노출되지 않음. 스트림은 Tauri 이벤트로 프론트에 전달 |
| 상태관리 | Svelte runes + 코어가 단일 진실 소스 | UI 상태 최소화. 데이터는 항상 Rust 코어에 |

### 보류/차후 결정
- LuaJIT 도입 여부 (데스크톱 한정 성능 옵션) — M4에서 벤치마크 후
- 데스크톱용 임베디드 웹뷰 엔진 통일(예: CEF) — 시스템 웹뷰 편차가 실제 문제 될 때만 재검토

---

## 2. 전체 아키텍처

```
┌─────────────────────────────────────────────────────┐
│  WebView (Svelte)                                   │
│  ├─ 앱 셸 UI: 내비/설정/캐릭터 목록                    │
│  ├─ 채팅 서피스: 가상 스크롤 + 캐시된 HTML 부착        │
│  └─ 플러그인 샌드박스: iframe + postMessage 브릿지    │
├──────────────── Tauri IPC / Events ─────────────────┤
│  Rust Core (lorepia-core workspace)                 │
│  ├─ engine    : 채팅 파이프라인 오케스트레이션          │
│  ├─ prompt    : 매크로 치환, 프롬프트 어셈블리          │
│  ├─ lorebook  : 트리거 매칭 엔진 (키워드/정규식/조건)   │
│  ├─ script    : Lua VM 풀 + regex 스크립트 실행        │
│  ├─ render    : 마크다운→HTML 변환 + 새니타이즈 + 캐시  │
│  ├─ providers : LLM API 어댑터 (OpenAI 호환/Claude/…)  │
│  ├─ storage   : SQLite, 에셋(이미지/오디오) 관리       │
│  └─ importer  : 외부 카드/모듈 포맷 변환 (PC 우선)      │
└─────────────────────────────────────────────────────┘
```

### 채팅 파이프라인 (요청 1회 흐름)
```
유저 입력
 → [script] 입력 훅 (Lua onInput)
 → [prompt] 매크로 치환 ({{char}}, {{user}}, 변수 등)
 → [lorebook] 트리거 스캔 → 활성 엔트리 선별 → 삽입 위치/순서 결정
 → [prompt] 최종 프롬프트 어셈블리 (토큰 예산 내 트리밍)
 → [providers] 스트리밍 요청
 → 토큰 청크 → [render] 증분 마크다운 렌더 → 이벤트로 웹뷰 전달
 → 완료 시 [script] 출력 훅 (regex 스크립트, Lua onOutput, 미디어 트리거)
 → [render] 최종 HTML 캐시 → [storage] 저장
```

핵심: **웹뷰는 이 파이프라인에 관여하지 않는다.** 결과만 받아 그린다. 메인스레드가 항상 한가함 → 제스처/스크롤 끊김 없음.

---

## 3. 성능 설계 (0.1초를 지키는 구체 수단)

| 항목 | 수단 | 목표 |
|---|---|---|
| 대화 스크롤 | 가상 스크롤(뷰포트 ±버퍼 30~50개만 DOM 존재) | 대화 10만 개에서도 60fps |
| 메시지 렌더 | 코어에서 HTML 사전 렌더 + 캐시(재변환 금지) | 캐시 히트 시 부착만, <1ms |
| 히스토리 로드 | SQLite 청크 lazy load, 전체 메모리 적재 금지 | 채팅방 진입 <150ms |
| 화면 밖 비용 | `content-visibility: auto`, CSS containment | 레이아웃 계산 차단 |
| 애니메이션 | transform/opacity만, passive listener, `touch-action` | 컴포지터 스레드 전담 |
| 로어북 매칭 | Rust 정규식 사전 컴파일 + 키워드 인덱스 | 엔트리 1,000개 스캔 <5ms |
| 플러그인 폭주 | iframe 격리 + Lua 명령 카운트 제한 | 무한루프여도 채팅 생존 |
| 전송 오버헤드 | 입력→API 발사까지 파이프라인 전체 | <50ms |

성능 예산은 CI에서 벤치마크로 회귀 감시 (criterion + 프론트 Lighthouse 스크립트).

---

## 4. 데이터 모델 (SQLite 스키마 개요)

```
characters   (id, name, avatar_asset, spec_json, created_at, …)
chats        (id, character_id, title, settings_json, …)
messages     (id, chat_id, role, raw_text, rendered_html, tokens, created_at)
lorebook     (id, owner_id, keys, regex, content, position, order, cond_json, enabled)
variables    (chat_id, key, value)            -- 상태창/스크립트용 chat state
assets       (id, hash, mime, path)           -- 이미지/오디오, 콘텐츠 해시 중복제거
modules      (id, type, manifest_json, blob)  -- 플러그인/프리셋/regex 스크립트
providers    (id, name, base_url, key_enc, model, params_json)
settings     (key, value)
messages_fts (FTS5, raw_text)                 -- 전문검색
```

- API 키는 OS 키체인(keyring crate)에 암호화 저장, DB에는 참조만.
- 에셋은 파일시스템 + 해시 참조(카드에 내장된 이미지 중복 제거).
- 백업/이전: DB + 에셋 폴더 통째 export/import (단일 zip).

---

## 5. 플러그인 & 카드 API 계약 (제품의 심장)

### 5.1 창작자가 만들 수 있는 것
| 산출물 | 기술 | 실행 위치 |
|---|---|---|
| 커스텀 UI (상태창/맵/버튼/오버레이) | HTML/CSS/JS | 샌드박스 iframe |
| 메시지 스타일링 | CSS (스코프 적용) | 채팅 서피스 |
| 텍스트 변환 (regex 스크립트) | 정규식 find/replace | Rust 코어 |
| 로직/상태 (변수, 조건, 프롬프트 개입) | Lua | Rust 코어 (mlua) |
| 로어북 | 선언적 JSON (키워드/정규식/조건/위치/순서) | Rust 코어 |
| 미디어 트리거 (조건부 이미지/음악) | 선언적 규칙 + Lua | 코어 판정 → UI 재생 |

### 5.2 이벤트 훅 (Lua / JS 공통 개념)
```
onChatLoad(chat)                 -- 채팅방 진입
onInput(text) -> text            -- 전송 전 입력 가공
onPromptBuild(blocks) -> blocks  -- 프롬프트 블록 삽입/제거/재배열
onStreamToken(token)             -- 스트리밍 중 (UI 반응용, 가공 불가)
onOutput(text) -> text           -- 응답 완료 후 가공
onRenderMessage(html) -> html    -- 표시 직전 (regex 스크립트가 여기)
onVariableChange(key, value)     -- 상태창 갱신 트리거
onTrigger(event)                 -- 로어북/미디어 트리거 발동 알림
```

### 5.3 UI 플러그인 브릿지 API (postMessage, capability 기반)
```
lorepia.getVariables() / setVariable(key, value)
lorepia.getMessages(range) / getLastMessage()
lorepia.sendAsUser(text) / insertPrompt(block)
lorepia.playAudio(assetId) / showImage(assetId)
lorepia.ui.setPanel(html) / setOverlay(html)
lorepia.on(event, handler)
```
- 플러그인 manifest에 요구 권한 선언 → 설치 시 유저에게 표시 (예: "채팅 내용 읽기").
- 네트워크 접근은 기본 차단, manifest 선언 + 유저 승인 시만 허용.
- iframe은 `sandbox` 속성 + CSP로 격리. 앱 전역 DOM 접근 불가, 브릿지로만 소통.

### 5.4 UI 슬롯 (창작자 UI가 붙는 자리)
```
slot:status-panel   -- 채팅 상단/사이드 상태창
slot:overlay        -- 채팅 위 오버레이 (맵, 연출)
slot:message-embed  -- 특정 메시지에 부착되는 위젯
slot:input-toolbar  -- 입력창 옆 커스텀 버튼
```
반응형 규칙: 슬롯은 PC/모바일 두 브레이크포인트만 보장. 창작자는 그 안에서 CSS로 대응.

---

## 6. 호환성 레이어 (importer)

- **읽기 지원 (1순위):** Character Card V2 / V3(charx) — 생태계 표준. PNG 임베드 + charx 아카이브 파싱.
- **변환기 (PC앱 내장):** 기존 카드/모듈/프리셋/regex 스크립트 파일 → LorePia 네이티브 포맷 변환. 샘플 파일 기반으로 포맷 리버스는 **파일 구조 분석**으로만 진행 (코드 참조 없음).
- 변환 불가 항목은 리포트로 표시 ("이 카드의 X 기능은 미지원").
- 네이티브 포맷: `*.lorepia` = zip(manifest.json + assets/ + scripts/). 스펙 문서 공개 → 서드파티 도구 허용.

---

## 7. 스토어 정책 대응

| 리스크 | 대응 |
|---|---|
| 실행 코드 다운로드 (Apple 2.5.2) | JS는 시스템 WebKit에서만 실행(허용 범위), Lua는 인터프리터·앱 핵심기능 변경 불가 구조. **앱이 원격에서 코드를 자동 다운로드하지 않음** — 유저가 파일을 수동 임포트 |
| UGC 모더레이션 (Apple 1.2) | 앱 내 공유/탐색/커뮤니티 기능 없음 = UGC 앱 아님. 브라우저/파일뷰어와 동일 포지션. (차후 공유 기능 추가 시 신고·차단 세트로 재대응) |
| 성인 콘텐츠 | 앱 자체는 콘텐츠 미포함. 연령 등급은 "무제한 웹 접근/사용자 생성 텍스트" 기준으로 17+/성인 등급 신청 |
| API 키 요구 | BYO-key 구조는 심사 시 리젝 사유 아님(선례 다수). 심사용 데모 키/모드 준비 |

---

## 8. 라이선스

의도: 코드는 자유 사용, 단 **이 코드를 쓴 앱에 저작 표기가 남아야 함.**

- 표준 라이선스 중 최근접: **Apache-2.0** — 재배포 시 LICENSE·NOTICE 보존 의무. 단, "앱 UI에 표시" 까지는 강제 안 됨.
- UI 표시까지 강제하려면 커스텀 조항 추가 필요 (구 BSD 광고조항 계열) → OSI 비인증이 되고 오픈소스 생태계 편입에 불리.
- **권고: Apache-2.0 + NOTICE 파일**로 시작하고, UI 크레딧은 커뮤니티 규범으로 유도. 강제 원하면 그때 커스텀 듀얼 라이선스 검토.

---

## 9. 저장소 구조

```
lorepia/
├─ crates/
│  ├─ lorepia-core/       # 파이프라인 오케스트레이션
│  ├─ lorepia-prompt/     # 매크로/어셈블리
│  ├─ lorepia-lorebook/   # 트리거 엔진
│  ├─ lorepia-script/     # Lua VM, regex 스크립트
│  ├─ lorepia-render/     # md→html, sanitize, cache
│  ├─ lorepia-providers/  # LLM 어댑터
│  ├─ lorepia-storage/    # SQLite, assets
│  └─ lorepia-importer/   # 외부 포맷 변환
├─ apps/
│  └─ desktop-mobile/     # Tauri 2 프로젝트 (5 OS 단일)
│     ├─ src-tauri/       # Rust 바인딩, commands, events
│     └─ src/             # Svelte 프론트
├─ specs/                 # 카드 포맷/플러그인 API 스펙 문서 (공개)
├─ fixtures/              # 샘플 카드/모듈 (호환 테스트용)
└─ .github/workflows/     # CI: 테스트 + 벤치 회귀 + 5 OS 빌드
```

개발 규범: Contract-First (스펙 → 테스트 → 구현), 체크포인트 커밋, clippy/eslint 자동화, 기능 플래그.

---

## 10. 로드맵

| 마일스톤 | 내용 | 완료 기준 |
|---|---|---|
| **M0 스캐폴드** | Tauri 2 + Rust workspace + Svelte 셋업, 5 OS 빌드 CI | 5개 플랫폼 "Hello" 빌드 통과 |
| **M1 채팅 코어** | providers + 스트리밍 + SQLite + 가상 스크롤 채팅 UI | BYO 키로 실제 대화, 1만 메시지 60fps |
| **M2 프롬프트 엔진** | 매크로 치환 + 로어북 트리거 + 프롬프트 어셈블리 | 로어북 조건 삽입 동작, 매칭 <5ms |
| **M3 카드 임포트** | V2/V3 카드 읽기 + 네이티브 포맷 + 변환 리포트 | 샘플 카드 정상 로드 |
| **M4 스크립팅** | Lua 훅 + regex 스크립트 + variables | 상태값 기반 프롬프트 개입 동작 |
| **M5 플러그인 UI** | iframe 샌드박스 + 브릿지 API + 슬롯 + 미디어 트리거 | 창작자 상태창/음악 트리거 데모 |
| **M6 모바일 마감** | iOS/Android 제스처·레이아웃 폴리시, 키체인, 백업 | 폰에서 데스크톱과 동일 기능 |
| **M7 출시** | 코드사인, 스토어 심사 대응, 문서/스펙 공개 | App Store + Play 등록 |

각 마일스톤 시작 시 해당 기능의 **Risu 기능 명세**(네가 만져보고 정리한 것 + 샘플 파일)를 인풋으로 받아 스펙 확정 → 구현.

---

## 11. 바로 다음 액션

1. `specs/plugin-api.md` 초안 — 5장 훅/브릿지를 정식 스펙으로 (Contract-First 시작점)
2. M0 스캐폴드 생성 (repo 초기화, CI 매트릭스)
3. 네가 준비할 것: Risu 기능 관찰 노트 1차분 + 샘플 카드/모듈/프리셋 파일 → `fixtures/`행
