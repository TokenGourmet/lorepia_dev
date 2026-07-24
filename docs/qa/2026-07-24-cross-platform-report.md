# LorePia 교차 플랫폼 기능·UI QA 보고서

> 이 문서는 수정 전 결함 기준선이다. 이후 수정과 재검증 결과는
> [2026-07-24-cross-platform-fix-verification.md](2026-07-24-cross-platform-fix-verification.md)를 참고한다.

- 기준일: 2026-07-24
- 대상: `main`의 `8106a1d` 위 현재 미커밋 작업 트리
- 테스트 계획: [2026-07-24-cross-platform-checklist.md](2026-07-24-cross-platform-checklist.md)
- 증거: [evidence/2026-07-24-cross-platform/](evidence/2026-07-24-cross-platform/)
- 상태: `PASS`, `FAIL`, `BLOCKED`, `NOT RUN`

체크리스트는 실행 전에 고정한 합격 기준 문서다. 체크리스트 자체의
`PENDING` 표기는 바꾸지 않았고, 실제 결과는 이 보고서에 별도로 기록했다.

## 결론

현재 작업 트리를 교차 플랫폼 합격으로 판정할 수 없다.

- Android는 소프트웨어 키보드가 채팅 입력창을 완전히 가리고, 상단 조작부가
  상태바와 겹친다. 두 문제 모두 주 사용 흐름을 막는 `P1`이다.
- iOS는 현재 아카이브의 앱을 수동 설치하면 주요 화면이 동작하지만, 표준
  Tauri 빌드 명령이 패키징 마지막 단계에서 실패한다.
- macOS debug 앱은 검색, 키보드 탐색, 채팅 차단 이유, 모달, 스크롤,
  시스템 공유까지 직접 동작했다. 다만 공통 서재 데이터·hit area 문제와
  AppKit fault 로그가 남아 있어 완전 합격은 아니다.
- 사전에 정의한 `AUTO-01`~`AUTO-06`은 모두 통과했다. 그러나 이 묶음은
  키보드, 안전영역, 실제 시스템 제스처, 가짜 서재 미리보기를 잡지 못했고,
  별도 `git diff --check`는 실패했다.

따라서 현 단계 판정은 **기능 데모 가능, 교차 플랫폼 출하 불가**다.

## 테스트 환경과 산출물

| 플랫폼 | 환경 | 빌드 판정 | 직접 UI 판정 |
| --- | --- | --- | --- |
| iOS | iPhone 17 Simulator, iOS 26.5, Xcode 26.6 | `FAIL` — 컴파일·아카이브 후 앱 rename에서 종료 코드 1 | 현재 아카이브 앱 수동 설치 후 주요 기능 동작 |
| Android | `sdk_gphone64_arm64`, Android 16, 1080×2340 @ 420 dpi | `BLOCKED` — AArch64 Rust 산출물은 컴파일됐으나 JRE 부재로 Gradle APK 조립 불가 | 기존 설치 debug APK에서 직접 테스트. 현재 작업 트리와 정확히 같은 APK인지는 보증 불가 |
| macOS | macOS 26.5.2 (25F84) | `PASS` — debug `.app` 생성 | 현재 debug 앱에서 직접 테스트 |

### 빌드 세부 결과

- iOS: `npm run tauri -- ios build --debug --target aarch64-sim --no-sign`
  실행 시 Xcode 단계와 `.xcarchive` 내 `LorePia.app` 생성까지 진행됐으나,
  마지막 rename이 `Directory not empty (os error 66)`으로 실패했다.
  최종 target 경로에 이전 앱이 남을 수 있어 잘못된 산출물을 설치할 위험도
  확인했다. 테스트에는 현재 `.xcarchive` 안의 앱을 직접 설치했다.
- Android: 명시적으로 `ANDROID_HOME`과 `NDK_HOME`을 지정하자 현재 Rust
  네이티브 라이브러리는 정상 컴파일됐다. 이후 `gradlew` 실행 시 로컬
  Java Runtime이 없어 APK 조립이 중단됐다. 이는 이번 환경의 차단이며,
  앱 소스 컴파일 오류로 판정하지 않았다.
  [Android 빌드 요약](evidence/2026-07-24-cross-platform/android-build-summary.txt)
- macOS: `npm run tauri -- build --debug --bundles app`이 성공했고
  `target/debug/bundle/macos/LorePia.app`을 직접 실행했다.

## 확정 결함과 차단 관찰

Android 런타임 결과는 기존 설치 debug APK에서 얻었다. 화면과 DOM이 현재
공통 소스 구현과 일치하는 것은 확인했지만, JRE 부재로 현재 작업 트리의
APK를 새로 조립하지 못했으므로 정확히 같은 바이너리라고 보증하지 않는다.

