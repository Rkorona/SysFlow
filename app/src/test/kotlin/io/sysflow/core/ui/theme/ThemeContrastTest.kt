package io.sysflow.core.ui.theme

import androidx.compose.ui.graphics.Color
import org.junit.Assert.assertTrue
import org.junit.Test

/** WCAG 2.1 AA contrast checks for the foreground/background pairs used by the theme. */
class ThemeContrastTest {
    @Test
    fun `core material roles meet readable contrast in both themes`() {
        val pairs = listOf(
            // Light brand and semantic roles.
            SysFlowPalette.PrimaryLight to SysFlowPalette.OnPrimaryLight,
            SysFlowPalette.PrimaryContainerLight to SysFlowPalette.OnPrimaryContainerLight,
            SysFlowPalette.SecondaryLight to SysFlowPalette.OnSecondaryLight,
            SysFlowPalette.SecondaryContainerLight to SysFlowPalette.OnSecondaryContainerLight,
            SysFlowPalette.TertiaryLight to SysFlowPalette.OnTertiaryLight,
            SysFlowPalette.TertiaryContainerLight to SysFlowPalette.OnTertiaryContainerLight,
            SysFlowPalette.ErrorLight to SysFlowPalette.OnErrorLight,
            SysFlowPalette.ErrorContainerLight to SysFlowPalette.OnErrorContainerLight,
            SysFlowPalette.BackgroundLight to SysFlowPalette.OnBackgroundLight,
            SysFlowPalette.SurfaceLight to SysFlowPalette.OnSurfaceLight,
            SysFlowPalette.SurfaceVariantLight to SysFlowPalette.OnSurfaceVariantLight,
            SysFlowPalette.SurfaceDimLight to SysFlowPalette.OnSurfaceLight,
            SysFlowPalette.SurfaceBrightLight to SysFlowPalette.OnSurfaceLight,
            SysFlowPalette.SurfaceContainerLowestLight to SysFlowPalette.OnSurfaceLight,
            SysFlowPalette.SurfaceContainerLowLight to SysFlowPalette.OnSurfaceLight,
            SysFlowPalette.SurfaceContainerLight to SysFlowPalette.OnSurfaceLight,
            SysFlowPalette.SurfaceContainerHighLight to SysFlowPalette.OnSurfaceLight,
            SysFlowPalette.SurfaceContainerHighestLight to SysFlowPalette.OnSurfaceLight,
            SysFlowPalette.InverseSurfaceLight to SysFlowPalette.InverseOnSurfaceLight,
            // Dark brand and semantic roles.
            SysFlowPalette.PrimaryDark to SysFlowPalette.OnPrimaryDark,
            SysFlowPalette.PrimaryContainerDark to SysFlowPalette.OnPrimaryContainerDark,
            SysFlowPalette.SecondaryDark to SysFlowPalette.OnSecondaryDark,
            SysFlowPalette.SecondaryContainerDark to SysFlowPalette.OnSecondaryContainerDark,
            SysFlowPalette.TertiaryDark to SysFlowPalette.OnTertiaryDark,
            SysFlowPalette.TertiaryContainerDark to SysFlowPalette.OnTertiaryContainerDark,
            SysFlowPalette.ErrorDark to SysFlowPalette.OnErrorDark,
            SysFlowPalette.ErrorContainerDark to SysFlowPalette.OnErrorContainerDark,
            SysFlowPalette.BackgroundDark to SysFlowPalette.OnBackgroundDark,
            SysFlowPalette.SurfaceDark to SysFlowPalette.OnSurfaceDark,
            SysFlowPalette.SurfaceVariantDark to SysFlowPalette.OnSurfaceVariantDark,
            SysFlowPalette.SurfaceDimDark to SysFlowPalette.OnSurfaceDark,
            SysFlowPalette.SurfaceBrightDark to SysFlowPalette.OnSurfaceDark,
            SysFlowPalette.SurfaceContainerLowestDark to SysFlowPalette.OnSurfaceDark,
            SysFlowPalette.SurfaceContainerLowDark to SysFlowPalette.OnSurfaceDark,
            SysFlowPalette.SurfaceContainerDark to SysFlowPalette.OnSurfaceDark,
            SysFlowPalette.SurfaceContainerHighDark to SysFlowPalette.OnSurfaceDark,
            SysFlowPalette.SurfaceContainerHighestDark to SysFlowPalette.OnSurfaceDark,
            SysFlowPalette.InverseSurfaceDark to SysFlowPalette.InverseOnSurfaceDark,
            // Fixed roles are shared across light/dark schemes.
            SysFlowPalette.PrimaryFixed to SysFlowPalette.OnPrimaryFixed,
            SysFlowPalette.PrimaryFixedDim to SysFlowPalette.OnPrimaryFixed,
            SysFlowPalette.PrimaryFixed to SysFlowPalette.OnPrimaryFixedVariant,
            SysFlowPalette.SecondaryFixed to SysFlowPalette.OnSecondaryFixed,
            SysFlowPalette.SecondaryFixedDim to SysFlowPalette.OnSecondaryFixed,
            SysFlowPalette.SecondaryFixed to SysFlowPalette.OnSecondaryFixedVariant,
            SysFlowPalette.TertiaryFixed to SysFlowPalette.OnTertiaryFixed,
            SysFlowPalette.TertiaryFixedDim to SysFlowPalette.OnTertiaryFixed,
            SysFlowPalette.TertiaryFixed to SysFlowPalette.OnTertiaryFixedVariant,
        )

        assertReadablePairs(pairs, "Material role")
    }

