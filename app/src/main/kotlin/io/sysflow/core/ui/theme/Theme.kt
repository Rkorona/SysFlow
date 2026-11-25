@file:OptIn(androidx.compose.material3.ExperimentalMaterial3ExpressiveApi::class)

package io.sysflow.core.ui.theme

import android.os.Build
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.material3.ExperimentalMaterial3ExpressiveApi
import androidx.compose.material3.MaterialExpressiveTheme
import androidx.compose.material3.MotionScheme
import androidx.compose.material3.LocalContentColor
import androidx.compose.material3.Shapes
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.dynamicDarkColorScheme
import androidx.compose.material3.dynamicLightColorScheme
import androidx.compose.material3.expressiveLightColorScheme
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.dp

private val SysFlowLightColorScheme = expressiveLightColorScheme().copy(
    primary = SysFlowPalette.PrimaryLight,
    onPrimary = SysFlowPalette.OnPrimaryLight,
    primaryContainer = SysFlowPalette.PrimaryContainerLight,
    onPrimaryContainer = SysFlowPalette.OnPrimaryContainerLight,
    inversePrimary = SysFlowPalette.InversePrimaryLight,
    secondary = SysFlowPalette.SecondaryLight,
    onSecondary = SysFlowPalette.OnSecondaryLight,
    secondaryContainer = SysFlowPalette.SecondaryContainerLight,
    onSecondaryContainer = SysFlowPalette.OnSecondaryContainerLight,
    tertiary = SysFlowPalette.TertiaryLight,
    onTertiary = SysFlowPalette.OnTertiaryLight,
    tertiaryContainer = SysFlowPalette.TertiaryContainerLight,
    onTertiaryContainer = SysFlowPalette.OnTertiaryContainerLight,
    primaryFixed = SysFlowPalette.PrimaryFixed,
    primaryFixedDim = SysFlowPalette.PrimaryFixedDim,
    onPrimaryFixed = SysFlowPalette.OnPrimaryFixed,
    onPrimaryFixedVariant = SysFlowPalette.OnPrimaryFixedVariant,
    secondaryFixed = SysFlowPalette.SecondaryFixed,
    secondaryFixedDim = SysFlowPalette.SecondaryFixedDim,
    onSecondaryFixed = SysFlowPalette.OnSecondaryFixed,
    onSecondaryFixedVariant = SysFlowPalette.OnSecondaryFixedVariant,
    tertiaryFixed = SysFlowPalette.TertiaryFixed,
    tertiaryFixedDim = SysFlowPalette.TertiaryFixedDim,
    onTertiaryFixed = SysFlowPalette.OnTertiaryFixed,
    onTertiaryFixedVariant = SysFlowPalette.OnTertiaryFixedVariant,
    error = SysFlowPalette.ErrorLight,
    onError = SysFlowPalette.OnErrorLight,
    errorContainer = SysFlowPalette.ErrorContainerLight,
    onErrorContainer = SysFlowPalette.OnErrorContainerLight,
    background = SysFlowPalette.BackgroundLight,
    onBackground = SysFlowPalette.OnBackgroundLight,
    surface = SysFlowPalette.SurfaceLight,
    onSurface = SysFlowPalette.OnSurfaceLight,
    surfaceVariant = SysFlowPalette.SurfaceVariantLight,
    onSurfaceVariant = SysFlowPalette.OnSurfaceVariantLight,
    surfaceTint = SysFlowPalette.SurfaceTintLight,
    inverseSurface = SysFlowPalette.InverseSurfaceLight,
    inverseOnSurface = SysFlowPalette.InverseOnSurfaceLight,
    outline = SysFlowPalette.OutlineLight,
    outlineVariant = SysFlowPalette.OutlineVariantLight,
    scrim = SysFlowPalette.ScrimLight,
    surfaceDim = SysFlowPalette.SurfaceDimLight,
    surfaceBright = SysFlowPalette.SurfaceBrightLight,
    surfaceContainerLowest = SysFlowPalette.SurfaceContainerLowestLight,
    surfaceContainerLow = SysFlowPalette.SurfaceContainerLowLight,
    surfaceContainer = SysFlowPalette.SurfaceContainerLight,
    surfaceContainerHigh = SysFlowPalette.SurfaceContainerHighLight,
    surfaceContainerHighest = SysFlowPalette.SurfaceContainerHighestLight,
)

