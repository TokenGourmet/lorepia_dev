package dev.lorepia.client

import androidx.core.graphics.Insets
import org.junit.Assert.assertEquals
import org.junit.Test

class AndroidInsetPolicyTest {
  @Test
  fun keepsSystemBarsAndCutoutAsTheStableContentBoundary() {
    val padding = resolveAndroidContentInsets(
      safeInsets = Insets.of(7, 63, 9, 24),
      imeInsets = Insets.NONE,
      imeVisible = false,
    )

    assertEquals(Insets.of(7, 63, 9, 24), padding)
  }

  @Test
  fun raisesContentAboveTheVisibleImeWithoutAddingBottomInsetsTwice() {
    val padding = resolveAndroidContentInsets(
      safeInsets = Insets.of(0, 63, 0, 24),
      imeInsets = Insets.of(0, 0, 0, 731),
      imeVisible = true,
    )

    assertEquals(Insets.of(0, 63, 0, 731), padding)
  }

  @Test
  fun ignoresStaleImeGeometryAfterTheImeBecomesHidden() {
    val padding = resolveAndroidContentInsets(
      safeInsets = Insets.of(0, 63, 0, 24),
      imeInsets = Insets.of(0, 0, 0, 731),
      imeVisible = false,
    )

    assertEquals(Insets.of(0, 63, 0, 24), padding)
  }
}
