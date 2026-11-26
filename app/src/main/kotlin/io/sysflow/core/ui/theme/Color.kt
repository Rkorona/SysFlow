package io.sysflow.core.ui.theme

import androidx.compose.runtime.Composable
import androidx.compose.runtime.Immutable
import androidx.compose.runtime.ReadOnlyComposable
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.material3.MaterialTheme
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.lerp

/**
 * SysFlow brand palette.
 *
 * Primary: silicon teal — control, stability and efficiency. Large brand surfaces
 * (primaryContainer) use #006B5B; the primary role itself is a deeper teal (#004D41) so
 * controls stay distinct from those surfaces. Accents drawn *on* a primaryContainer
 * surface use [SysFlowExtendedColors.containerAccent] instead of `primary`.
 * Secondary: cool blue — telemetry and information.
 * Tertiary: warm amber — boost, thermal and attention states.
 * Neutrals use a subtle green/graphite undertone so the interface feels technical
 * without becoming the generic purple Material starter theme.
 */
internal object SysFlowPalette {
    // Light brand roles
    val PrimaryLight = Color(0xFF004D41)
    val OnPrimaryLight = Color(0xFFFFFFFF)
    val PrimaryContainerLight = Color(0xFF006B5B)
    val OnPrimaryContainerLight = Color(0xFFFFFFFF)
    val InversePrimaryLight = Color(0xFF70DCC2)

    val SecondaryLight = Color(0xFF4D6078)
    val OnSecondaryLight = Color(0xFFFFFFFF)
    val SecondaryContainerLight = Color(0xFFD7E5F8)
    val OnSecondaryContainerLight = Color(0xFF091D32)

    val TertiaryLight = Color(0xFF805500)
    val OnTertiaryLight = Color(0xFFFFFFFF)
    val TertiaryContainerLight = Color(0xFFFFDEA6)
    val OnTertiaryContainerLight = Color(0xFF291800)

    // Fixed roles intentionally keep the same semantic tone in both themes.
    val PrimaryFixed = Color(0xFF006B5B)
    val PrimaryFixedDim = Color(0xFF004D41)
    val OnPrimaryFixed = Color(0xFFFFFFFF)
    val OnPrimaryFixedVariant = Color(0xFFBFF3E5)
    val SecondaryFixed = Color(0xFFD7E5F8)
    val SecondaryFixedDim = Color(0xFFB8C9E2)
    val OnSecondaryFixed = Color(0xFF091D32)
    val OnSecondaryFixedVariant = Color(0xFF364960)
    val TertiaryFixed = Color(0xFFFFDEA6)
    val TertiaryFixedDim = Color(0xFFEFC16F)
    val OnTertiaryFixed = Color(0xFF291800)
    val OnTertiaryFixedVariant = Color(0xFF624000)

    val ErrorLight = Color(0xFFBA1A1A)
    val OnErrorLight = Color(0xFFFFFFFF)
    val ErrorContainerLight = Color(0xFFFFDAD6)
    val OnErrorContainerLight = Color(0xFF410002)

    val BackgroundLight = Color(0xFFF5F8F6)
    val OnBackgroundLight = Color(0xFF171D1B)
    val SurfaceLight = Color(0xFFF5F8F6)
    val OnSurfaceLight = Color(0xFF171D1B)
    val SurfaceVariantLight = Color(0xFFDCE5E0)
    val OnSurfaceVariantLight = Color(0xFF404A45)
    val SurfaceTintLight = PrimaryLight
    val InverseSurfaceLight = Color(0xFF2C3330)
    val InverseOnSurfaceLight = Color(0xFFEEF2EF)
    val OutlineLight = Color(0xFF707B75)
    val OutlineVariantLight = Color(0xFFC0CAC4)
    val ScrimLight = Color(0xFF000000)
    val SurfaceDimLight = Color(0xFFD6DED9)
    val SurfaceBrightLight = Color(0xFFF5F8F6)
    val SurfaceContainerLowestLight = Color(0xFFFFFFFF)
    val SurfaceContainerLowLight = Color(0xFFEFF4F0)
    val SurfaceContainerLight = Color(0xFFE9EFEB)
    val SurfaceContainerHighLight = Color(0xFFE3EAE5)
    val SurfaceContainerHighestLight = Color(0xFFDDE5E0)

    // Dark brand roles
    val PrimaryDark = Color(0xFF79DCC2)
    val OnPrimaryDark = Color(0xFF00382E)
    val PrimaryContainerDark = Color(0xFF005143)
    val OnPrimaryContainerDark = Color(0xFF9AF0D8)
    val InversePrimaryDark = Color(0xFF004D41)

    val SecondaryDark = Color(0xFFB8C9E2)
    val OnSecondaryDark = Color(0xFF213247)
    val SecondaryContainerDark = Color(0xFF364960)
    val OnSecondaryContainerDark = Color(0xFFD7E5F8)

