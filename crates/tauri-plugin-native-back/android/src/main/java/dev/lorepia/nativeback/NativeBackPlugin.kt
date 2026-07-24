package dev.lorepia.nativeback

import android.app.Activity
import android.os.Build
import android.webkit.WebView
import androidx.activity.BackEventCompat
import androidx.activity.OnBackPressedCallback
import androidx.appcompat.app.AppCompatActivity
import app.tauri.annotation.Command
import app.tauri.annotation.InvokeArg
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin

@InvokeArg
class SetEnabledArgs {
  var enabled: Boolean = false
}

@TauriPlugin
class NativeBackPlugin(private val activity: Activity) : Plugin(activity) {
  private val host = activity as AppCompatActivity
  private var webView: WebView? = null
  private var activeSwipeEdge = NativeBackEdge.LEFT

  private val backCallback = object : OnBackPressedCallback(false) {
    override fun handleOnBackStarted(backEvent: BackEventCompat) {
      activeSwipeEdge = nativeBackEdge(backEvent)
      dispatchProgress(
        NativeBackPhase.START,
        backEvent.progress,
        activeSwipeEdge,
      )
    }

    override fun handleOnBackProgressed(backEvent: BackEventCompat) {
      activeSwipeEdge = nativeBackEdge(backEvent)
      dispatchProgress(
        NativeBackPhase.PROGRESS,
        backEvent.progress,
        activeSwipeEdge,
      )
    }

    override fun handleOnBackCancelled() {
      dispatchProgress(
        NativeBackPhase.CANCEL,
        0f,
        activeSwipeEdge,
      )
      activeSwipeEdge = NativeBackEdge.LEFT
    }

    override fun handleOnBackPressed() {
      val committedEdge = activeSwipeEdge
      dispatchProgress(
        phase = NativeBackPhase.COMMIT,
        progress = 1f,
        edge = committedEdge,
        includeLegacyCommit = true,
      )
      activeSwipeEdge = NativeBackEdge.LEFT
    }
  }

  init {
    host.onBackPressedDispatcher.addCallback(host, backCallback)
  }

  override fun load(webView: WebView) {
    this.webView = webView
  }

  override fun onDestroy(activity: AppCompatActivity) {
    backCallback.remove()
    webView = null
  }

  @Command
  fun complete(invoke: Invoke) {
    resolveOnMain(invoke) {
      backCallback.isEnabled = false
      activeSwipeEdge = NativeBackEdge.LEFT
    }
  }

  @Command
  fun pop(invoke: Invoke) {
    resolveOnMain(invoke) {
      if (backCallback.isEnabled) {
        activeSwipeEdge = NativeBackEdge.LEFT
        dispatchProgress(
          phase = NativeBackPhase.COMMIT,
          progress = 1f,
          edge = activeSwipeEdge,
          includeLegacyCommit = true,
        )
      }
    }
  }

  @Command
  fun prepare(invoke: Invoke) {
    resolveOnMain(invoke) {
      backCallback.isEnabled = true
    }
  }

  @Command
  fun setEnabled(invoke: Invoke) {
    val args = invoke.parseArgs(SetEnabledArgs::class.java)
    resolveOnMain(invoke) {
      backCallback.isEnabled = args.enabled
      activeSwipeEdge = NativeBackEdge.LEFT
    }
  }

  @Command
  fun status(invoke: Invoke) {
    resolveOnMain(invoke)
  }

  private fun resolveOnMain(
    invoke: Invoke,
    action: () -> Unit = {},
  ) {
    host.runOnUiThread {
      action()
      invoke.resolve(currentStatus())
    }
  }

  private fun currentStatus(): JSObject = JSObject().apply {
    put("supported", true)
    put("active", backCallback.isEnabled)
    put(
      "gestureEnabled",
      backCallback.isEnabled && Build.VERSION.SDK_INT >= Build.VERSION_CODES.UPSIDE_DOWN_CAKE,
    )
  }

  private fun dispatchProgress(
    phase: NativeBackPhase,
    progress: Float,
    edge: NativeBackEdge = NativeBackEdge.LEFT,
    includeLegacyCommit: Boolean = false,
  ) {
    val script = nativeBackEventScript(
      phase = phase,
      progress = progress,
      edge = edge,
      includeLegacyCommit = includeLegacyCommit,
    )
    webView?.post {
      webView?.evaluateJavascript(script, null)
    }
  }

  private fun nativeBackEdge(
    backEvent: BackEventCompat,
  ): NativeBackEdge =
    if (backEvent.swipeEdge == BackEventCompat.EDGE_RIGHT) {
      NativeBackEdge.RIGHT
    } else {
      NativeBackEdge.LEFT
    }
}
