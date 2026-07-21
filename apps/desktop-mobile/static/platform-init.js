// The Android wrapper applies physical system-bar insets to the WebView itself.
// Mark that ownership before first paint so CSS env() insets cannot double it.
if (navigator.userAgent.includes("Android")) {
  document.documentElement.dataset.nativeInsetOwner = "android-view-padding";
}