### F-01 · P1 · Android 채팅 입력창이 IME 뒤로 완전히 숨음

- 체크리스트: `CHAT-06`, `AND-03`
- 재현: 카이 채팅 진입 → 입력창 선택 → 소프트웨어 키보드 표시
- 기대: composer와 전송 상태가 키보드 위에 유지
- 실제: composer가 CSS y=828~872에 남고 IME가 그 영역을 모두 덮는다.
  전송 불가 이유도 키보드 뒤에 표시된다.
- 근거: [화면](evidence/2026-07-24-cross-platform/android-12-chat-ime.png),
  [DOM 위치](evidence/2026-07-24-cross-platform/android-12-chat-ime-dom.json),
  [시스템 inset](evidence/2026-07-24-cross-platform/android-12-chat-insets.txt)
- 관련 구현: `keyboard-inset.svelte.ts`는 `visualViewport`에 의존하고,
  Android `MainActivity`는 system bar/cutout만 반영한다.

### F-02 · P1 · Android 상단 조작부가 상태바와 겹침

- 체크리스트: `NAV-08`, `CHAT-13`
- 재현: 서재 검색 또는 채팅 상단바 표시
- 실제: 검색 버튼은 CSS y=5~49에 있고 시스템 상태바는 물리 y=0~63을
  차지한다. 채팅 뒤로가기와 정체성 헤더도 같은 상단 inset 문제를 공유한다.
- 근거: [검색 화면](evidence/2026-07-24-cross-platform/android-08-search-open.png),
  [검색 rect](evidence/2026-07-24-cross-platform/android-59-search-safe-area-rect.json),
  [시스템 inset](evidence/2026-07-24-cross-platform/android-59-system-insets.txt),
  [채팅 화면](evidence/2026-07-24-cross-platform/android-11-kai-chat.png)

### F-03 · P1 · iOS 표준 빌드가 패키징 마지막 단계에서 실패

- 체크리스트: `ENV-01`
- 실제 오류: `failed to rename app ... Directory not empty (os error 66)`
- 영향: 컴파일 결과는 생기지만 명령은 실패하고, 최종 앱 경로에는 이전
  산출물이 남을 수 있다. 자동 설치가 성공한 것처럼 오인할 위험이 있다.
- 근거: [iOS 빌드 로그](evidence/2026-07-24-cross-platform/ios-build.log)

### F-04 · P2 · 공통 서재가 샘플 문장·시간을 실제 대화 이력처럼 표시

- 체크리스트: `LIB-02`
- 실제: 저장 채팅이 없는 캐릭터도 `sample.ts`의 고정 `lastMessage`와
  `lastAt`을 최근 대화처럼 표시한다. 캐릭터 채팅을 한 번 열어 빈 채팅이
  생성되면 그 행만 “아직 대화가 없습니다.”로 바뀌어 앞뒤가 모순된다.
- 적용: iOS, Android, macOS 공통
- 근거: [iOS 서재](evidence/2026-07-24-cross-platform/ios-final-library-restored.jpeg),
  [Android 서재](evidence/2026-07-24-cross-platform/android-60-final-library.png),
  [macOS 서재](evidence/2026-07-24-cross-platform/macos-final-library-restored.jpeg),
  [Android 접근성 덤프](evidence/2026-07-24-cross-platform/android-51-library-accessibility.json)

### F-05 · P2 · 공통 주요 조작 영역이 44pt보다 작음

- 체크리스트: `NAV-07`
- 실제 DOM hit rect:
  - 설정 테마·모드 segment: 30px
  - 채팅 전송 버튼: 34×34px
  - 삭제 모달의 취소·삭제: 36px
  - 신고 CTA: 36px, 오류 재시도: 32px
- 30~36px의 시각 크기는 유지할 수 있지만, 별도의 44pt hit wrapper나
  가상 hit 영역이 없다.
- Android에서 실측했고 iOS·macOS도 같은 공통 CSS를 사용한다. 출하 차단
  영향은 터치 플랫폼인 iOS와 Android에서 더 크다.
- 근거: [설정 rect](evidence/2026-07-24-cross-platform/android-32-account-top-dom.json),
  [삭제 모달 rect](evidence/2026-07-24-cross-platform/android-19-delete-modal-dom.json),
  [composer rect](evidence/2026-07-24-cross-platform/android-12-chat-ime-dom.json)

### F-06 · P2 후보 · Android 진단 파일 전달 결과 미확인

