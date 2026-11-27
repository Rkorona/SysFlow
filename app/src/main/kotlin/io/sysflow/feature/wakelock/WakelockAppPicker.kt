@file:OptIn(ExperimentalMaterial3Api::class, ExperimentalMaterial3ExpressiveApi::class)

package io.sysflow.feature.wakelock

import android.content.Context
import android.content.pm.ApplicationInfo
import android.content.pm.PackageManager
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.asPaddingValues
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.navigationBars
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.Block
import androidx.compose.material.icons.rounded.Close
import androidx.compose.material.icons.rounded.Lock
import androidx.compose.material.icons.rounded.Search
import androidx.compose.material.icons.rounded.SearchOff
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.ExperimentalMaterial3ExpressiveApi
import androidx.compose.material3.FilterChip
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.LoadingIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.Text
import androidx.compose.material3.TextField
import androidx.compose.material3.TextFieldDefaults
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.produceState
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import io.sysflow.core.model.isProtectedPackage
import io.sysflow.core.ui.components.AppIcon
import io.sysflow.core.ui.components.IconBadge
import io.sysflow.core.ui.components.InfoPill
import io.sysflow.core.ui.components.resolveAppIdentity
import io.sysflow.core.ui.components.segmentedShape
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext

/** One installed app as the picker sees it. */
private data class AppEntry(
    val packageName: String,
    val label: String,
    val system: Boolean,
    val icon: ImageBitmap?,
)

/** What the sheet is picking: apps to block, or apps to whitelist. */
internal enum class WakelockPickerMode { Block, Exempt }

private enum class PickerFilter(val label: String) {
    All("全部"),
    Blocked("已拦截"),
    User("用户应用"),
    System("系统应用"),
}

/**
 * Bottom sheet for choosing which apps to block.
 *
 * Installed apps are resolved off the main thread (labels and icons for a few hundred packages is
 * not free) and the list shows a loading indicator until they are ready. Rows are sorted by name
 * once and never re-sorted by blocked state, so toggling an app never makes the list jump.
 */
@Composable
internal fun WakelockAppPickerSheet(
    blocked: Set<String>,
    pending: String?,
    /** Policy published by the module (`wakelock.protected`); empty means "fall back locally". */
    protectedPackages: Set<String> = emptySet(),
    mode: WakelockPickerMode = WakelockPickerMode.Block,
    exempt: Set<String> = emptySet(),
    onToggle: (String, Boolean) -> Unit,
    onDismiss: () -> Unit,
) {
    val context = LocalContext.current
    val apps by produceState<List<AppEntry>?>(initialValue = null, protectedPackages) {
        value = withContext(Dispatchers.Default) {
            loadInstalledApps(context, protectedPackages)
        }
    }
    var query by remember { mutableStateOf("") }
    var filter by remember { mutableStateOf(PickerFilter.All) }
    // In exempt mode the switch means "on the whitelist", so the filter and the rows follow it.
    val selected = if (mode == WakelockPickerMode.Exempt) exempt else blocked
    val filtered = remember(apps, query, filter, selected) {
        apps.orEmpty().filter { app ->
            val matchesFilter = when (filter) {
                PickerFilter.All -> true
                PickerFilter.Blocked -> app.packageName in selected
                PickerFilter.User -> !app.system
                PickerFilter.System -> app.system
            }
            matchesFilter && (
                query.isBlank() ||
                    app.label.contains(query, ignoreCase = true) ||
                    app.packageName.contains(query, ignoreCase = true)
                )
        }
    }
    val sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true)
    val bottomInset = WindowInsets.navigationBars.asPaddingValues().calculateBottomPadding()
    val scheme = MaterialTheme.colorScheme

    ModalBottomSheet(
        onDismissRequest = onDismiss,
        sheetState = sheetState,
        containerColor = scheme.surfaceContainerLow,
    ) {
        Column(
            modifier = Modifier
                .fillMaxWidth()
                .fillMaxHeight(0.9f)
                .imePadding()
                .padding(horizontal = 16.dp),
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            Row(
                modifier = Modifier.fillMaxWidth(),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(12.dp),
            ) {
                IconBadge(
                    icon = if (mode == WakelockPickerMode.Exempt) {
                        Icons.Rounded.Lock
                    } else {
                        Icons.Rounded.Block
                    },
                    container = scheme.tertiaryContainer,
                    content = scheme.onTertiaryContainer,
                    size = 48.dp,
                )
                Column(modifier = Modifier.weight(1f)) {
                    Text(
                        if (mode == WakelockPickerMode.Exempt) "保护应用" else "拦截唤醒锁",
                        style = MaterialTheme.typography.titleLargeEmphasized,
                    )
                    Text(
                        if (mode == WakelockPickerMode.Exempt) {
                            "保护后该应用永远不会被拦截；已拦截的会同时解除。"
                        } else {
                            "按应用把 WAKE_LOCK 设为 ignore；系统关键应用不可选。"
                        },
                        style = MaterialTheme.typography.bodySmall,
                        color = scheme.onSurfaceVariant,
                    )
                }
                InfoPill(
                    text = if (mode == WakelockPickerMode.Exempt) {
                        "已保护 ${exempt.size}"
                    } else {
                        "已拦截 ${blocked.size}"
                    },
                    container = scheme.tertiaryContainer,
                    content = scheme.onTertiaryContainer,
                )
            }

            TextField(
                value = query,
                onValueChange = { query = it },
                modifier = Modifier.fillMaxWidth(),
                singleLine = true,
                shape = CircleShape,
                placeholder = { Text("搜索应用名或包名") },
                leadingIcon = { Icon(Icons.Rounded.Search, contentDescription = null) },
                trailingIcon = {
                    if (query.isNotEmpty()) {
                        IconButton(onClick = { query = "" }) {
                            Icon(Icons.Rounded.Close, contentDescription = "清除搜索")
                        }
                    }
                },
                colors = TextFieldDefaults.colors(
                    focusedContainerColor = scheme.surfaceContainerHigh,
                    unfocusedContainerColor = scheme.surfaceContainerHigh,
                    disabledContainerColor = scheme.surfaceContainerHigh,
                    focusedIndicatorColor = Color.Transparent,
                    unfocusedIndicatorColor = Color.Transparent,
                    disabledIndicatorColor = Color.Transparent,
                    errorIndicatorColor = Color.Transparent,
                ),
            )

            Row(
                modifier = Modifier
                    .fillMaxWidth()
                    .horizontalScroll(rememberScrollState()),
                horizontalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                PickerFilter.entries.forEach { entry ->
                    FilterChip(
                        selected = filter == entry,
                        onClick = { filter = entry },
                        label = {
                            Text(
                                if (mode == WakelockPickerMode.Exempt && entry == PickerFilter.Blocked) {
                                    "已保护"
                                } else {
                                    entry.label
                                },
                            )
                        },
                    )
                }
            }

            when {
                apps == null -> Box(
                    modifier = Modifier
                        .fillMaxWidth()
                        .weight(1f),
                    contentAlignment = Alignment.Center,
                ) { LoadingIndicator() }

                filtered.isEmpty() -> Column(
                    modifier = Modifier
                        .fillMaxWidth()
                        .weight(1f),
                    horizontalAlignment = Alignment.CenterHorizontally,
                    verticalArrangement = Arrangement.Center,
                ) {
                    Box(
                        modifier = Modifier.size(72.dp),
                        contentAlignment = Alignment.Center,
                    ) {
                        Icon(
                            Icons.Rounded.SearchOff,
                            contentDescription = null,
                            tint = scheme.onSurfaceVariant,
                            modifier = Modifier.size(40.dp),
                        )
                    }
                    Text(
                        "没有匹配的应用",
                        style = MaterialTheme.typography.titleMediumEmphasized,
                        color = scheme.onSurfaceVariant,
                    )
                }

                else -> LazyColumn(
                    modifier = Modifier
                        .fillMaxWidth()
                        .weight(1f),
                    contentPadding = PaddingValues(bottom = 16.dp + bottomInset),
                    verticalArrangement = Arrangement.spacedBy(4.dp),
                ) {
                    itemsIndexed(filtered, key = { _, app -> app.packageName }) { index, app ->
                        PickerRow(
                            app = app,
                            isSelected = app.packageName in selected,
                            isPending = pending == app.packageName,
                            protectedPackages = protectedPackages,
                            shapeIndex = index,
                            shapeCount = filtered.size,
                            onToggle = onToggle,
                            modifier = Modifier.animateItem(),
                        )
                    }
                }
            }
        }
    }
}

