package dev.lorepia.nativeback

internal enum class NativeBackPhase(val wireValue: String) {
  START("start"),
  PROGRESS("progress"),
  CANCEL("cancel"),
  COMMIT("commit"),
}

internal enum class NativeBackEdge(val wireValue: String) {
  LEFT("left"),
  RIGHT("right"),
}

internal fun normalizedProgress(progress: Float): Float =
  if (progress.isFinite()) progress.coerceIn(0f, 1f) else 0f

internal fun nativeBackEventScript(
  phase: NativeBackPhase,
  progress: Float,
  edge: NativeBackEdge = NativeBackEdge.LEFT,
  includeLegacyCommit: Boolean = false,
): String {
  val safeProgress = normalizedProgress(progress)
  val legacyCommit = if (includeLegacyCommit) {
    "window.dispatchEvent(new Event('lorepia:native-back'));"
  } else {
    ""
  }
  return """
    (() => {
      window.dispatchEvent(new CustomEvent('lorepia:native-back-progress', {
        detail: {
          phase: '${phase.wireValue}',
          progress: $safeProgress,
          edge: '${edge.wireValue}'
        }
      }));
      $legacyCommit
    })();
  """.trimIndent()
}