- 체크리스트: `SET-12`
- 상태: `BLOCKED`
- 실제: “파일 저장을 요청했습니다.”가 표시됐지만 공유 시트가 열리지 않고
  파일도 생성되지 않았다. 앱 포커스도 그대로였다. 이 에뮬레이터에는
  Download 서비스가 없어 환경 제한과 앱 결함을 완전히 분리할 수 없다.
- 구현 위험: Android WebView에서 `<a download>.click()`을 실행한 뒤
  실제 저장 여부를 확인하지 않고 `downloaded`를 반환한다.
- 근거: [실행 화면](evidence/2026-07-24-cross-platform/android-43-diagnostics-share.png),
  [상태](evidence/2026-07-24-cross-platform/android-43-diagnostics-share-status.json),
  [파일 확인](evidence/2026-07-24-cross-platform/android-43-diagnostics-download-files.txt)
- 비교: iOS Activity View와 macOS 시스템 공유 popover는 직접 열리고
  취소됐다.
  [iOS 공유](evidence/2026-07-24-cross-platform/ios-share-sheet.jpeg),
  [macOS 공유](evidence/2026-07-24-cross-platform/macos-share-popover.jpeg)

### F-07 · P2 · Android 실제 시스템 에지에서 화면 추종 애니메이션 없음

- 체크리스트: `AND-02`
- 화면 안쪽에서 시작한 자체 drag는 화면·헤더·뒤로가기 버튼이 함께
  이동하고 취소도 된다.
- 실제 왼쪽 OS edge에서 시작하면 시스템 화살표만 보이고 앱 화면의
  translate는 0이다. 손을 놓은 뒤에만 서재로 이동한다.
- 근거: [실제 edge 중간](evidence/2026-07-24-cross-platform/android-31-edge-motionevent-mid.png),
  [edge 상태](evidence/2026-07-24-cross-platform/android-31-edge-motionevent-mid.json),
  [자체 drag 비교](evidence/2026-07-24-cross-platform/android-30-interactive-back-mid.png)

### F-08 · P2 · iOS 기존 Vertex 설정이 비활성 선택 상태로 남음

- 체크리스트: `SET-04`, `CHAT-07`
- 기존 iOS 데이터에서 현재 `configuration-only`인 Vertex AI가 선택된
  상태로 복원됐다. radio는 비활성인데 현재 제공자로 표시되고 채팅 전송을
  막는다. 다른 지원 제공자를 고르면 벗어날 수 있지만, 마이그레이션 후
  첫 화면 상태가 모순적이다.
- 근거: [계정 화면](evidence/2026-07-24-cross-platform/ios-account-current.jpeg),
  [전송 차단 이유](evidence/2026-07-24-cross-platform/ios-send-blocked-reason.jpeg)

### F-09 · P3 · Android 하단 탭 긴 누름에 내부 URL 노출

- 체크리스트: `AND-04`
- 짧은 탭의 진회색 플래시는 재현되지 않았다. 다만 약 0.5초 누르면
  `http://tauri.localhost/home`이 WebView 툴팁으로 나타난다.
- 근거: [긴 누름](evidence/2026-07-24-cross-platform/android-49-dock-touch-down.png)

### F-10 · P3 · 작업 트리 whitespace 검사 실패

- 체크리스트 외 저장소 위생 검사
- `git diff --check`가
  `crates/tauri-plugin-native-back/.tauri/tauri-api/Sources/Tauri/Logger.swift`
  16, 20, 26, 31, 37, 44, 54, 59, 65, 77, 78, 82행의 trailing
  whitespace를 보고한다.
- 제품 실행 결함은 아니지만 이 상태 그대로는 깨끗한 패치가 아니다.
- 근거: [git diff 검사 로그](evidence/2026-07-24-cross-platform/git-diff-check.log)

## 직접 UI와 소스 판정 요약

표의 `(source)`는 해당 플랫폼에서 hit rect를 다시 실측했다는 뜻이 아니라,
Android에서 실측한 것과 같은 공통 CSS 경로를 사용한다는 소스 판정이다.

### 공통 기능

