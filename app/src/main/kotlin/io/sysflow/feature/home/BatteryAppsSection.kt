package io.sysflow.feature.home

import android.content.Context
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.platform.LocalContext
import io.sysflow.core.model.BatteryAppEntry
import io.sysflow.core.model.BatteryApps
import io.sysflow.core.ui.components.AppIdentity
import io.sysflow.core.ui.components.resolveAppIdentity

/**
 * Loads the BatteryStats ranking once and on demand.
 *
 * A fresh dump costs hundreds of milliseconds behind a 60 s module-side cache, so the page never
 * polls it: it loads on entry and when the user taps refresh.
 */
class BatteryAppsState(
    val page: BatteryApps?,
    val loading: Boolean,
    val refreshing: Boolean,
    val failed: Boolean,
    val refresh: () -> Unit,
) {
    val entries: List<BatteryAppEntry> get() = page?.entries.orEmpty()
}

@Composable
fun rememberBatteryAppsState(loadApps: suspend (refresh: Boolean) -> BatteryApps?): BatteryAppsState {
    var page by remember { mutableStateOf<BatteryApps?>(null) }
    var loading by remember { mutableStateOf(true) }
    var refreshing by remember { mutableStateOf(false) }
    var failed by remember { mutableStateOf(false) }
    var refreshToken by remember { mutableStateOf(0) }

    LaunchedEffect(refreshToken) {
        val refresh = refreshToken > 0
        if (refresh) refreshing = true else loading = true
        val result = loadApps(refresh)
        page = result
        failed = result == null
        loading = false
        refreshing = false
    }

    return BatteryAppsState(
        page = page,
        loading = loading,
        refreshing = refreshing,
        failed = failed,
        refresh = { if (!refreshing && !loading) refreshToken += 1 },
    )
}

/**
 * Labels and icons are resolved once per load, never per recomposition. Without
 * `QUERY_ALL_PACKAGES` in the manifest most of these lookups throw and every row degrades to the raw
 * package name.
 */
@Composable
fun rememberAppInfos(entries: List<BatteryAppEntry>): Map<String, AppIdentity> {
    val context = LocalContext.current
    val key = remember(entries) { entries.joinToString("|") { it.uid } }
    return remember(key) {
        entries.associate { entry -> entry.uid to resolveAppInfo(context, entry) }
    }
}

private fun resolveAppInfo(context: Context, entry: BatteryAppEntry): AppIdentity {
    val packageName = entry.packages.firstOrNull()
    val fallback = entry.name ?: packageName ?: entry.uid
    if (packageName == null) {
        return AppIdentity(fallback, null)
    }
    val identity = resolveAppIdentity(context, packageName)
    return AppIdentity(
        label = identity.label.takeIf { it != packageName } ?: fallback,
        icon = identity.icon,
    )
}
