# LorePia (로어피아) — 기술 계획서 v2

> 크로스플랫폼(Windows / macOS / Linux / iOS / Android) AI 캐릭터 채팅 클라이언트.
> 창작자가 HTML/CSS/JS/Lua로 UI·로어북·미디어 트리거를 직접 설계하고, 그것이 AI 채팅 파이프라인에 네이티브급 성능으로 연동되는 앱.
>
> **데이터 흐름 명시:** LorePia 운영 서버는 존재하지 않으며 콘텐츠를 저장·수집하지 않는다. 단, 사용자가 메시지를 전송하면 조립된 프롬프트(대화·로어북·카드 내용 포함)가 **사용자가 선택·설정한 LLM 제공자에게 전송**된다. 그 외 모든 데이터는 사용자 기기에만 저장된다.

**v1 → v2 변경 요약:** M-1(위험 제거) 신설, 스트리밍을 Tauri Channel 기반으로 재설계, regex 이중 엔진 + 실행 한도, 스키마에 메시지 분기/마이그레이션/모듈 권한 추가, key_enc 모순 제거, 스토어 섹션 재작성(Play AI 신고 기능 포함), 플러그인 격리를 "설계"에서 "실증 대상"으로 격하, 완료 기준을 측정 가능한 수치로 교체.

---

## 0. 핵심 원칙

1. **성능 > 개발 편의.** 무거운 연산은 전부 Rust. 웹뷰는 뷰포트만 그린다.
2. **단일 코드베이스, 5 OS.** 창작자는 OS를 모른다. PC/모바일 반응형 2 브레이크포인트만 고려.
3. **창작자 콘텐츠 = 웹 표준 (HTML/CSS/JS + Lua).** 채팅 서피스는 반드시 웹이어야 한다.
4. **클린룸 아님, 클론 아님.** 기존 코드 0줄. 기능 관찰 + 유저 파일 포맷 분석 기반 신규 설계.
5. **검증 전 확정 금지.** 아래 스택은 M-1 실증을 통과해야 "확정"이 된다. 실증 실패 시 대안 경로가 각 항목에 명시돼 있다.

---

## 1. 기술스택 (M-1 실증 대상 가설)

| 레이어 | 선택 | 실증 항목 (M-1) | 실패 시 대안 |
|---|---|---|---|
| 코어 엔진 | **Rust** (workspace) | 5 OS 크로스컴파일 + FFI | 없음 (전제) |
| 앱 셸 | **Tauri 2** | 모바일 IPC/권한/Channel 동작 | 데스크톱 Tauri + 모바일 별도 셸(코어 재사용) |
| 프론트엔드 | **Svelte 5 + TS + Vite** | 가상 스크롤 1만 msg 60fps | Solid |
| 로컬 DB | **SQLite** (rusqlite) + FTS5 | 5 OS 파일 잠금/동시성/한글 FTS 토크나이저 | — |
| 스크립팅 | **Lua 5.4** (mlua, vendored) | iOS 빌드(인터프리터), 명령 카운트 중단 | 선언형 규칙 엔진으로 축소 |
| 마크다운 | **comrak** | 증분 렌더 성능 | pulldown-cmark |
| 정규식 | **이중 엔진**: 기본 `regex`(선형 보장) + 호환 모드 `fancy-regex`(한도 하) | 백트래킹 폭탄 negative test | 호환 모드 기능 축소 |
| HTML 새니타이즈 | **ammonia** (Rust, 최종 단계 단일 관문) | 새니타이즈 우회 negative test | — |
| LLM 통신 | **reqwest** + SSE → **Tauri `ipc::Channel`** | 배칭/순서/취소/backpressure | — |
| 비밀 저장 | **OS 키체인** (keyring crate) | 5 OS 각각 실증 (특히 Linux 헤드리스) | Linux 한정: 마스터키 암호화 로컬 파일 폴백 |

