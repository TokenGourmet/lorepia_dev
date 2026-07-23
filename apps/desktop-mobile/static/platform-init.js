const userAgent = navigator.userAgent;
const isIOS =
  /iPad|iPhone|iPod/u.test(userAgent) ||
  (navigator.platform === "MacIntel" && navigator.maxTouchPoints > 1);

// Mark iOS before first paint so its WKWebView-only interaction treatment
// never leaks into Android, desktop, or the browser development surface.
if (isIOS) {
  document.documentElement.dataset.nativePlatform = "ios";
}

// The Android wrapper applies physical system-bar insets to the WebView itself.
// Mark that ownership before first paint so CSS env() insets cannot double it.
if (userAgent.includes("Android")) {
  document.documentElement.dataset.nativePlatform = "android";
  document.documentElement.dataset.nativeInsetOwner = "android-view-padding";
}
