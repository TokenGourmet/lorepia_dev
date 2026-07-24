package dev.lorepia.nativeback

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class NativeBackEventTest {
  @Test
  fun clampsFrameworkProgressToThePublicEventContract() {
    assertEquals(0f, normalizedProgress(Float.NaN))
    assertEquals(0f, normalizedProgress(-0.25f))
    assertEquals(0.5f, normalizedProgress(0.5f))
    assertEquals(1f, normalizedProgress(1.25f))
  }

  @Test
  fun progressEventUsesTheSharedWebContractWithoutACommit() {
    val script = nativeBackEventScript(NativeBackPhase.PROGRESS, 0.625f)

    assertTrue(script.contains("'lorepia:native-back-progress'"))
    assertTrue(script.contains("phase: 'progress'"))
    assertTrue(script.contains("progress: 0.625"))
    assertTrue(script.contains("edge: 'left'"))
    assertFalse(script.contains("'lorepia:native-back'"))
  }

  @Test
  fun rightEdgeProgressPreservesTheGestureDirection() {
    val script = nativeBackEventScript(
      phase = NativeBackPhase.PROGRESS,
      progress = 0.5f,
      edge = NativeBackEdge.RIGHT,
    )

    assertTrue(script.contains("edge: 'right'"))
  }

  @Test
  fun committedGestureAlsoEmitsTheLegacyNavigationEvent() {
    val script = nativeBackEventScript(
      phase = NativeBackPhase.COMMIT,
      progress = 1f,
      includeLegacyCommit = true,
    )

    assertTrue(script.contains("phase: 'commit'"))
    assertTrue(script.contains("progress: 1.0"))
    assertTrue(
      script.contains(
        "window.dispatchEvent(new Event('lorepia:native-back'));",
      ),
    )
  }
}