private val SysFlowDarkColorScheme = darkColorScheme(
    primary = SysFlowPalette.PrimaryDark,
    onPrimary = SysFlowPalette.OnPrimaryDark,
    primaryContainer = SysFlowPalette.PrimaryContainerDark,
    onPrimaryContainer = SysFlowPalette.OnPrimaryContainerDark,
    inversePrimary = SysFlowPalette.InversePrimaryDark,
    secondary = SysFlowPalette.SecondaryDark,
    onSecondary = SysFlowPalette.OnSecondaryDark,
    secondaryContainer = SysFlowPalette.SecondaryContainerDark,
    onSecondaryContainer = SysFlowPalette.OnSecondaryContainerDark,
    tertiary = SysFlowPalette.TertiaryDark,
    onTertiary = SysFlowPalette.OnTertiaryDark,
    tertiaryContainer = SysFlowPalette.TertiaryContainerDark,
    onTertiaryContainer = SysFlowPalette.OnTertiaryContainerDark,
    primaryFixed = SysFlowPalette.PrimaryFixed,
    primaryFixedDim = SysFlowPalette.PrimaryFixedDim,
    onPrimaryFixed = SysFlowPalette.OnPrimaryFixed,
    onPrimaryFixedVariant = SysFlowPalette.OnPrimaryFixedVariant,
    secondaryFixed = SysFlowPalette.SecondaryFixed,
    secondaryFixedDim = SysFlowPalette.SecondaryFixedDim,
    onSecondaryFixed = SysFlowPalette.OnSecondaryFixed,
    onSecondaryFixedVariant = SysFlowPalette.OnSecondaryFixedVariant,
    tertiaryFixed = SysFlowPalette.TertiaryFixed,
    tertiaryFixedDim = SysFlowPalette.TertiaryFixedDim,
    onTertiaryFixed = SysFlowPalette.OnTertiaryFixed,
    onTertiaryFixedVariant = SysFlowPalette.OnTertiaryFixedVariant,
    error = SysFlowPalette.ErrorDark,
    onError = SysFlowPalette.OnErrorDark,
    errorContainer = SysFlowPalette.ErrorContainerDark,
    onErrorContainer = SysFlowPalette.OnErrorContainerDark,
    background = SysFlowPalette.BackgroundDark,
    onBackground = SysFlowPalette.OnBackgroundDark,
    surface = SysFlowPalette.SurfaceDark,
    onSurface = SysFlowPalette.OnSurfaceDark,
    surfaceVariant = SysFlowPalette.SurfaceVariantDark,
    onSurfaceVariant = SysFlowPalette.OnSurfaceVariantDark,
    surfaceTint = SysFlowPalette.SurfaceTintDark,
    inverseSurface = SysFlowPalette.InverseSurfaceDark,
    inverseOnSurface = SysFlowPalette.InverseOnSurfaceDark,
    outline = SysFlowPalette.OutlineDark,
    outlineVariant = SysFlowPalette.OutlineVariantDark,
    scrim = SysFlowPalette.ScrimDark,
    surfaceDim = SysFlowPalette.SurfaceDimDark,
    surfaceBright = SysFlowPalette.SurfaceBrightDark,
    surfaceContainerLowest = SysFlowPalette.SurfaceContainerLowestDark,
    surfaceContainerLow = SysFlowPalette.SurfaceContainerLowDark,
    surfaceContainer = SysFlowPalette.SurfaceContainerDark,
    surfaceContainerHigh = SysFlowPalette.SurfaceContainerHighDark,
    surfaceContainerHighest = SysFlowPalette.SurfaceContainerHighestDark,
)

/** Shape scale used by Expressive components and all default Material surfaces. */
private val SysFlowShapes = Shapes(
    extraSmall = RoundedCornerShape(10.dp),
    small = RoundedCornerShape(14.dp),
    medium = RoundedCornerShape(20.dp),
    large = RoundedCornerShape(28.dp),
    extraLarge = RoundedCornerShape(36.dp),
    largeIncreased = RoundedCornerShape(32.dp),
    extraLargeIncreased = RoundedCornerShape(40.dp),
    extraExtraLarge = RoundedCornerShape(48.dp),
)

/**
 * Complete SysFlow Material 3 Expressive theme.
 * Dynamic wallpaper colors are deliberately opt-in so the SysFlow identity remains
 * consistent across devices; callers may enable them explicitly when desired.
 */
@OptIn(ExperimentalMaterial3ExpressiveApi::class)
@Composable
fun SysFlowTheme(
    darkTheme: Boolean = isSystemInDarkTheme(),
    dynamicColor: Boolean = false,
    content: @Composable () -> Unit,
) {
    val context = LocalContext.current
    val colorScheme = when {
        dynamicColor && Build.VERSION.SDK_INT >= Build.VERSION_CODES.S -> {
            if (darkTheme) dynamicDarkColorScheme(context) else dynamicLightColorScheme(context)
        }
        darkTheme -> SysFlowDarkColorScheme
        else -> SysFlowLightColorScheme
    }
    val extendedColors = if (darkTheme) DarkSysFlowExtendedColors else LightSysFlowExtendedColors

    CompositionLocalProvider(LocalSysFlowExtendedColors provides extendedColors) {
        MaterialExpressiveTheme(
            colorScheme = colorScheme,
            motionScheme = MotionScheme.expressive(),
            shapes = SysFlowShapes,
            typography = Typography,
        ) {
            // Screens are laid out on a background Box rather than a full-screen Surface.
            // Without this provider, Text(color = Color.Unspecified) falls back to black,
            // which becomes unreadable in dark theme. Nested Surface components continue
            // to supply their own on-container foreground colors as usual.
            CompositionLocalProvider(LocalContentColor provides colorScheme.onBackground) {
                content()
            }
        }
    }
}
