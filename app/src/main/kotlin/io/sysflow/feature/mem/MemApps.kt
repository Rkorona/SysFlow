@file:OptIn(ExperimentalMaterial3Api::class, ExperimentalMaterial3ExpressiveApi::class)

package io.sysflow.feature.mem

import androidx.compose.animation.animateContentSize
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.IntrinsicSize
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.asPaddingValues
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.navigationBars
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.Apps
import androidx.compose.material.icons.rounded.PieChart
import androidx.compose.material.icons.rounded.Warning
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.ButtonGroupDefaults
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.ExperimentalMaterial3ExpressiveApi
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.Text
import androidx.compose.material3.ToggleButton
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.Shape
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import io.sysflow.core.ui.components.AppIcon
import io.sysflow.core.ui.components.rememberAppIdentities
import io.sysflow.core.ui.components.segmentedShape
import java.util.Locale

/** The two on-demand views of per-process PSS. */
private enum class AppsView(val label: String, val icon: ImageVector) {
    Ranking("应用排行", Icons.Rounded.Apps),
    Split("系统 / 用户", Icons.Rounded.PieChart),
}

/**
 * Who is holding the memory: a per-app ranking and the system-vs-user split.
 *
 * Both views scan `/proc` per process, so neither runs with the page — each waits for a tap and
 * then keeps its result. They are peers in one connected toggle group rather than two stacked
 * cards, because only one of them is ever the answer to the question being asked.
 */
@Composable
internal fun AppsSection(
    state: MemViewModel.UiState,
    onLoadApps: (Boolean) -> Unit,
    onLoadBreakdown: (Boolean) -> Unit,
    onOpen: (MemAppEntry) -> Unit,
    modifier: Modifier = Modifier,
) {
    var viewIndex by rememberSaveable { mutableIntStateOf(0) }
    val views = AppsView.entries
    val view = views[viewIndex.coerceIn(0, views.lastIndex)]

    Column(
        modifier = modifier
            .fillMaxWidth()
            .animateContentSize(),
        verticalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        SectionHeader("应用内存", supporting = "按进程 PSS 汇总，需要扫描全部进程，只在点击时读取")
        Row(
            modifier = Modifier.fillMaxWidth(),
            horizontalArrangement = Arrangement.spacedBy(ButtonGroupDefaults.ConnectedSpaceBetween),
        ) {
            views.forEachIndexed { index, entry ->
                val shapes = when (index) {
                    0 -> ButtonGroupDefaults.connectedLeadingButtonShapes()
                    views.lastIndex -> ButtonGroupDefaults.connectedTrailingButtonShapes()
                    else -> ButtonGroupDefaults.connectedMiddleButtonShapes()
                }
                ToggleButton(
                    checked = entry == view,
                    onCheckedChange = { viewIndex = entry.ordinal },
                    modifier = Modifier.weight(1f),
                    shapes = shapes,
                ) {
                    Icon(
                        imageVector = entry.icon,
                        contentDescription = null,
                        modifier = Modifier.size(ButtonDefaults.IconSize),
                    )
                    Spacer(Modifier.size(ButtonDefaults.IconSpacing))
                    Text(entry.label, maxLines = 1)
                }
            }
        }
        when (view) {
            AppsView.Ranking -> RankingPanel(state, onLoadApps, onOpen)
            AppsView.Split -> SplitPanel(state, onLoadBreakdown)
        }
    }
}

// ---- ranking -------------------------------------------------------------------------------

@Composable
private fun RankingPanel(
    state: MemViewModel.UiState,
    onLoad: (Boolean) -> Unit,
    onOpen: (MemAppEntry) -> Unit,
) {
    val apps = state.apps
    when {
        apps == null -> EmptyActionTile(
            icon = Icons.Rounded.Apps,
            text = "按进程扫描 /proc 后取 PSS 前几名，只在点击时读取。",
            actionLabel = if (state.appsLoading) "读取中…" else "加载排行",
            onAction = { onLoad(false) },
            loading = state.appsLoading,
        )

        !apps.available -> NoteTile("无法读取 /proc，应用排行不可用。", icon = Icons.Rounded.Warning)

        apps.entries.isEmpty() -> NoteTile("没有可显示的应用。")

        else -> RankingList(apps, loading = state.appsLoading, onRefresh = { onLoad(true) }, onOpen = onOpen)
    }
}

