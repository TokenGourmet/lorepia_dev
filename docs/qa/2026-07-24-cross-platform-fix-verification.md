# LorePia 교차 플랫폼 수정·재검증 보고서

- 기준일: 2026-07-24
- 기준 리비전: `main`의 `8106a1d` 위 현재 미커밋 작업 트리
- 선행 보고서: [2026-07-24-cross-platform-report.md](2026-07-24-cross-platform-report.md)
- 체크리스트: [2026-07-24-cross-platform-checklist.md](2026-07-24-cross-platform-checklist.md)
- 증거 폴더: [evidence/2026-07-24-cross-platform/](evidence/2026-07-24-cross-platform/)

## 결론

선행 보고서의 `F-01`~`F-10`은 이번 작업 트리에서 수정됐다. 특히 이번
추가 요청의 두 항목은 다음과 같이 반영됐다.

1. 채팅 하단의 `+`는 별도 원형 버튼이 아니라 입력창과 같은 캡슐 안의
   44px leading hit slot으로 합쳐졌다.
2. Android와 데스크톱의 뒤로가기는 현재 화면 전체가 손가락·포인터를
   따라 이동하면서 뒤 화면을 노출한다. 전경 화면은 26px 곡률과 그림자를
   끝까지 유지하고, 뒤 화면은 parallax·scale·dim에서 원래 상태로
   복원된다. 취소와 완료는 현재 진행률에서 이어지며 좌우 edge가
   대칭으로 동작한다.

출하 전 별도 과제로 남는 것은 실제 제공자·실기기·장시간 테스트다. 이번
수정 범위에서 새 `P0`~`P2` 회귀는 발견되지 않았다.

## 구현 결과

### 채팅 composer

- `+`, 입력 영역, 전송 버튼을 하나의 composer capsule 안에 배치했다.
- `+`는 프레임 없는 44px hit slot이며 미구현 첨부 기능임을 접근 가능한
  이름과 상태로 알린다.
- 전송 버튼은 시각 원 34px를 유지하면서 실제 hit area는 44px다.
- 모델, 자격증명, 저장소가 준비되지 않은 경우 전송 조작 시 정확한
  차단 이유가 보인다.

근거:
[Android](evidence/2026-07-24-cross-platform/android-latest-chat-composer.png),
[iOS](evidence/2026-07-24-cross-platform/ios-latest-chat-composer.png),
[macOS](evidence/2026-07-24-cross-platform/macos-latest-chat-composer.jpeg)

### 공통 interactive back

- 움직이는 단위를 콘텐츠 카드가 아닌 현재 route의 전체 전경 shell로
  바꿔 상단바, 뒤로가기 버튼, 본문, composer가 함께 이동한다.
- 실제 목적지 URL과 일치하는 이전 렌더를 underlay stack에서 선택한다.
  underlay는 네트워크나 route를 다시 실행하지 않는 non-raster inert DOM
  visual clone이다.
- 전경은 전환 중 26px 곡률과 진행 방향별 그림자를 유지한다.
- underlay에는 scale, parallax, dim을 적용하고 진행률에 따라 해제한다.
- 짧은 이동은 현재 위치에서 취소 settle, 임계치·속도를 넘으면 현재
  위치에서 완료 settle한다.
- Android의 왼쪽·오른쪽 시스템 edge progress와 데스크톱 pointer·wheel
  입력이 같은 CSS 진행률 계약을 사용한다.
- Android는 root layout 한 곳만 native commit을 소유해 이중 pop을
  방지한다. 열린 확인 dialog가 있으면 route를 움직이지 않고 dialog를
  먼저 닫는다.
- iOS는 UIKit navigation controller와
  `interactiveContentPopGestureRecognizer`를 source of truth로 유지하고,
  네이티브 chrome이 활성일 때 중복 Web 뒤로가기 요소를 접근성 트리에서
  제외한다.

직접 증거:

- Android 왼쪽 edge:
  [서재 underlay](evidence/2026-07-24-cross-platform/android-latest-left-edge-mid.png)
