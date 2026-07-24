package dev.lorepia.nativechrome

import android.app.Activity
import android.content.res.ColorStateList
import android.content.res.Configuration
import android.graphics.Color
import android.util.TypedValue
import android.view.Gravity
import android.view.Menu
import android.view.View
import android.webkit.WebView
import android.widget.FrameLayout
import androidx.appcompat.app.AppCompatActivity
import androidx.core.graphics.ColorUtils
import androidx.core.graphics.Insets
import androidx.core.view.ViewCompat
import androidx.core.view.WindowInsetsCompat
import app.tauri.annotation.Command
import app.tauri.annotation.InvokeArg
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin
import com.google.android.material.bottomnavigation.BottomNavigationView
import com.google.android.material.navigation.NavigationBarView
import com.google.android.material.shape.MaterialShapeDrawable
import com.google.android.material.shape.ShapeAppearanceModel

private enum class NativeChromeTab(
  val wireValue: String,
  val label: String,
  val itemId: Int,
  val iconResource: Int,
) {
  HOME(
    "home",
    "홈",
    View.generateViewId(),
    R.drawable.ic_native_tab_home_selector,
  ),
  LIBRARY(
    "library",
    "서재",
    View.generateViewId(),
    R.drawable.ic_native_tab_library_selector,
  ),
  CREATE(
    "create",
    "생성",
    View.generateViewId(),
    R.drawable.ic_native_tab_create_selector,
  ),
  ACCOUNT(
    "account",
    "계정",
    View.generateViewId(),
    R.drawable.ic_native_tab_account_selector,
  );

  companion object {
    fun fromWireValue(value: String): NativeChromeTab? =
      entries.firstOrNull { it.wireValue == value }

    fun fromItemId(value: Int): NativeChromeTab? =
      entries.firstOrNull { it.itemId == value }
  }
}

private enum class NativeChromeAppearance(val wireValue: String) {
  SYSTEM("system"),
  LIGHT("light"),
  DARK("dark");

  companion object {
    fun fromWireValue(value: String): NativeChromeAppearance? =
      entries.firstOrNull { it.wireValue == value }
  }
}

private data class NativeChromeState(
  val visible: Boolean = false,
  val selectedTab: NativeChromeTab = NativeChromeTab.LIBRARY,
  val appearance: NativeChromeAppearance = NativeChromeAppearance.SYSTEM,
  val compact: Boolean = false,
)

@InvokeArg
class NativeChromeStateArgs {
  var visible: Boolean = false
  var selectedTab: String = NativeChromeTab.LIBRARY.wireValue
  var minimized: Boolean = false
  var appearance: String = NativeChromeAppearance.SYSTEM.wireValue
  var compact: Boolean = false
}

@TauriPlugin
class NativeChromePlugin(private val activity: Activity) : Plugin(activity) {
  private var webView: WebView? = null
  private var dock: BottomNavigationView? = null
  private var state = NativeChromeState()
  private var applyingSelection = false
  private var pendingTab: NativeChromeTab? = null
  private var pendingSelectionGeneration = 0
  private var imeVisible = false

  override fun load(webView: WebView) {
    this.webView = webView
  }

  override fun onDestroy(activity: AppCompatActivity) {
    dock?.let { current ->
      (current.parent as? FrameLayout)?.removeView(current)
    }
    dock = null
    webView = null
  }