- DB에는 비밀값을 저장하지 않는다. `providers` 테이블은 키체인 항목의 **참조 ID만** 보유. (v1의 `key_enc` 삭제 — 모순 해소)
- LuaJIT은 M4 이후 데스크톱 한정 성능 옵션으로만 재검토.

---

## 2. 아키텍처

```
┌─────────────────────────────────────────────────────┐
│  WebView (Svelte)                                   │
│  ├─ 앱 셸 UI: 홈/설정 (Tauri IPC 사용)               │
│  ├─ 채팅 서피스: 가상 스크롤 + 캐시 HTML 부착         │
│  └─ 플러그인 iframe: Tauri IPC 권한 0                │
│       └─ postMessage → 호스트 typed broker 만 통과    │
├──── Tauri IPC (commands) / ipc::Channel (stream) ───┤
│  Rust Core (lorepia-core workspace)                 │
│  ├─ engine / prompt / lorebook / script / render    │
│  ├─ providers / storage / importer                  │
│  └─ broker: 플러그인 요청 검증·권한 판정·rate limit    │
└─────────────────────────────────────────────────────┘
```

### 2.1 스트리밍 설계 (v2 재설계)
Tauri event는 저지연 스트리밍용이 아니므로 사용하지 않는다. 토큰 스트림은 `ipc::Channel`로 전달하며:

- **배칭**: 토큰을 16~50ms 윈도우로 묶어 전송 (프레임당 1회 이하 DOM 갱신)
- **순서번호(seq)**: 각 청크에 단조 증가 seq 부여, 프론트는 순서 검증 후 부착
- **취소**: 요청별 CancellationToken. UI 중단 버튼 → command → reqwest abort → Channel 종료 신호
- **backpressure**: 프론트 소비 지연 시 코어가 배칭 윈도우를 자동 확대 (드랍 없음)
- **부분 실패**: 스트림 중단 시 마지막 seq까지를 partial로 저장, `request_state`에 기록 (§4)

### 2.2 플러그인 격리 (설계가 아니라 실증 대상)
전제: **iframe은 인터페이스 경계일 뿐 CPU·보안 경계가 아니다.** 시스템 웹뷰에서 iframe은 부모와 렌더러를 공유할 수 있고, Tauri는 Linux/Android에서 iframe 요청과 상위 창 요청을 권한 계층에서 구분하지 못할 수 있다.

방어 계층 (모두 M-1에서 negative test로 실증):
1. **Tauri IPC 권한 0**: 플러그인 iframe 오리진에는 capability를 일절 부여하지 않는다. 코어 접근은 호스트 JS의 typed broker(postMessage 스키마 검증 + 권한 판정 + rate limit)만 경유
2. **JS 폭주 워치독**: 호스트가 플러그인 iframe에 주기적 ping. 응답 없으면 iframe 강제 리로드/비활성 + 유저 알림. (같은 렌더러 공유 시 완전 방어가 불가능할 수 있음 → M-1에서 OS별 실측)
3. **Lua**: mlua 명령 카운트 훅으로 인스트럭션/메모리 한도, 초과 시 즉시 중단. `os`/`io` 등 위험 stdlib 제거
4. **최종 새니타이즈 단일 관문**: `onRenderMessage` 등 모든 HTML 산출물은 표시 직전 ammonia를 반드시 통과. 훅이 새니타이즈 "이후"에 개입하는 경로를 만들지 않는다 (파이프라인 순서로 강제)
5. **네트워크**: 플러그인 기본 차단. manifest 선언 + 유저 승인 + broker 프록시 경유만 허용

**Store-Safe Profile (폴백, 기본값 아님):** M-1에서 모바일 격리 실증이 실패하는 경우에 한해, 모바일 빌드에서 JS 플러그인을 끄고 선언형 UI(템플릿 + 데이터 바인딩) + Lua(코어 실행이라 격리 가능)만 지원하는 프로파일을 검토한다. 실증을 먼저 하고, 실패 근거가 있을 때만 도입한다.