- Android 오른쪽 edge:
  [대칭 이동](evidence/2026-07-24-cross-platform/android-latest-right-edge-mid.png)
- Android 채팅 설정 → 채팅:
  [중간](evidence/2026-07-24-cross-platform/android-latest-info-left-edge-mid.png),
  [완료](evidence/2026-07-24-cross-platform/android-latest-info-back-result.png)
- Android 신고 → 채팅 설정:
  [중간](evidence/2026-07-24-cross-platform/android-latest-report-left-edge-mid.png),
  [취소 복원](evidence/2026-07-24-cross-platform/android-latest-report-cancel-restored.png),
  [완료](evidence/2026-07-24-cross-platform/android-latest-report-back-result.png)

### 선행 QA 결함 처리

| 선행 ID | 결과 | 수정·재검증 |
| --- | --- | --- |
| `F-01` Android IME | `FIXED` | IME 표시 시 WebView 하단이 물리 `2277 → 1457px`로 resize되고 composer가 바로 위에 유지됐다. 닫으면 원복됐다. [화면](evidence/2026-07-24-cross-platform/android-latest-ime.png) |
| `F-02` Android top inset | `FIXED` | 상태바 inset을 native root에서 소유하고 상단 조작부와 겹치지 않게 했다. |
| `F-03` iOS 패키징 충돌 | `FIXED` | 빌드 전 stale destination을 제한적으로 정리하는 스크립트와 경계 테스트를 추가했다. 표준 simulator 앱 빌드·설치가 성공했다. |
| `F-04` 가짜 서재 최근 대화 | `FIXED` | 저장 이력이 없으면 샘플 문장·시간을 최근 대화처럼 표시하지 않는다. |
| `F-05` 44pt hit area | `FIXED` | composer, 설정 segment, 신고·삭제·재시도 등 주요 조작 영역을 44px 이상으로 보강했다. |
| `F-06` Android 가짜 다운로드 성공 | `FIXED` | 결과를 확인하지 않은 성공 문구를 제거하고 실제 공유·복사·오류 상태만 표시한다. 시스템 저장 서비스 유무 자체는 기기 환경에 따른다. |
| `F-07` Android OS edge 추종 없음 | `FIXED` | native back progress를 Web transition에 연결했다. 왼쪽·오른쪽 edge에서 화면 추종, 취소, 완료를 직접 확인했다. |
| `F-08` 비활성 Vertex 선택 잔류 | `FIXED` | 기존 설정을 지원 OpenAI 설정으로 마이그레이션하고 config-only Vertex를 다시 선택할 수 없게 했다. |
| `F-09` 모바일 내부 URL long-press | `FIXED` | iOS·Android 내부 탐색 항목의 context menu와 drag preview를 차단하고 데스크톱 우클릭은 보존했다. |
| `F-10` 후행 공백 | `FIXED` | Swift 파일의 12개 후행 공백을 제거했고 `git diff --check`가 통과했다. |

## 플랫폼 직접 검증

### iOS

- 환경: iPhone 17 Simulator, iOS 26.5
- 현재 작업 트리의 simulator 앱을 빌드·설치·실행했다.
- 합쳐진 composer와 네이티브 뒤로가기 버튼의 push/pop을 확인했다.
- UIKit interactive pop 구현과 JS/native 경계 테스트는 통과했다.
- 한계: macOS 마우스 drag는 Simulator에서 finger touch edge gesture를
  합성하지 못했다. 따라서 이번 실행에서 iOS의 실터치 진행률 추종을
  직접 `PASS`로 과장하지 않는다. 실기기 touch 검증이 남아 있다.

산출물:
`apps/desktop-mobile/src-tauri/gen/apple/build/arm64-sim/LorePia.app`

### Android

- 환경: Android 16 emulator, 1080×2340 @ 420dpi
- 현재 universal debug APK를 설치하고 실제 시스템 edge를 조작했다.
- 왼쪽 edge와 오른쪽 edge 모두 방향에 맞게 전경 전체가 이동하고 뒤
  route가 보였다.