@Composable
private fun PickerRow(
    app: AppEntry,
    isSelected: Boolean,
    isPending: Boolean,
    protectedPackages: Set<String>,
    shapeIndex: Int,
    shapeCount: Int,
    onToggle: (String, Boolean) -> Unit,
    modifier: Modifier = Modifier,
) {
    val scheme = MaterialTheme.colorScheme
    val protected = isProtectedPackage(app.packageName, protectedPackages)
    val interactive = !protected && !isPending
    ConnectedItem(
        shape = segmentedShape(shapeIndex, shapeCount),
        modifier = modifier,
        container = if (isSelected) scheme.tertiaryContainer else scheme.surfaceContainerHigh,
        onClick = { onToggle(app.packageName, !isSelected) },
        enabled = interactive,
    ) {
        AppIcon(icon = app.icon, size = 40.dp, highlight = isSelected)
        Column(
            modifier = Modifier.weight(1f),
            verticalArrangement = Arrangement.spacedBy(2.dp),
        ) {
            Row(
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(6.dp),
            ) {
                Text(
                    app.label,
                    modifier = Modifier.weight(1f, fill = false),
                    style = MaterialTheme.typography.titleSmall,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
                if (protected) {
                    Icon(
                        Icons.Rounded.Lock,
                        contentDescription = "系统关键应用",
                        tint = scheme.onSurfaceVariant,
                        modifier = Modifier.size(14.dp),
                    )
                }
            }
            Text(
                if (protected) "${app.packageName} · 系统关键应用，已受保护" else app.packageName,
                style = MaterialTheme.typography.labelSmall,
                color = if (isSelected) {
                    scheme.onTertiaryContainer.copy(alpha = 0.8f)
                } else {
                    scheme.onSurfaceVariant
                },
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
        }
        if (isPending) {
            Box(Modifier.size(width = 52.dp, height = 32.dp), contentAlignment = Alignment.Center) {
                LoadingIndicator(modifier = Modifier.size(28.dp))
            }
        } else {
            BlockSwitch(
                blocked = isSelected,
                enabled = !protected,
                onToggle = { want -> onToggle(app.packageName, want) },
            )
        }
    }
}

/** Every installed package with its label and icon, protected apps last, then by name. */
private fun loadInstalledApps(
    context: Context,
    protectedPackages: Set<String>,
): List<AppEntry> {
    val manager = context.packageManager
    return manager.getInstalledApplications(PackageManager.GET_META_DATA)
        .map { info ->
            val identity = resolveAppIdentity(context, info.packageName)
            AppEntry(
                packageName = info.packageName,
                label = identity.label,
                system = (info.flags and ApplicationInfo.FLAG_SYSTEM) != 0,
                icon = identity.icon,
            )
        }
        .sortedWith(
            compareBy(
                { isProtectedPackage(it.packageName, protectedPackages) },
                { it.label.lowercase() },
            ),
        )
}