### 2.3 채팅 파이프라인
```
유저 입력 → [script] onInput → [prompt] 매크로 치환
 → [lorebook] 트리거 스캔·삽입 → [prompt] 어셈블리(토큰 예산 트리밍)
 → [providers] 스트리밍(Channel) → 증분 렌더 → 완료 시
 → [script] regex 스크립트·onOutput·미디어 트리거
 → [render] ammonia 새니타이즈(최종 관문) → HTML 캐시 → [storage] 저장
```
웹뷰 메인스레드에는 DOM 부착·가상 스크롤·iframe 중계 작업이 남는다("항상 한가함" 아님). 무거운 연산이 없도록 유지하는 것이 목표이며, 이는 §3 성능 예산으로 검증한다.

---

## 3. 성능 예산 (측정 가능 수치, CI 회귀 감시)

측정 환경 고정: 저사양 기준기 = Android 보급기(예: Galaxy A2x급) + 5년차 Windows 노트북. 모든 수치는 **p95**.

| 항목 | 기준 | 측정 방법 |
|---|---|---|
| 채팅방 진입 | p95 < 200ms (메시지 5천 개 DB) | 자동화 시나리오 + tracing |
| 스크롤 | 프레임 드랍 < 1% @ 60fps, 1만 msg | DevTools 프레임 로그 |
| 전송 오버헤드 (입력→API 발사) | p95 < 50ms | 코어 tracing |
| 로어북 매칭 (엔트리 1,000개) | p95 < 5ms | criterion 벤치 |
| 스트리밍 중 입력 지연 | p95 < 100ms | 시나리오 테스트 |
| 상주 메모리 (채팅 1개 열림) | < 400MB (모바일) | OS 프로파일러 |
| regex 스크립트 1회 실행 | 한도 10ms, 초과 시 중단·보고 | 코어 타이머 |
| Lua 훅 1회 실행 | 한도 50ms/인스트럭션 캡 | mlua 훅 |

수단(v1 유지): 가상 스크롤(뷰포트 ±버퍼만 DOM), 코어 HTML 사전 렌더+캐시, SQLite 청크 lazy load, content-visibility/containment, transform-only 애니메이션, 정규식 사전 컴파일+키워드 인덱스.

---

## 4. 데이터 모델 v2 (SQLite)

```
schema_meta   (version)                       -- 마이그레이션 버전, 기동 시 순차 적용
characters    (id, name, avatar_asset, spec_json, created_at)
chats         (id, character_id, title, settings_json, active_message_id)
messages      (id, chat_id, parent_id, role, raw_text,
               rendered_html, renderer_ver, tokens, state, created_at)
               -- parent_id로 분기 트리: 리롤 = 같은 parent의 형제.
               -- UI의 ‹2/3› 스와이프 = 형제 간 이동, active_message_id가 현재 경로 결정
               -- state: complete | partial | failed
               -- renderer_ver: 렌더러 갱신 시 캐시 무효화 판단
request_state (id, chat_id, message_id, provider_id, status, last_seq, error, created_at)
lorebook      (id, owner_id, keys, regex, content, position, ord, cond_json, enabled)
variables     (chat_id, key, value)
assets        (id, hash, mime, path)
modules       (id, type, manifest_json, blob, enabled)
module_perms  (module_id, permission, granted_at)   -- 유저 승인 기록
providers     (id, name, base_url, keyring_ref, model, params_json)  -- 비밀값 없음
settings      (key, value)
messages_fts  (FTS5: raw_text)                -- 한글 토크나이저는 M-1에서 결정(trigram 후보)
```
백업/이전: DB + 에셋 폴더 단일 zip export/import. 마이그레이션은 전방향만 지원(버전 다운그레이드 시 안내).

---

## 5. 플러그인 & 카드 API 계약

v1의 산출물 표·훅 8종·브릿지 API·슬롯 4종( status-panel / overlay / message-embed / input-toolbar ) 유지하되:

- **API 고정 시점 변경**: M-1에서 Risu 관찰 노트 + fixture 분석이 끝난 **후에** `specs/plugin-api.md`를 고정한다. (관찰 전 고정 금지)
- 모든 브릿지 호출은 broker에서 권한 판정: manifest 선언 → 설치 시 유저 표시 → `module_perms` 기록 → 호출 시 검증
- `setPanel(html)` 류 UI 주입은 슬롯 iframe 내부로만 렌더 — 앱 DOM 직접 주입 경로 없음
- `onRenderMessage`의 출력은 항상 최종 새니타이즈를 재통과 (§2.2-4)
- regex 스크립트: 기본은 선형 엔진 문법. 호환 모드(fancy-regex)는 카드 임포트 호환용으로만, 실행시간·입력 길이 한도 하에 허용. 한도 초과 패턴은 변환 리포트에 표시

---

## 6. 호환성 레이어 (importer)

- 읽기 1순위: Character Card V2 / V3(charx). PNG 임베드 + zip 아카이브 파싱
- 포맷 리버스는 **유저 파일 구조 분석으로만** (기존 앱 코드 참조 없음)
- **임포터 하드닝 (M-1로 이동):** zip bomb, 경로 탈출(../), 초대형 엔트리, 비정상 PNG 청크, 악성 스크립트 포함 카드에 대한 negative test 통과가 임포터 출시 조건. 임포트 파일은 신뢰하지 않는 입력으로 취급
- 변환 불가 항목은 리포트로 표시. 네이티브 포맷 `*.lorepia` = zip(manifest + assets/ + scripts/), 스펙 공개

---

## 7. 스토어 정책 대응 (v2 재작성 — 단정 대신 대응 설계)

| 리스크 | 현황 인식 | 대응 |
|---|---|---|
| 외부 코드 실행 (Apple 2.5.2) | 시스템 WebKit 내 JS는 허용 범위라는 선례가 있으나, 플러그인 구조에 대한 심사 재량 존재 | 원격 자동 다운로드 없음(수동 임포트만) 유지 + 심사 노트에 구조 설명 첨부. 리젝 시 Store-Safe Profile 폴백 (§2.2) |
| Google Play AI 정책 | **AI 챗봇 앱은 로컬/BYO 여부와 무관하게 앱 내 유해 출력 신고 기능 요구** | 메시지 액션에 "신고" 추가: 로컬 기록 + (동의 시) 개발자 전송. M6 전 구현 |
| UGC 심사 (Apple 1.2) | 공유 서버 없음 = UGC 앱 아님 주장 가능하나 확정 아님 | 앱 내 공유/탐색 기능 미탑재 유지. 차후 공유 기능 추가 시 신고·차단 세트 선행 |
| 연령 등급 | 고정 선택이 아니라 설문 기반 산출, 지역별 상이 | 제출 시점에 설문 기준으로 산출. "무제한 텍스트 생성" 항목 정직하게 응답, 성인 등급 각오 |
| BYO API 키 | 리젝 사유 아님(선례 다수)이나 심사 계정 필요 | 심사용 데모 프로바이더/키 준비 |

원칙: 스토어 섹션의 모든 문장은 "허용된다"가 아니라 "이렇게 대응한다"로 쓴다. 심사 지침은 제출 직전 재확인.

---

## 8. 라이선스 (v1 유지)

Apache-2.0 + NOTICE로 시작. UI 표시 강제는 커스텀 조항(OSI 비인증)이 필요하므로 당장은 커뮤니티 규범으로 유도, 필요 시 듀얼 라이선스 재검토. 외부 GPL 코드 미사용이므로 라이선스 선택 자유.

---

## 9. 저장소 구조 (v1 유지 + 추가)