- 채팅 → 서재, 대화 설정 → 채팅, 신고 → 대화 설정의 실제 목적지가
  underlay와 일치했다.
- 짧은 gesture cancel은 같은 route로 복원됐고 긴 gesture complete는
  한 단계만 이동했다.
- IME 표시·해제, top/bottom inset, composer 위치 복원을 확인했다.

산출물:
`apps/desktop-mobile/src-tauri/gen/android/app/build/outputs/apk/universal/debug/app-universal-debug.apk`

- 크기: `236,833,656` bytes
- SHA-256:
  `8a075db4dfeae252cff831c9a8e8c791639d1b4073de122970a2224c5c0e7d00`

### macOS

- 환경: macOS 26.5.2
- 현재 debug 앱에서 합쳐진 composer를 확인했다.
- 긴 mouse drag는 채팅에서 서재로 완료됐고, 약 8px의 짧은 drag는
  채팅 화면으로 취소 복원됐다.
- 최신 작업 트리로 macOS `.app`을 다시 빌드해 보존했다.
- hardware trackpad의 wheel gesture는 코드·단위 테스트로 검증했으며,
  이번 직접 조작은 mouse drag 기준이다.
- Chrome은 열거나 조작하지 않았다.

산출물:
`target/debug/bundle/macos/LorePia.app`

## 자동 검증

| 검증 | 결과 |
| --- | --- |
| Vitest | `PASS` — 35 files, 251 tests |
| Node 경계 테스트 | `PASS` — 36/36 |
| `svelte-check` | `PASS` — 0 errors, 0 warnings |
| 프로덕션 Web build | `PASS` — SSR 213 modules, client 238 modules, built-boundary 55 files |
| `cargo fmt --all -- --check` | `PASS` |
| `cargo clippy -p lorepia-app --tests -- -D warnings` | `PASS` — warnings 0 |
| `cargo test -p lorepia-app provider_stream::tests::` | `PASS` — 46/46 |
| native-back Rust package | `PASS` — build, unit/doc test failure 0 |
| Android app JVM tests | `PASS` — 3/3 |
| Android native-back JVM tests | `PASS` — 4/4 |
| macOS Tauri debug app build | `PASS` |
| `git diff --check` | `PASS` |

Android JVM 재검증 시 연결돼 있던 외장 SDK 경로가 사라져 1차 실행이
중단됐지만, 임시 SDK를 별도 생성해 같은 명령을 다시 실행했고 최종
`BUILD SUCCESSFUL`을 확인했다. 이 임시 SDK는 테스트 후 삭제했다.

## 공식 동작 기준

- Apple UIKit interactive pop:
  <https://developer.apple.com/documentation/uikit/uinavigationcontroller/interactivecontentpopgesturerecognizer>
- Apple UIKit, iOS 26:
  <https://developer.apple.com/videos/play/wwdc2025/243/>
- Apple gestures:
  <https://developer.apple.com/design/human-interface-guidelines/gestures>
- Android predictive back custom animation:
  <https://developer.android.com/guide/navigation/custom-back/support-animations-views>
- Android software keyboard:
  <https://developer.android.com/develop/ui/views/layout/sw-keyboard>

## 남은 검증 범위

- iOS 실기기에서 interactive edge의 진행률·취소·완료와 긴 누름 확대경
- Android 실기기 제조사별 predictive back와 시스템 공유·파일 저장 결과
- 실제 제공자 자격증명으로 스트리밍·취소·오류·재시도
- VoiceOver·TalkBack 전체 흐름, 회전, 오프라인, 8시간·72시간 soak,
  release 서명·스토어 패키징

## 작업 트리와 정리

- 이번 수정은 커밋하지 않았다.
- 작업 트리는 시작 전부터 다른 제품 변경과 `.claude/`를 포함한 dirty
  상태였으며, 해당 사용자 변경을 되돌리지 않았다.
- 검증을 위해 생성한 임시 Android SDK는 삭제했다.
- 재생성 가능한 Android Rust 중간 캐시는 정리했고, 최신 APK·iOS 앱·
  macOS 앱과 QA 증거는 보존했다.
