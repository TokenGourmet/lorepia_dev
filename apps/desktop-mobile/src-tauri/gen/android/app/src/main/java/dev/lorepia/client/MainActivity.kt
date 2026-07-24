package dev.lorepia.client

import android.content.Context
import android.os.Bundle
import android.view.View
import android.view.ViewGroup
import android.view.WindowManager
import android.webkit.WebView
import androidx.activity.enableEdgeToEdge
import androidx.core.graphics.Insets
import androidx.core.view.ViewCompat
import androidx.core.view.WindowInsetsCompat

class MainActivity : TauriActivity() {
  private external fun initNdkContext(context: Context)

  override fun onCreate(savedInstanceState: Bundle?) {
    enableEdgeToEdge()
    window.setSoftInputMode(WindowManager.LayoutParams.SOFT_INPUT_ADJUST_RESIZE)
    super.onCreate(savedInstanceState)
    initNdkContext(applicationContext)
  }

  override fun onWebViewCreate(webView: WebView) {
    super.onWebViewCreate(webView)
    webView.overScrollMode = View.OVER_SCROLL_NEVER
    webView.isVerticalScrollBarEnabled = false
    webView.isHorizontalScrollBarEnabled = false

    ViewCompat.setOnApplyWindowInsetsListener(webView) { view, insets ->
      val safeTypes =
        WindowInsetsCompat.Type.systemBars() or
          WindowInsetsCompat.Type.displayCutout()
      val safeInsets = insets.getInsets(safeTypes)
      val imeInsets = insets.getInsets(WindowInsetsCompat.Type.ime())
      val boundaryInsets = resolveAndroidContentInsets(
        safeInsets = safeInsets,
        imeInsets = imeInsets,
        imeVisible = insets.isVisible(WindowInsetsCompat.Type.ime()),
      )
      val layoutParams = view.layoutParams
      if (layoutParams is ViewGroup.MarginLayoutParams) {
        if (
          layoutParams.leftMargin != boundaryInsets.left ||
          layoutParams.topMargin != boundaryInsets.top ||
          layoutParams.rightMargin != boundaryInsets.right ||
          layoutParams.bottomMargin != boundaryInsets.bottom
        ) {
          layoutParams.setMargins(
            boundaryInsets.left,
            boundaryInsets.top,
            boundaryInsets.right,
            boundaryInsets.bottom,
          )
          view.layoutParams = layoutParams
        }
      }

      // The native WebView boundary owns these dimensions. WebView does not
      // translate HTML layout coordinates for View padding, so margins form
      // the physical safe viewport instead. Zero only the handled types so
      // WebView still receives subsequent inset changes and cannot retain
      // ghost or duplicate safe-area/IME padding.
      WindowInsetsCompat.Builder(insets)
        .setInsets(safeTypes, Insets.NONE)
        .setInsets(WindowInsetsCompat.Type.ime(), Insets.NONE)
        .build()
    }
    ViewCompat.requestApplyInsets(webView)
  }
}

internal fun resolveAndroidContentInsets(
  safeInsets: Insets,
  imeInsets: Insets,
  imeVisible: Boolean,
): Insets = Insets.of(
  safeInsets.left,
  safeInsets.top,
  safeInsets.right,
  if (imeVisible) {
    maxOf(safeInsets.bottom, imeInsets.bottom)
  } else {
    safeInsets.bottom
  },
)