```
lorepia/
├─ crates/ (core, prompt, lorebook, script, render, providers, storage, importer, broker)
├─ apps/desktop-mobile/ (Tauri 2: src-tauri + Svelte src)
├─ specs/           # 플러그인 API·카드 포맷 스펙 (M-1 이후 고정)
├─ fixtures/        # 호환 테스트 파일 — 자작·허락받은 것·CC 라이선스만 (무단 수록 금지)
├─ spikes/          # M-1 수직 실증 코드 (버려도 되는 코드로 취급)
└─ .github/workflows/  # 5 OS 빌드 + 테스트 + 벤치 회귀 + 모바일 smoke
```
디자인 토큰 파일(`tokens.css`)은 M0에서 확정, 전 화면이 이것만 사용.

---

## 10. 로드맵 v2

**운영 원칙:** 코드베이스는 처음부터 5 OS CI를 유지하되, 기능 개발은 데스크톱 Creator MVP 우선. **모바일(iOS/Android) smoke test는 매 마일스톤 완료 조건에 포함** — 모바일 리스크를 M6까지 미루지 않는다.

| 마일스톤 | 내용 | 완료 기준 (측정 가능) |
|---|---|---|
| **M-1 위험 제거** | ① 5 OS 수직 실증 스파이크: SQLite/FTS5(한글 토크나이저), Lua 한도 중단, 파일 임포트, 키체인, Channel 스트리밍, 오디오 재생 ② negative test: 악성 zip/regex 폭탄/Lua 폭주/JS 폭주/iframe IPC 격리 ③ Risu 기능 관찰 노트 1차 + fixture 확보 + 동작 golden test ④ 성능 기준기·p95 목표 확정 ⑤ 결과로 플러그인 API 고정, Store-Safe Profile 필요 여부 판정 | 실증 매트릭스 5 OS × 6 항목 전부 pass/fail 기록, negative test 전건 방어 확인 또는 완화책 문서화 |
| **M0 스캐폴드** | Tauri 2 + workspace + Svelte + 디자인 토큰 + CI(5 OS 빌드·벤치·모바일 smoke) | 5 OS 빌드 통과, CI에서 벤치 회귀 게이트 동작 |
| **M1 채팅 코어** | providers + Channel 스트리밍 + 키체인 연동 + SQLite(분기 스키마 포함) + 가상 스크롤 | 실 대화 동작, 진입 p95<200ms@5천msg, 스크롤 드랍<1%@1만msg, 취소·partial 저장 동작 |
| **M2 프롬프트 엔진** | 매크로 + 로어북 + 어셈블리 + 이중 regex | 매칭 p95<5ms@1천 엔트리, golden test 통과 |
| **M3 카드 임포트** | V2/V3 읽기 + 네이티브 포맷 + 변환 리포트 (하드닝은 M-1 완료분 사용) | fixture 전건 로드 or 리포트, negative test 재통과 |
| **M4 스크립팅** | Lua 훅 + regex 스크립트 + variables + 실행 한도 | 한도 초과 시 중단·보고 동작, golden test |
| **M5 플러그인 UI** | iframe broker + 슬롯 + 미디어 트리거 + 워치독 | 악성 플러그인 3종(폭주/탈출/권한 초과) 전부 무력화, 상태창 데모 |
| **M6 모바일 마감** | 제스처·IME/키보드·safe area·햅틱·백업·**신고 기능** | 성능 예산 전 항목 모바일 기준기에서 pass |
| **M7 출시** | 코드사인, 심사 대응, 스펙 문서 공개 | App Store + Play 등록 |

---

## 11. 바로 다음 액션

1. `spikes/` M-1 시작 — 우선순위: ① Channel 스트리밍(모바일 포함) ② 모바일 iframe 격리 실측 ③ 키체인 5 OS
2. 네가 준비: Risu 기능 관찰 노트 1차분 + fixture(자작/허락/CC만) → M-1 ③에 투입
3. M-1 결과 리뷰 후 `specs/plugin-api.md` 고정 → M0