| 영역 | iOS | Android | macOS |
| --- | --- | --- | --- |
| 루트 셸·하단바 | `PASS` — 홈·서재·생성·계정, 백그라운드 복귀 | `PASS` — 4탭, 재실행·복귀 | `PASS` — 서재·계정 및 좁은/넓은 창. 4탭 전체 순회는 `NOT RUN` |
| 서재 행·검색 | `PASS` — 검색 열기·입력·닫기 | `PASS` — 72px 행, 48px 아바타, 76px 시작 구분선 실측 | `PASS` — 검색, Escape, 끝까지 스크롤 |
| 서재 내용 정직성 | `FAIL` — 샘플 최근 대화 | `FAIL` — 샘플 최근 대화 | `FAIL` — 샘플 최근 대화 |
| 채팅 빈 상태·차단 이유 | `PASS` | `PASS` | `PASS` |
| 소프트웨어 키보드 | `PASS` — composer가 키보드 위에 유지 | `FAIL` — composer 완전 가림 | `PASS` — 입력·focus-visible |
| 정보·삭제 모달 | `PASS` — 취소까지, 실제 삭제 제외 | `PASS` — 취소까지, 실제 삭제 제외 | `PASS` — Escape 취소까지, 실제 삭제 제외 |
| 신고 빈 상태 | `PASS` | `PASS` | `PASS` |
| 테마 | `PASS` — dark 확인 후 system 복원 | `PASS` — light 재실행 유지 후 system 복원 | `PASS` — dark 확인 후 원래 light 복원 |
| 진단 초안·JSON 검토·복사 | `PASS` | `PASS` | `PASS` |
| 진단 공유·저장 | `PASS` — Activity View | `BLOCKED` — 결과 없음, 에뮬레이터 Download 서비스 부재 | `PASS` — 시스템 공유 popover 및 취소 |
| 작은 hit area | `FAIL` (source) — Android와 같은 공통 CSS | `FAIL` (direct UI) — 런타임 실측 | `FAIL` (source) — 체크리스트의 44pt 기준상 공통 CSS |

### 플랫폼 상호작용

| ID | 상태 | 판정 |
| --- | --- | --- |
| `IOS-01` | `NOT RUN` | Simulator의 마우스·자동화 입력으로 네이티브 edge cancel/complete 추종을 증명하지 못했다. 실패나 통과로 단정하지 않는다. |
| `IOS-02` | `PASS` | 소프트웨어 키보드를 직접 표시했고 composer가 홈 인디케이터와 키보드 위에 유지됐다. |
| `IOS-03` | `NOT RUN` | 실터치 장기 누름·확대경·선택 억제는 시뮬레이터 마우스로 보증할 수 없다. |
| `IOS-04` | `PASS` | 시스템 Activity View가 열리고 취소됐다. |
| `AND-01` | `PASS` | 시스템 back이 신고→설정→채팅→서재를 한 단계씩 닫았다. |
| `AND-02` | `FAIL` | 실제 OS edge에서는 앱 화면 추종이 없다. |
| `AND-03` | `FAIL` | IME inset 처리 실패로 composer가 가려진다. |
| `AND-04` | `FAIL` | 짧은 탭 flash는 없었으나 긴 누름에 내부 URL이 노출됐다. |
| `MAC-01` | `PASS` | 약 360px 좁은 창과 넓은 창에서 레이아웃·최대 폭을 확인했다. |
| `MAC-02` | `PASS` | Tab, Enter, Escape, focus-visible이 동작했다. |
| `MAC-03` | `PASS` | 계정과 서재를 휠 스크롤했고 마지막 행이 dock 위까지 올라왔다. |
| `MAC-04` | `NOT RUN` | 지원되는 macOS 공유 popover와 취소는 `SET-12`로 통과했다. 이 ID가 요구하는 “Web Share 미지원 시 파일 저장 또는 복사 폴백” 환경은 만들지 못했다. |

## 실행하지 않았거나 외부 조건으로 막힌 항목

아래 항목은 성공으로 추정하지 않았다.

- `CHAT-02`~`CHAT-05`: 저장 메시지와 200개 이상 대화의 실제 UI 복원,
  prepend anchor, 종료·재시도. 관련 단위 테스트만 통과했다.
- `CHAT-08`: 빈 입력 거부는 확인했지만 과도한 장문 입력은 직접 실행하지 않았다.
- `CHAT-09`: 네이티브 캐릭터 ID·페르소나 경계는 Rust 테스트로 통과했지만
  실제 제공자 응답으로 검증하지 않았다.
- `CHAT-10`: 실제 제공자 스트리밍, 취소, 실패, 중복 저장은
  `BLOCKED`다.
- `INFO-06`: 실제 삭제는 사용자 데이터를 보호하기 위해 실행하지 않았다.
- `INFO-08`, `INFO-09`: 저장된 실제 AI 응답이 없어 신고 초안·전달은
  `BLOCKED`다.
