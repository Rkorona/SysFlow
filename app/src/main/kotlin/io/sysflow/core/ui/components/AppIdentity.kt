package io.sysflow.core.ui.components

import android.content.Context
import android.content.pm.PackageManager
import android.os.Build
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.size
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.Android
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.core.graphics.drawable.toBitmap

/** An installed app's user-visible label and launcher icon, resolved from its package name. */
data class AppIdentity(val label: String, val icon: ImageBitmap?)

/**
 * Resolve [packageName] to its app label and launcher icon.
 *
 * The telemetry layers (BatteryStats side, AppOps rules) only know package names and UIDs, so the UI
 * resolves them here. It degrades to the raw package name with no icon when the package is not
 * installed or not visible, so a row always renders something.
 */
fun resolveAppIdentity(context: Context, packageName: String): AppIdentity {
    val pm = context.packageManager
    return runCatching {
        val info = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            pm.getApplicationInfo(packageName, PackageManager.ApplicationInfoFlags.of(0))
        } else {
            @Suppress("DEPRECATION")
            pm.getApplicationInfo(packageName, 0)
        }
        val label = pm.getApplicationLabel(info).toString().takeIf(String::isNotBlank) ?: packageName
        val icon = runCatching {
            pm.getApplicationIcon(info).toBitmap(width = 96, height = 96).asImageBitmap()
        }.getOrNull()
        AppIdentity(label, icon)
    }.getOrDefault(AppIdentity(packageName, null))
}

/**
 * Resolve every package in [packageNames] once and remember the result until the set changes.
 *
 * Resolution costs a PackageManager lookup plus icon decode per package, so it must never run per
 * row on every recomposition. Keying on the package set keeps it to one pass per distinct list.
 */
@Composable
fun rememberAppIdentities(packageNames: Collection<String>): Map<String, AppIdentity> {
    val context = LocalContext.current
    val packages = remember(packageNames.toSet()) { packageNames.toSet() }
    return remember(packages) { packages.associateWith { resolveAppIdentity(context, it) } }
}

/** Launcher icon for [identity], falling back to the generic Android glyph when it is unknown. */
@Composable
fun AppIcon(
    icon: ImageBitmap?,
    modifier: Modifier = Modifier,
    size: Dp = 32.dp,
    highlight: Boolean = false,
) {
    val shape = MaterialTheme.shapes.small
    if (icon != null) {
        Image(
            bitmap = icon,
            contentDescription = null,
            modifier = modifier
                .size(size)
                .clip(shape),
        )
        return
    }
    Box(
        modifier = modifier
            .size(size)
            .clip(shape)
            .background(
                if (highlight) {
                    MaterialTheme.colorScheme.primaryContainer
                } else {
                    MaterialTheme.colorScheme.surfaceContainerHighest
                },
            ),
        contentAlignment = Alignment.Center,
    ) {
        Icon(
            Icons.Rounded.Android,
            contentDescription = null,
            tint = if (highlight) {
                MaterialTheme.colorScheme.onPrimaryContainer
            } else {
                MaterialTheme.colorScheme.onSurfaceVariant
            },
            modifier = Modifier.size(size * 0.6f),
        )
    }
}