@Composable
private fun RankingList(
    apps: MemApps,
    loading: Boolean,
    onRefresh: () -> Unit,
    onOpen: (MemAppEntry) -> Unit,
) {
    // Icons are resolved once per distinct package set, never per row on recomposition.
    val identities = rememberAppIdentities(apps.entries.mapNotNull { it.packages.firstOrNull() })
    val top = (apps.entries.maxOfOrNull { it.pssKb } ?: 1L).coerceAtLeast(1L)

    Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
        Row(
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            Column(modifier = Modifier.weight(1f)) {
                Text(
                    "合计 ${formatKb(apps.totalPssKb)}",
                    style = MaterialTheme.typography.titleMediumEmphasized,
                )
                Text(
                    buildString {
                        append("前 ${apps.entries.size} 名")
                        if (apps.estimatedTail) append(" · 截断点以下按 RSS 估算")
                        if (apps.skipped > 0) append(" · 跳过 ${apps.skipped} 个不可读进程")
                        if (apps.cached) append(" · 缓存结果")
                    },
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
            RefreshAction(loading = loading, description = "刷新排行", onClick = onRefresh)
        }
        Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
            apps.entries.forEachIndexed { index, entry ->
                AppRow(
                    entry = entry,
                    icon = identities[entry.packages.firstOrNull()]?.icon,
                    relative = entry.pssKb.toFloat() / top,
                    highlight = index == 0,
                    shape = segmentedShape(index, apps.entries.size),
                    onClick = { onOpen(entry) },
                )
            }
        }
        NoteTile("按 UID 汇总，点一行看单应用明细；进程级列表与强制停止属于进程模块。")
    }
}