    @Test
    fun `extended telemetry states meet readable contrast in both themes`() {
        val pairs = listOf(
            LightSysFlowExtendedColors.success to LightSysFlowExtendedColors.onSuccess,
            LightSysFlowExtendedColors.successContainer to LightSysFlowExtendedColors.onSuccessContainer,
            LightSysFlowExtendedColors.warning to LightSysFlowExtendedColors.onWarning,
            LightSysFlowExtendedColors.warningContainer to LightSysFlowExtendedColors.onWarningContainer,
            LightSysFlowExtendedColors.info to LightSysFlowExtendedColors.onInfo,
            LightSysFlowExtendedColors.infoContainer to LightSysFlowExtendedColors.onInfoContainer,
            DarkSysFlowExtendedColors.success to DarkSysFlowExtendedColors.onSuccess,
            DarkSysFlowExtendedColors.successContainer to DarkSysFlowExtendedColors.onSuccessContainer,
            DarkSysFlowExtendedColors.warning to DarkSysFlowExtendedColors.onWarning,
            DarkSysFlowExtendedColors.warningContainer to DarkSysFlowExtendedColors.onWarningContainer,
            DarkSysFlowExtendedColors.info to DarkSysFlowExtendedColors.onInfo,
            DarkSysFlowExtendedColors.infoContainer to DarkSysFlowExtendedColors.onInfoContainer,
            // Accent badges/buttons drawn on a primaryContainer surface (label = the container colour).
            LightSysFlowExtendedColors.containerAccent to SysFlowPalette.PrimaryContainerLight,
            DarkSysFlowExtendedColors.containerAccent to SysFlowPalette.PrimaryContainerDark,
            // Regular on-surface text over the soft tint used by the overview core tiles.
            LightSysFlowExtendedColors.containerTint to SysFlowPalette.OnSurfaceLight,
            DarkSysFlowExtendedColors.containerTint to SysFlowPalette.OnSurfaceDark,
        )

        assertReadablePairs(pairs, "Telemetry state")
    }

    private fun assertReadablePairs(pairs: List<Pair<Color, Color>>, description: String) {
        pairs.forEachIndexed { index, (background, foreground) ->
            assertTrue(
                "$description color pair $index has insufficient contrast: ${contrast(background, foreground)}",
                contrast(background, foreground) >= 4.5,
            )
        }
    }

    private fun contrast(background: Color, foreground: Color): Double {
        val first = luminance(background)
        val second = luminance(foreground)
        val lighter = maxOf(first, second)
        val darker = minOf(first, second)
        return (lighter + 0.05) / (darker + 0.05)
    }

    private fun luminance(color: Color): Double {
        fun linear(channel: Float): Double {
            val value = channel.toDouble()
            return if (value <= 0.04045) value / 12.92 else Math.pow((value + 0.055) / 1.055, 2.4)
        }

        return 0.2126 * linear(color.red) +
            0.7152 * linear(color.green) +
            0.0722 * linear(color.blue)
    }
}