    val TertiaryDark = Color(0xFFEFC16F)
    val OnTertiaryDark = Color(0xFF452B00)
    val TertiaryContainerDark = Color(0xFF624000)
    val OnTertiaryContainerDark = Color(0xFFFFDEA6)

    val ErrorDark = Color(0xFFFFB4AB)
    val OnErrorDark = Color(0xFF690005)
    val ErrorContainerDark = Color(0xFF93000A)
    val OnErrorContainerDark = Color(0xFFFFDAD6)

    val BackgroundDark = Color(0xFF101513)
    val OnBackgroundDark = Color(0xFFE0E7E2)
    val SurfaceDark = Color(0xFF101513)
    val OnSurfaceDark = Color(0xFFE0E7E2)
    val SurfaceVariantDark = Color(0xFF404A45)
    val OnSurfaceVariantDark = Color(0xFFC0CAC4)
    val SurfaceTintDark = PrimaryDark
    val InverseSurfaceDark = Color(0xFFE0E7E2)
    val InverseOnSurfaceDark = Color(0xFF2C3330)
    val OutlineDark = Color(0xFF8A958F)
    val OutlineVariantDark = Color(0xFF404A45)
    val ScrimDark = Color(0xFF000000)
    val SurfaceDimDark = Color(0xFF101513)
    val SurfaceBrightDark = Color(0xFF363D39)
    val SurfaceContainerLowestDark = Color(0xFF0B100E)
    val SurfaceContainerLowDark = Color(0xFF191F1C)
    val SurfaceContainerDark = Color(0xFF1D2420)
    val SurfaceContainerHighDark = Color(0xFF272E2A)
    val SurfaceContainerHighestDark = Color(0xFF323A35)
}

/**
 * Per-module tone. Drives the hub/overview card colours *and* the module's detail pages
 * (see ModuleTheme), so a module looks the same everywhere. CPU keeps the original brand teal.
 * [accent] is drawn on top of [container] (meters, status dots); [warn] is the warning text
 * colour that stays readable on that container; [tint] is the soft fill used by charts/tiles.
 */
@Immutable
data class ModuleTone(
    val container: Color,
    val onContainer: Color,
    val accent: Color,
    val warn: Color,
    val primary: Color,
    val onPrimary: Color,
    val secondaryContainer: Color,
    val onSecondaryContainer: Color,
    val tint: Color,
)

@Immutable
data class ModuleTones(
    val cpu: ModuleTone,
    val battery: ModuleTone,
    val mem: ModuleTone,
    val storage: ModuleTone,
    val process: ModuleTone,
    val wakelock: ModuleTone,
) {
    fun forModule(id: String): ModuleTone? = when (id) {
        "cpu" -> cpu
        "battery" -> battery
        "mem" -> mem
        "storage" -> storage
        "process" -> process
        "wakelock" -> wakelock
        else -> null
    }
}

private fun lightTone(
    container: Long, accent: Long, warn: Long, primary: Long, secC: Long, onSecC: Long,
    tint: Color? = null,
): ModuleTone {
    val c = Color(container)
    return ModuleTone(
        container = c,
        onContainer = Color(0xFFFFFFFF),
        accent = Color(accent),
        warn = Color(warn),
        primary = Color(primary),
        onPrimary = Color(0xFFFFFFFF),
        secondaryContainer = Color(secC),
        onSecondaryContainer = Color(onSecC),
        tint = tint ?: lerp(c, Color.White, 0.62f),
    )
}

private fun darkTone(
    container: Long, onContainer: Long, accent: Long, warn: Long, primary: Long, onPrimary: Long,
    secC: Long, onSecC: Long,
): ModuleTone = ModuleTone(
    container = Color(container),
    onContainer = Color(onContainer),
    accent = Color(accent),
    warn = Color(warn),
    primary = Color(primary),
    onPrimary = Color(onPrimary),
    secondaryContainer = Color(secC),
    onSecondaryContainer = Color(onSecC),
    tint = Color(container),
)

internal val LightModuleTones = ModuleTones(
    cpu = lightTone(0xFF006B5B, 0xFF8CE9D1, 0xFFFFD76A, 0xFF004D41, 0xFFD7E5F8, 0xFF091D32, tint = Color(0xFF9FC4BC)),
    battery = lightTone(0xFF8A5000, 0xFFFFC774, 0xFFFFC2B8, 0xFF6B3C00, 0xFFFFDDB8, 0xFF2B1700),
    mem = lightTone(0xFF2A5EA8, 0xFFA9C8FF, 0xFFFFD76A, 0xFF0F4A8A, 0xFFD6E3FF, 0xFF001B3E),
    storage = lightTone(0xFF6B4FA0, 0xFFD5BEFF, 0xFFFFD76A, 0xFF4F3787, 0xFFE9DDFF, 0xFF22005D),
    process = lightTone(0xFFA23B5E, 0xFFFFB1C8, 0xFFFFE08A, 0xFF80274A, 0xFFFFD9E2, 0xFF3E001D),
    wakelock = lightTone(0xFF5C6A00, 0xFFD2E26A, 0xFFFFD0C2, 0xFF464F00, 0xFFDDE98F, 0xFF191E00),
)