  @Command
  fun setState(invoke: Invoke) {
    val args = invoke.parseArgs(NativeChromeStateArgs::class.java)
    val selectedTab = NativeChromeTab.fromWireValue(args.selectedTab)
    val appearance =
      NativeChromeAppearance.fromWireValue(args.appearance)
    if (selectedTab == null || appearance == null) {
      invoke.reject(
        "Native chrome state contains an unknown closed enum value",
        "INVALID_NATIVE_CHROME_STATE",
      )
      return
    }

    resolveOnMain(invoke) {
      state = NativeChromeState(
        visible = args.visible,
        selectedTab = selectedTab,
        appearance = appearance,
        compact = args.compact,
      )
      if (pendingTab == selectedTab) {
        pendingTab = null
        pendingSelectionGeneration += 1
      }
      if (state.compact) {
        installDockIfNeeded()
      }
      applyState()
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
    activity.runOnUiThread {
      action()
      invoke.resolve(currentStatus())
    }
  }

  private fun currentStatus(): JSObject {
    val active = state.compact && dock != null
    return JSObject().apply {
      put("supported", true)
      put("active", active)
      put("compact", state.compact)
      put("visible", active && state.visible && !imeVisible)
      put("selectedTab", state.selectedTab.wireValue)
      // Android's stable Material navigation bar does not copy iOS tab
      // minimization. The bridge keeps the same closed response shape.
      put("minimized", false)
    }
  }

  private fun installDockIfNeeded() {
    if (dock != null) {
      return
    }

    val nativeDock = BottomNavigationView(activity).apply {
      id = View.generateViewId()
      minimumHeight = dp(80)
      labelVisibilityMode =
        NavigationBarView.LABEL_VISIBILITY_LABELED
      isItemHorizontalTranslationEnabled = false
      itemIconSize = dp(22)
      setItemPaddingTop(dp(12))
      setItemPaddingBottom(dp(16))
      setActiveIndicatorLabelPadding(dp(4))
      setItemActiveIndicatorEnabled(true)
      setItemActiveIndicatorWidth(dp(64))
      setItemActiveIndicatorHeight(dp(32))
      setItemActiveIndicatorShapeAppearance(
        ShapeAppearanceModel.builder()
          .setAllCornerSizes(dp(16).toFloat())
          .build(),
      )
      itemRippleColor = ColorStateList.valueOf(Color.TRANSPARENT)
      importantForAccessibility = View.IMPORTANT_FOR_ACCESSIBILITY_YES

      NativeChromeTab.entries.forEachIndexed { order, tab ->
        menu.add(
          Menu.NONE,
          tab.itemId,
          order,
          tab.label,
        ).setIcon(tab.iconResource)
      }

      setOnItemSelectedListener { item ->
        val tab = NativeChromeTab.fromItemId(item.itemId)
          ?: return@setOnItemSelectedListener false
        if (applyingSelection) {
          true
        } else {
          pendingSelectionGeneration += 1
          val generation = pendingSelectionGeneration
          pendingTab = tab
          select(tab)
          postDelayed({
            if (
              pendingSelectionGeneration == generation &&
              pendingTab == tab
            ) {
              pendingTab = null
              applyState()
            }
          }, 2_000)
          // Material commits the checked item immediately. SvelteKit later
          // confirms it through setState; stale route commits cannot bounce
          // the indicator away from the newest native tap.
          true
        }
      }
      setOnItemReselectedListener { item ->
        if (!applyingSelection) {
          NativeChromeTab.fromItemId(item.itemId)?.let(::select)
        }
      }
    }

    val layoutParams = FrameLayout.LayoutParams(
      FrameLayout.LayoutParams.MATCH_PARENT,
      dp(80),
      Gravity.BOTTOM,
    ).apply {
      leftMargin = dp(16)
      rightMargin = dp(16)
      bottomMargin = dp(16)
    }
    activity.addContentView(nativeDock, layoutParams)
    ViewCompat.setOnApplyWindowInsetsListener(nativeDock) {
        view,
        insets,
      ->
      val safeTypes =
        WindowInsetsCompat.Type.systemBars() or
          WindowInsetsCompat.Type.displayCutout()
      val safeInsets = insets.getInsets(safeTypes)
      imeVisible =
        insets.isVisible(WindowInsetsCompat.Type.ime())

      val currentParams = view.layoutParams as FrameLayout.LayoutParams
      val left = dp(16) + safeInsets.left
      val right = dp(16) + safeInsets.right
      val bottom = dp(16) + safeInsets.bottom
      if (
        currentParams.leftMargin != left ||
        currentParams.rightMargin != right ||
        currentParams.bottomMargin != bottom
      ) {
        currentParams.leftMargin = left
        currentParams.rightMargin = right
        currentParams.bottomMargin = bottom
        view.layoutParams = currentParams
      }
      applyVisibility()

      WindowInsetsCompat.Builder(insets)
        .setInsets(safeTypes, Insets.NONE)
        .setInsets(WindowInsetsCompat.Type.ime(), Insets.NONE)
        .build()
    }
    dock = nativeDock
    applyStyle(nativeDock)
    ViewCompat.requestApplyInsets(nativeDock)
  }

  private fun applyState() {
    val nativeDock = dock ?: return
    val displayedTab = pendingTab ?: state.selectedTab
    applyingSelection = true
    try {
      if (nativeDock.selectedItemId != displayedTab.itemId) {
        nativeDock.selectedItemId = displayedTab.itemId
      }
    } finally {
      applyingSelection = false
    }
    applyStyle(nativeDock)
    applyVisibility()
  }

  private fun applyVisibility() {
    dock?.visibility =
      if (state.compact && state.visible && !imeVisible) {
        View.VISIBLE
      } else {
        View.GONE
      }
  }

  private fun applyStyle(nativeDock: BottomNavigationView) {
    val dark = when (state.appearance) {
      NativeChromeAppearance.DARK -> true
      NativeChromeAppearance.LIGHT -> false
      NativeChromeAppearance.SYSTEM ->
        activity.resources.configuration.uiMode and
          Configuration.UI_MODE_NIGHT_MASK ==
          Configuration.UI_MODE_NIGHT_YES
    }
    val surface = if (dark) {
      Color.rgb(38, 38, 36)
    } else {
      Color.rgb(250, 249, 245)
    }
    val foreground = if (dark) {
      Color.rgb(236, 233, 224)
    } else {
      Color.rgb(75, 73, 66)
    }
    val inactive = ColorUtils.setAlphaComponent(foreground, 150)
    val accent = resolveThemeColor(
      android.R.attr.colorAccent,
      if (dark) Color.rgb(100, 181, 246) else Color.rgb(25, 118, 210),
    )
    val states = arrayOf(
      intArrayOf(android.R.attr.state_checked),
      intArrayOf(),
    )
    val navigationColors = ColorStateList(
      states,
      intArrayOf(accent, inactive),
    )
    val shape = ShapeAppearanceModel.builder()
      .setAllCornerSizes(dp(40).toFloat())
      .build()
    val background = MaterialShapeDrawable(shape).apply {
      fillColor = ColorStateList.valueOf(surface)
      initializeElevationOverlay(activity)
      elevation = dp(8).toFloat()
    }

    nativeDock.background = background
    ViewCompat.setElevation(nativeDock, dp(8).toFloat())
    nativeDock.itemIconTintList = navigationColors
    nativeDock.itemTextColor = navigationColors
    nativeDock.itemActiveIndicatorColor =
      ColorStateList.valueOf(
        ColorUtils.setAlphaComponent(accent, 28),
      )
  }

  private fun select(tab: NativeChromeTab) {
    val status = currentStatus()
    if (
      status.optBoolean("active") != true ||
      status.optBoolean("visible") != true
    ) {
      return
    }
    dispatchTab(tab)
  }

  private fun dispatchTab(tab: NativeChromeTab) {
    val script =
      "window.dispatchEvent(new CustomEvent('lorepia:native-tab'," +
        "{detail:{tab:'${tab.wireValue}'}}))"
    webView?.post {
      webView?.evaluateJavascript(script, null)
    }
  }

  private fun resolveThemeColor(
    attribute: Int,
    fallback: Int,
  ): Int {
    val value = TypedValue()
    return if (activity.theme.resolveAttribute(attribute, value, true)) {
      if (value.resourceId != 0) {
        activity.getColor(value.resourceId)
      } else {
        value.data
      }
    } else {
      fallback
    }
  }

  private fun dp(value: Int): Int =
    TypedValue.applyDimension(
      TypedValue.COMPLEX_UNIT_DIP,
      value.toFloat(),
      activity.resources.displayMetrics,
    ).toInt()
}