- `SET-05`: 모델 ID의 저장·재실행 복원은 실행하지 않았다.
- `SET-06`: 실제 자격증명 저장과 재시도는 `BLOCKED`다.
- `SET-07`: 저장 revision 충돌의 UI 복구는 실행하지 않았다.
- `REG-02`, `REG-03`: 모든 플랫폼의 360~375px 조합, 긴 캐릭터명·모델명·
  오류 문자열은 완전 실행하지 않았다.
- `REG-06`: 비식별 진단 구조와 자동 경계 테스트는 통과했지만, 실제 secret과
  실제 provider 원시 오류를 넣는 E2E는 `BLOCKED`다.
- `MAC-04`: macOS에서 Web Share가 미지원인 조건의 파일 저장·복사 폴백은
  만들지 못했다. 시스템 공유 popover 자체는 `SET-12`로 통과했다.
- VoiceOver·TalkBack 전체 흐름, 회전, 프로세스 강제 종료 후 복원,
  오프라인, 8시간·72시간 soak, 실기기, 서명 release 빌드는 `NOT RUN`이다.

## 자동 검증

| ID | 결과 |
| --- | --- |
| `AUTO-01` | `PASS` — `svelte-check` 0 errors, 0 warnings |
| `AUTO-02` | `PASS` — Vitest 31 files / 226 tests, Node boundary 33 tests |
| `AUTO-03` | `PASS` — 정적 빌드 및 55개 built-boundary 파일 검증 |
| `AUTO-04` | `PASS` — `cargo fmt --all -- --check` |
| `AUTO-05` | `PASS` — Clippy warnings 0 |
| `AUTO-06` | `PASS` — `provider_stream::tests::` 46 passed |

근거:
[frontend 자동 검증 로그](evidence/2026-07-24-cross-platform/automated-frontend.log),
[Rust 자동 검증 로그](evidence/2026-07-24-cross-platform/automated-rust.log)

## 로그 관찰

현재 화면 결함으로 단정하지 않았지만 다음 항목은 후속 재현이 필요하다.

- macOS: `Invalid view geometry: height is negative` AppKit fault 1회.
- iOS Simulator: CoreHaptics pattern 파일 부재, RemoteTextInput session,
  WebKit snapshot·suspension 경고와 향후 `UIScene` lifecycle 요구 fault.
  테스트 중 앱 크래시는 관찰되지 않았다.
- Android: cold launch 때
  `tile memory limits exceeded, some content may not draw` 경고 2회.
  테스트 화면 누락이나 크래시는 관찰되지 않았다.

근거:
[macOS 로그](evidence/2026-07-24-cross-platform/macos-runtime-errors.log),
[iOS 로그](evidence/2026-07-24-cross-platform/ios-runtime-errors.log),
[Android 로그 필터](evidence/2026-07-24-cross-platform/android-56-final-error-filter.txt)

## 수정 우선순위와 재검증 범위

1. Android IME와 top safe-area inset을 먼저 고친다.
2. iOS 패키징 대상 충돌을 제거하고, stale 앱이 남지 않는 표준 빌드·설치
   경로를 만든다.
3. 서재의 hard-coded 최근 대화 표시를 제거하고, 모든 터치 조작에 실제
   44pt hit area를 제공한다.
4. Android artifact delivery를 네이티브 share/save 결과 기반으로 바꾸고,
   OS edge back 진행률 연동과 URL tooltip 억제를 정리한다.
5. 비활성 Vertex가 기존 선택값으로 남을 때 지원 제공자로 유도하거나
   명시적인 마이그레이션 상태를 보여 준다.
6. 위 수정 후 `F-01`~`F-09`를 우선 재실행하고, 폐기용 데이터·테스트
   자격증명·실기기를 준비해 `BLOCKED`와 `NOT RUN` 항목을 별도 통과시킨다.

## 테스트 후 복원

- iOS: 하드웨어 키보드 연결을 다시 켰고, 테마·표시 모드를 원래 값으로
  복원한 뒤 서재 화면으로 돌아왔다.
- Android: 시스템 테마, 채팅 모드, OpenAI, 자격증명 0자, 서재 화면으로
  복원했다.
- macOS: 원래 light 테마와 story 모드를 보존하고 서재 화면으로 돌아왔다.
- Chrome은 조작하지 않았다.
- 빌드 전 `cargo clean`으로 재생성 가능한 `target` 캐시를 비웠고 이후
  macOS 앱과 Android 네이티브 중간 산출물을 다시 컴파일했다. 소스와 앱
  데이터에는 영향이 없다.
- QA 실행 중 기존 제품 코드에 추가 변경이나 커밋은 하지 않았으며, QA
  문서와 증거만 추가했다. 작업 트리는 QA 시작 전부터 제품 코드 변경을
  포함한 dirty 상태였다.