/** One ranked app: icon, name, a bar relative to the heaviest app, and its PSS and share. */
@Composable
private fun AppRow(
    entry: MemAppEntry,
    icon: ImageBitmap?,
    relative: Float,
    highlight: Boolean,
    shape: Shape,
    onClick: () -> Unit,
) {
    val scheme = MaterialTheme.colorScheme
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .clip(shape)
            .background(scheme.surfaceContainerHigh)
            .clickable(onClick = onClick)
            .padding(horizontal = 16.dp, vertical = 12.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(14.dp),
    ) {
        AppIcon(icon = icon, size = 44.dp, highlight = highlight)
        Column(modifier = Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(2.dp)) {
            Text(
                entry.name,
                style = MaterialTheme.typography.bodyLarge,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
            Text(
                buildString {
                    append("${entry.procs} 个进程")
                    if (entry.swapPssKb > 0) append(" · 换出 ${formatKbShort(entry.swapPssKb)}")
                },
                style = MaterialTheme.typography.bodySmall,
                color = scheme.onSurfaceVariant,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
            Box(
                Modifier
                    .padding(top = 4.dp)
                    .fillMaxWidth()
                    .height(4.dp)
                    .clip(CircleShape)
                    .background(scheme.surfaceContainerHighest),
            ) {
                Box(
                    Modifier
                        .fillMaxWidth(relative.coerceIn(0f, 1f))
                        .fillMaxHeight()
                        .background(if (highlight) scheme.primary else scheme.secondary),
                )
            }
        }
        Column(horizontalAlignment = Alignment.End) {
            Text(formatKbShort(entry.pssKb), style = MaterialTheme.typography.titleSmallEmphasized)
            Text(
                String.format(Locale.ROOT, "%.1f%%", entry.share * 100),
                style = MaterialTheme.typography.labelMedium,
                color = scheme.onSurfaceVariant,
            )
        }
    }
}

// ---- system / user split -------------------------------------------------------------------

/**
 * System vs user app memory (`sysflow mem breakdown`).
 *
 * Kept apart from the composition bar on purpose: this is a PSS sum over processes, which is a
 * different measurement from "used memory" — shared pages are counted fractionally per process and
 * kernel/GPU memory is not app memory at all. Two layers, two captions, no arithmetic between them.
 */
@Composable
private fun SplitPanel(state: MemViewModel.UiState, onLoad: (Boolean) -> Unit) {
    val breakdown = state.breakdown
    when {
        breakdown == null -> EmptyActionTile(
            icon = Icons.Rounded.PieChart,
            text = "按进程 PSS 汇总「系统应用」与「用户应用」，需要扫描全部进程（约半秒）。",
            actionLabel = if (state.breakdownLoading) "分析中…" else "开始分析",
            onAction = { onLoad(false) },
            loading = state.breakdownLoading,
        )

        !breakdown.available -> NoteTile("无法读取 /proc，应用内存分解不可用。", icon = Icons.Rounded.Warning)

        else -> SplitResult(breakdown, loading = state.breakdownLoading, onRefresh = { onLoad(true) })
    }
}

@Composable
private fun SplitResult(breakdown: MemBreakdown, loading: Boolean, onRefresh: () -> Unit) {
    val scheme = MaterialTheme.colorScheme
    val total = (breakdown.systemPssKb + breakdown.userPssKb).coerceAtLeast(1)

    Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
        Row(
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            Column(modifier = Modifier.weight(1f)) {
                Text(
                    "合计 PSS ${formatKb(breakdown.totalPssKb)}",
                    style = MaterialTheme.typography.titleMediumEmphasized,
                )
                Text(
                    buildString {
                        append("${breakdown.processes} 个进程")
                        if (breakdown.estimated > 0) append(" · ${breakdown.estimated} 个按 RSS 估算")
                        if (breakdown.skipped > 0) append(" · ${breakdown.skipped} 个不可读")
                    },
                    style = MaterialTheme.typography.bodySmall,
                    color = scheme.onSurfaceVariant,
                )
            }
            RefreshAction(loading = loading, description = "重新分析", onClick = onRefresh)
        }
        Row(
            modifier = Modifier.height(IntrinsicSize.Max),
            horizontalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            StatTile(
                label = "系统应用",
                value = formatKbShort(breakdown.systemPssKb),
                supporting = percent(breakdown.systemPssKb, total),
                container = scheme.primaryContainer,
                content = scheme.onPrimaryContainer,
                modifier = Modifier
                    .weight(1f)
                    .fillMaxHeight(),
            )
            StatTile(
                label = "用户应用",
                value = formatKbShort(breakdown.userPssKb),
                supporting = percent(breakdown.userPssKb, total),
                container = scheme.tertiaryContainer,
                content = scheme.onTertiaryContainer,
                modifier = Modifier
                    .weight(1f)
                    .fillMaxHeight(),
            )
        }
        // Same colours as the tiles above, so the bar needs no legend.
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .height(16.dp),
            horizontalArrangement = Arrangement.spacedBy(3.dp),
        ) {
            if (breakdown.systemPssKb > 0) {
                Box(
                    Modifier
                        .weight(breakdown.systemPssKb.toFloat() / total)
                        .fillMaxHeight()
                        .clip(RoundedCornerShape(topStart = 8.dp, bottomStart = 8.dp, topEnd = 3.dp, bottomEnd = 3.dp))
                        .background(scheme.primaryContainer),
                )
            }
            if (breakdown.userPssKb > 0) {
                Box(
                    Modifier
                        .weight(breakdown.userPssKb.toFloat() / total)
                        .fillMaxHeight()
                        .clip(RoundedCornerShape(topStart = 3.dp, bottomStart = 3.dp, topEnd = 8.dp, bottomEnd = 8.dp))
                        .background(scheme.tertiaryContainer),
                )
            }
        }
        NoteTile("此口径是各进程 PSS 之和，与「内存构成」里的「已用」不同，两者不可相加。")
    }
}

// ---- one app -------------------------------------------------------------------------------