internal val DarkModuleTones = ModuleTones(
    cpu = darkTone(0xFF005143, 0xFF9AF0D8, 0xFF79DCC2, 0xFFEFC16F, 0xFF79DCC2, 0xFF00382E, 0xFF364960, 0xFFD7E5F8),
    battery = darkTone(0xFF5F3A00, 0xFFFFDDB3, 0xFFFFB95C, 0xFFFFB4A6, 0xFFFFB95C, 0xFF462A00, 0xFF5B3E14, 0xFFFFDDB8),
    mem = darkTone(0xFF1B4A82, 0xFFD6E3FF, 0xFFA8C8FF, 0xFFEFC16F, 0xFFA8C8FF, 0xFF00305F, 0xFF304878, 0xFFD6E3FF),
    storage = darkTone(0xFF4E3A82, 0xFFE9DDFF, 0xFFD0BCFF, 0xFFEFC16F, 0xFFD0BCFF, 0xFF381E72, 0xFF4D3E7D, 0xFFE9DDFF),
    process = darkTone(0xFF7A2A47, 0xFFFFD9E2, 0xFFFFB1C8, 0xFFEFC16F, 0xFFFFB1C8, 0xFF5E1133, 0xFF6A3248, 0xFFFFD9E2),
    wakelock = darkTone(0xFF4A5200, 0xFFE1EC9A, 0xFFC9D668, 0xFFFFB4A6, 0xFFC9D668, 0xFF2F3300, 0xFF454B1B, 0xFFE1EC9A),
)

/** Extra semantic colors for device telemetry states beyond the standard M3 roles. */
@Immutable
data class SysFlowExtendedColors(
    val success: Color,
    val onSuccess: Color,
    val successContainer: Color,
    val onSuccessContainer: Color,
    val warning: Color,
    val onWarning: Color,
    val warningContainer: Color,
    val onWarningContainer: Color,
    val info: Color,
    val onInfo: Color,
    val infoContainer: Color,
    val onInfoContainer: Color,
    val boost: Color,
    /** Accent (progress, sliders, badges) to draw on top of a primaryContainer surface. */
    val containerAccent: Color,
    /** Soft primary-tinted fill for charts/tiles that carry regular on-surface text. */
    val containerTint: Color,
    val tones: ModuleTones,
)

internal val LightSysFlowExtendedColors = SysFlowExtendedColors(
    success = Color(0xFF006B4F),
    onSuccess = Color(0xFFFFFFFF),
    successContainer = Color(0xFFA4F2CE),
    onSuccessContainer = Color(0xFF002114),
    warning = Color(0xFF805600),
    onWarning = Color(0xFFFFFFFF),
    warningContainer = Color(0xFFFFDEA6),
    onWarningContainer = Color(0xFF291800),
    info = Color(0xFF405F91),
    onInfo = Color(0xFFFFFFFF),
    infoContainer = Color(0xFFD8E2FF),
    onInfoContainer = Color(0xFF001A41),
    boost = Color(0xFF004D41),
    containerAccent = Color(0xFF8CE9D1),
    containerTint = Color(0xFF9FC4BC),
    tones = LightModuleTones,
)

internal val DarkSysFlowExtendedColors = SysFlowExtendedColors(
    success = Color(0xFF83D9B5),
    onSuccess = Color(0xFF003826),
    successContainer = Color(0xFF00513A),
    onSuccessContainer = Color(0xFFA4F2CE),
    warning = Color(0xFFEFC16F),
    onWarning = Color(0xFF452B00),
    warningContainer = Color(0xFF624000),
    onWarningContainer = Color(0xFFFFDEA6),
    info = Color(0xFFB1C7FF),
    onInfo = Color(0xFF1A315D),
    infoContainer = Color(0xFF304875),
    onInfoContainer = Color(0xFFD8E2FF),
    boost = Color(0xFF79DCC2),
    containerAccent = Color(0xFF79DCC2),
    containerTint = Color(0xFF005143),
    tones = DarkModuleTones,
)

internal val LocalSysFlowExtendedColors = staticCompositionLocalOf { LightSysFlowExtendedColors }

/** Access semantic SysFlow colors from any composable: MaterialTheme.sysFlowColors.warning, etc. */
val MaterialTheme.sysFlowColors: SysFlowExtendedColors
    @Composable
    @ReadOnlyComposable
    get() = LocalSysFlowExtendedColors.current
