package dev.lorepia.client

import android.content.Context
import android.os.Bundle
import android.view.View
import android.webkit.WebView
import androidx.activity.enableEdgeToEdge
import androidx.core.view.ViewCompat
import androidx.core.view.WindowInsetsCompat

class MainActivity : TauriActivity() {
  private external fun initNdkContext(context: Context)

  override fun onCreate(savedInstanceState: Bundle?) {
    enableEdgeToEdge()
    super.onCreate(savedInstanceState)
    initNdkContext(applicationContext)
  }

  override fun onWebViewCreate(webView: WebView) {
    super.onWebViewCreate(webView)
    webView.overScrollMode = View.OVER_SCROLL_NEVER
    webView.isVerticalScrollBarEnabled = false
    webView.isHorizontalScrollBarEnabled = false

    ViewCompat.setOnApplyWindowInsetsListener(webView) { _, insets ->
      val bars = insets.getInsets(
        WindowInsetsCompat.Type.systemBars() or
          WindowInsetsCompat.Type.displayCutout(),
      )
      val density = resources.displayMetrics.density
      val top = (bars.top / density).toInt()
      val bottom = (bars.bottom / density).toInt()
      webView.evaluateJavascript(
        "document.documentElement.style.setProperty('--safe-top','${top}px');" +
          "document.documentElement.style.setProperty('--safe-bottom','${bottom}px');",
        null,
      )
      insets
    }
  }
}