/**
 * What one ranked app *is* — the numbers the ranking aggregates, plus the packages behind the UID.
 *
 * Deliberately not a bridge to a process page: that page (PID list, state, stop) belongs to the
 * process module, and a button that leads nowhere would be worse than no button.
 */
@Composable
internal fun AppDetailSheet(entry: MemAppEntry, onDismiss: () -> Unit) {
    val scheme = MaterialTheme.colorScheme
    val identities = rememberAppIdentities(listOfNotNull(entry.packages.firstOrNull()))
    val bottomInset = WindowInsets.navigationBars.asPaddingValues().calculateBottomPadding()
    val sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true)

    val tiles = buildList {
        add(
            DetailTile(
                "PSS",
                formatKb(entry.pssKb),
                scheme.primaryContainer,
                scheme.onPrimaryContainer,
            ),
        )
        add(
            DetailTile(
                "占应用内存",
                String.format(Locale.ROOT, "%.1f%%", entry.share * 100),
                scheme.secondaryContainer,
                scheme.onSecondaryContainer,
            ),
        )
        add(
            DetailTile(
                "进程数",
                "${entry.procs}",
                scheme.tertiaryContainer,
                scheme.onTertiaryContainer,
            ),
        )
        if (entry.swapPssKb > 0) {
            add(
                DetailTile(
                    "已换出",
                    formatKb(entry.swapPssKb),
                    scheme.surfaceContainerHigh,
                    scheme.onSurface,
                ),
            )
        }
    }

    ModalBottomSheet(
        onDismissRequest = onDismiss,
        sheetState = sheetState,
        containerColor = scheme.surfaceContainerLow,
    ) {
        Column(
            modifier = Modifier
                .fillMaxWidth()
                .verticalScroll(rememberScrollState())
                .padding(start = 16.dp, end = 16.dp, bottom = 24.dp + bottomInset),
            verticalArrangement = Arrangement.spacedBy(16.dp),
        ) {
            Row(
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(16.dp),
            ) {
                AppIcon(
                    icon = identities[entry.packages.firstOrNull()]?.icon,
                    size = 56.dp,
                    highlight = true,
                )
                Column(modifier = Modifier.weight(1f)) {
                    Text(
                        entry.name,
                        style = MaterialTheme.typography.titleLargeEmphasized,
                        maxLines = 2,
                        overflow = TextOverflow.Ellipsis,
                    )
                    Text(
                        "UID ${entry.uid} · ${entry.numericUid}",
                        style = MaterialTheme.typography.bodySmall,
                        color = scheme.onSurfaceVariant,
                    )
                }
            }
            tiles.chunked(2).forEach { pair ->
                Row(
                    modifier = Modifier.height(IntrinsicSize.Max),
                    horizontalArrangement = Arrangement.spacedBy(12.dp),
                ) {
                    pair.forEach { tile ->
                        StatTile(
                            label = tile.label,
                            value = tile.value,
                            container = tile.container,
                            content = tile.content,
                            modifier = Modifier
                                .weight(1f)
                                .fillMaxHeight(),
                        )
                    }
                    // An odd tile keeps its half-width instead of stretching across the sheet.
                    if (pair.size == 1) Spacer(Modifier.weight(1f))
                }
            }
            if (entry.packages.isNotEmpty()) {
                Text(
                    "包名",
                    style = MaterialTheme.typography.titleMediumEmphasized,
                    color = scheme.onSurfaceVariant,
                    modifier = Modifier.padding(start = 8.dp),
                )
                Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
                    entry.packages.forEachIndexed { index, packageName ->
                        Text(
                            packageName,
                            modifier = Modifier
                                .fillMaxWidth()
                                .clip(segmentedShape(index, entry.packages.size))
                                .background(scheme.surfaceContainerHigh)
                                .padding(horizontal = 16.dp, vertical = 12.dp),
                            style = MaterialTheme.typography.bodyMedium,
                        )
                    }
                }
            }
        }
    }
}

private data class DetailTile(
    val label: String,
    val value: String,
    val container: Color,
    val content: Color,
)
