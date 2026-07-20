# DC Cleaner Safe

`dccleaner3/dccleaner`의 보안 검토 뒤, 삭제에 필요한 최소 기능만 별도로 다시 작성한 Android 테스트 앱입니다. 원본 소스 파일이나 자산을 복사하지 않았습니다.

## 보안 경계

- 네이티브 화면은 DCInside 비밀번호를 입력받거나 저장하지 않습니다. 로그인은 앱 안의 Android WebView에서 공식 HTTPS DCInside 페이지에 직접 수행합니다.
- 앱 권한은 `android.permission.INTERNET` 하나뿐입니다.
- 계정 저장, 2Captcha, 광고, 분석/텔레메트리, 업데이트 검사, 백그라운드 서비스, Wake Lock, 부팅 리시버, 정확한 알람, 글쓰기, 댓글쓰기, 방명록 자동화, 동적 코드 로딩, 네이티브 라이브러리, 외부 로그 파일이 없습니다.
- 평문 HTTP를 막고, WebView 최상위 이동은 `dcinside.com`과 그 하위 도메인만 허용합니다. TLS 인증서 오류는 우회하지 않고 취소합니다.
- 미리보기에서 고정된 목록만 삭제합니다. 기본 상한은 오래된 항목 10개이며, 삭제 전 `삭제 N` 확인 문구를 직접 입력해야 합니다.
- 미리보기는 10분 뒤 만료됩니다. 쿠키가 바뀌면 다시 미리보기를 요구합니다.
- 캡챠, HTTP 401/403/429, 연속 실패 3회, 사용자 취소, 앱의 포그라운드 이탈 시 중단합니다.
- 세션 삭제 버튼은 WebView 쿠키·스토리지·캐시·기록을 지웁니다. 정상 종료 시 자동 정리도 기본 활성화됩니다.
- Android 백업은 꺼져 있습니다.

## 중요한 한계

이것은 비공식 테스트 빌드입니다. DCInside는 HTML, 쿠키, 삭제 엔드포인트, 자동화 방지 정책을 언제든 바꿀 수 있습니다. 먼저 중요하지 않은 계정과 1~10개의 작은 범위에서 검증하세요. 삭제는 복구되지 않습니다. 앱은 캡챠를 풀거나 우회하지 않습니다.

CI가 만드는 APK는 테스트용 debug 서명입니다. 옆에 제공되는 SHA-256 파일과 보안 검사 보고서를 확인하세요.

## 빌드

JDK 17, Android SDK Platform 35, Build Tools 35.0.0이 필요합니다.

```bash
gradle --no-daemon clean testDebugUnitTest lintDebug assembleDebug
bash scripts/security-check.sh app/build/outputs/apk/debug/app-debug.apk
```

## 라이선스

독립 재작성물이지만, 검토 대상이 AGPL-3.0 프로젝트였으므로 분쟁 여지를 줄이기 위해 이 프로젝트도 `AGPL-3.0-or-later`로 배포합니다. 배포 산출물에는 전체 AGPL-3.0 라이선스 본문을 함께 넣습니다.
