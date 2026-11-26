@file:OptIn(ExperimentalMaterial3ExpressiveApi::class, ExperimentalFoundationApi::class)

package io.sysflow.feature.process

import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.background
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.Apps
import androidx.compose.material.icons.rounded.Check
import androidx.compose.material.icons.rounded.Close
import androidx.compose.material.icons.rounded.Lock
import androidx.compose.material.icons.rounded.Memory
import androidx.compose.material.icons.rounded.Person
import androidx.compose.material.icons.rounded.Refresh
import androidx.compose.material.icons.rounded.Search
import androidx.compose.material.icons.rounded.Tune
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.Checkbox
import androidx.compose.material3.ExperimentalMaterial3ExpressiveApi
import androidx.compose.material3.FilledTonalButton
import androidx.compose.material3.FilterChip
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.LoadingIndicator
import androidx.compose.material3.MaterialShapes
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TextField
import androidx.compose.material3.TextFieldDefaults
import androidx.compose.material3.toShape
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.Shape
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import io.sysflow.core.ui.components.AppIcon
import io.sysflow.core.ui.components.InfoPill
import io.sysflow.core.ui.components.rememberAppIdentities
import io.sysflow.core.ui.components.segmentedShape
import io.sysflow.core.ui.theme.sysFlowColors

/**
 * The process list, read top to bottom as a question chain: how big is the picture → who is in
 * front → how do I narrow it down → who is heavy.
 *
 * The rows are one connected list (soft inner corners, rounded outer ones) instead of a pile of
 * separate cards, and each carries a thin usage bar drawn relative to the busiest row, so the
 * heavy processes read at a glance without opening anything.
 */
@Composable
internal fun ProcessListContent(
    state: ProcessViewModel.UiState,
    list: ProcessList,
    bottomInset: Dp,
    onRefresh: () -> Unit,
    onQuery: (String) -> Unit,
    onSubmitQuery: () -> Unit,
    onFilter: (String) -> Unit,
    onSort: (String) -> Unit,
    onToggleExperimental: () -> Unit,
    onOpenTuning: () -> Unit,
    onLoadMore: () -> Unit,
    onToggleSelectionMode: () -> Unit,
    onEnterSelection: (Int) -> Unit,
    onToggleSelected: (Int) -> Unit,
    onOpen: (Int) -> Unit,
) {
    val foregroundPackage = list.foreground.packageName
    val packages = remember(list.entries, foregroundPackage) {
        (list.entries.mapNotNull { it.packageName } + listOfNotNull(foregroundPackage)).distinct()
    }
    val identities = rememberAppIdentities(packages)
    val maxCpu = remember(list.entries) { list.entries.maxOfOrNull { it.cpuPct ?: 0.0 } ?: 0.0 }
    val maxRss = remember(list.entries) { list.entries.maxOfOrNull { it.rssKb ?: 0L } ?: 0L }

    LazyColumn(
        modifier = Modifier.fillMaxSize(),
        contentPadding = PaddingValues(
            start = 16.dp,
            end = 16.dp,
            top = 8.dp,
            bottom = (when {
                state.selectionMode -> 104.dp
                "stop_all" in state.features -> 88.dp
                else -> 24.dp
            }) + bottomInset,
        ),
        verticalArrangement = Arrangement.spacedBy(4.dp),
    ) {
        item(key = "hero") {
            ProcessHero(
                list = list,
                foregroundLabel = foregroundPackage?.let { identities[it]?.label ?: it },
                foregroundIcon = foregroundPackage?.let { identities[it]?.icon },
                pollMs = state.pollMs,
                sampleMs = state.sampleMs,
                modifier = Modifier.padding(bottom = 8.dp),
            )
        }

        item(key = "controls") {
            Column(
                modifier = Modifier.padding(bottom = 8.dp),
                verticalArrangement = Arrangement.spacedBy(12.dp),
            ) {
                ProcessSearchField(
                    query = state.query,
                    refreshing = state.refreshing,
                    onQuery = onQuery,
                    onSubmit = onSubmitQuery,
                    onRefresh = onRefresh,
                )
                ConnectedChoiceRow(
                    options = listOf("all", "user", "system", "protected"),
                    selected = state.filter,
                    label = { filterLabel(it) },
                    onSelect = onFilter,
                    icon = { filterIcon(it) },
                )
                ConnectedChoiceRow(
                    options = listOf("cpu", "rss", "name", "pid"),
                    selected = state.sort,
                    label = { sortLabel(it) },
                    onSelect = onSort,
                    compact = true,
                )
            }
        }

        if (state.hasExperimental) {
            item(key = "experimental") {
                ExperimentalTile(
                    enabled = state.experimental,
                    features = state.features,
                    onToggle = onToggleExperimental,
                    modifier = Modifier.padding(bottom = 8.dp),
                )
            }
        }

        item(key = "header") {
            SectionHeader(
                title = "进程",
                supporting = if (state.selectionMode) {
                    "点按勾选要批量处理的进程（受保护进程不可选）"
                } else {
                    "按${sortLabel(state.sort)}排序 · 点按查看详情，长按多选"
                },
                action = {
                    Row(
                        horizontalArrangement = Arrangement.spacedBy(4.dp),
                        verticalAlignment = Alignment.CenterVertically,
                    ) {
                        IconButton(onClick = onOpenTuning) {
                            Icon(
                                Icons.Rounded.Tune,
                                contentDescription = "刷新设置",
                                modifier = Modifier.size(20.dp),
                            )
                        }
                        FilterChip(
                            selected = state.selectionMode,
                            onClick = onToggleSelectionMode,
                            label = { Text(if (state.selectionMode) "完成" else "多选") },
                            leadingIcon = if (state.selectionMode) {
                                {
                                    Icon(
                                        Icons.Rounded.Check,
                                        contentDescription = null,
                                        modifier = Modifier.size(16.dp),
                                    )
                                }
                            } else {
                                null
                            },
                        )
                    }
                },
                modifier = Modifier.padding(bottom = 8.dp),
            )
        }

        if (list.entries.isEmpty()) {
            item(key = "empty") {
                EmptyActionTile(
                    icon = Icons.Rounded.Search,
                    text = "没有匹配的进程。",
                    actionLabel = "刷新",
                    onAction = onRefresh,
                )
            }
        }

        items(list.entries.size, key = { index ->
            val entry = list.entries[index]
            "${entry.pid}-${entry.name}"
        }) { index ->
            val entry = list.entries[index]
            ProcessRow(
                entry = entry,
                icon = entry.packageName?.let { identities[it]?.icon },
                shape = segmentedShape(index, list.entries.size),
                usage = usageFraction(entry, state.sort, maxCpu, maxRss),
                selectionMode = state.selectionMode,
                selected = entry.pid in state.selectedPids,
                onLongClick = { onEnterSelection(entry.pid) },
                onToggleSelect = { onToggleSelected(entry.pid) },
                onClick = { onOpen(entry.pid) },
                modifier = Modifier.animateItem(),
            )
        }

        item(key = "footer") {
            Column(
                modifier = Modifier.padding(top = 8.dp),
                verticalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                if (list.truncated && state.limit < ProcessViewModel.MAX_LIMIT) {
                    FilledTonalButton(
                        onClick = onLoadMore,
                        shapes = ButtonDefaults.shapes(),
                        modifier = Modifier.fillMaxWidth(),
                    ) {
                        Text("加载更多（当前 ${list.returned} / ${list.total}）")
                    }
                }
                NoteTile(
                    text = buildString {
                        append("共 ${list.total} 个进程")
                        if (list.truncated) append("，当前显示前 ${list.returned} 个")
                        if (list.skipped > 0) append("，${list.skipped} 个进程读取失败")
                        append(" · 采样 ${list.sampleMs} ms")
                    },
                )
            }
        }
    }
}

// ---- hero ------------------------------------------------------------------------------------

/**
 * Size of the picture: how many processes, what the listed ones add up to, and who is in front.
 *
 * The sums cover the rows the module returned (at most [ProcessViewModel.MAX_LIMIT]), and say so —
 * CPU is per core, so it can pass 100%, and RSS counts shared pages once per process.
 */
@Composable
private fun ProcessHero(
    list: ProcessList,
    foregroundLabel: String?,
    foregroundIcon: ImageBitmap?,
    /** Configured cadence (ProcessTuning); shown as-is so the card matches the tune dialog. */
    pollMs: Int,
    sampleMs: Int,
    modifier: Modifier = Modifier,
) {
    val scheme = MaterialTheme.colorScheme
    val accent = MaterialTheme.sysFlowColors.containerAccent
    val cpuSum = remember(list.entries) { list.entries.sumOf { it.cpuPct ?: 0.0 } }
    val rssSum = remember(list.entries) { list.entries.sumOf { it.rssKb ?: 0L } }

    Surface(
        modifier = modifier.fillMaxWidth(),
        shape = MaterialTheme.shapes.extraExtraLarge,
        color = scheme.primaryContainer,
        contentColor = scheme.onPrimaryContainer,
    ) {
        Column(
            modifier = Modifier.padding(20.dp),
            verticalArrangement = Arrangement.spacedBy(16.dp),
        ) {
            Row(
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(16.dp),
            ) {
                Box(
                    modifier = Modifier
                        .size(96.dp)
                        .clip(MaterialShapes.Cookie12Sided.toShape())
                        .background(accent),
                    contentAlignment = Alignment.Center,
                ) {
                    Text(
                        "${list.total}",
                        style = MaterialTheme.typography.headlineMediumEmphasized,
                        color = scheme.primaryContainer,
                        maxLines = 1,
                    )
                }
                Column(
                    modifier = Modifier.weight(1f),
                    verticalArrangement = Arrangement.spacedBy(4.dp),
                ) {
                    Text(
                        "运行中的进程",
                        style = MaterialTheme.typography.labelLarge,
                        color = scheme.onPrimaryContainer.copy(alpha = 0.75f),
                    )
                    Text(
                        if (list.truncated) "列出前 ${list.returned} 个" else "全部已列出",
                        style = MaterialTheme.typography.headlineSmallEmphasized,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                    )
                    Text(
                        if (pollMs % 1000 == 0) {
                            "每 ${pollMs / 1000} 秒刷新 · 采样 $sampleMs ms"
                        } else {
                            "每 $pollMs ms 刷新 · 采样 $sampleMs ms"
                        },
                        style = MaterialTheme.typography.bodySmall,
                        color = scheme.onPrimaryContainer.copy(alpha = 0.75f),
                    )
                }
            }

            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                HeroStat(
                    label = "列表 CPU",
                    value = formatProcessCpu(cpuSum),
                    supporting = "单核计，可超 100%",
                    modifier = Modifier.weight(1f),
                )
                HeroStat(
                    label = "列表 RSS",
                    value = formatProcessKb(rssSum),
                    supporting = "共享页会重复计",
                    modifier = Modifier.weight(1f),
                )
            }

            if (foregroundLabel != null) {
                Row(
                    modifier = Modifier
                        .fillMaxWidth()
                        .clip(CircleShape)
                        .background(scheme.onPrimaryContainer.copy(alpha = 0.12f))
                        .padding(start = 8.dp, end = 16.dp, top = 8.dp, bottom = 8.dp),
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(12.dp),
                ) {
                    AppIcon(icon = foregroundIcon, size = 36.dp, highlight = true)
                    Column(Modifier.weight(1f)) {
                        Text(
                            "前台应用",
                            style = MaterialTheme.typography.labelSmall,
                            color = scheme.onPrimaryContainer.copy(alpha = 0.75f),
                        )
                        Text(
                            foregroundLabel,
                            style = MaterialTheme.typography.titleSmallEmphasized,
                            maxLines = 1,
                            overflow = TextOverflow.Ellipsis,
                        )
                    }
                    list.foreground.pid?.let { pid ->
                        InfoPill(
                            text = "PID $pid",
                            container = scheme.onPrimaryContainer,
                            content = scheme.primaryContainer,
                        )
                    }
                }
            }
        }
    }
}

@Composable
private fun HeroStat(
    label: String,
    value: String,
    supporting: String,
    modifier: Modifier = Modifier,
) {
    val content = MaterialTheme.colorScheme.onPrimaryContainer
    Column(
        modifier = modifier
            .clip(MaterialTheme.shapes.large)
            .background(content.copy(alpha = 0.12f))
            .padding(horizontal = 14.dp, vertical = 12.dp),
        verticalArrangement = Arrangement.spacedBy(2.dp),
    ) {
        Text(
            label,
            style = MaterialTheme.typography.labelMedium,
            color = content.copy(alpha = 0.75f),
            maxLines = 1,
        )
        Text(
            value,
            style = MaterialTheme.typography.titleLargeEmphasized,
            maxLines = 1,
            overflow = TextOverflow.Ellipsis,
        )
        Text(
            supporting,
            style = MaterialTheme.typography.labelSmall,
            color = content.copy(alpha = 0.75f),
            maxLines = 1,
            overflow = TextOverflow.Ellipsis,
        )
    }
}

// ---- search / experimental -------------------------------------------------------------------

@Composable
private fun ProcessSearchField(
    query: String,
    refreshing: Boolean,
    onQuery: (String) -> Unit,
    onSubmit: () -> Unit,
    onRefresh: () -> Unit,
) {
    val scheme = MaterialTheme.colorScheme
    TextField(
        value = query,
        onValueChange = onQuery,
        modifier = Modifier.fillMaxWidth(),
        singleLine = true,
        shape = CircleShape,
        colors = TextFieldDefaults.colors(
            focusedContainerColor = scheme.surfaceContainerHigh,
            unfocusedContainerColor = scheme.surfaceContainerHigh,
            disabledContainerColor = scheme.surfaceContainerHigh,
            focusedIndicatorColor = Color.Transparent,
            unfocusedIndicatorColor = Color.Transparent,
            disabledIndicatorColor = Color.Transparent,
            errorIndicatorColor = Color.Transparent,
        ),
        leadingIcon = { Icon(Icons.Rounded.Search, contentDescription = null) },
        trailingIcon = {
            Row(verticalAlignment = Alignment.CenterVertically) {
                if (query.isNotEmpty()) {
                    IconButton(onClick = { onQuery("") }) {
                        Icon(Icons.Rounded.Close, contentDescription = "清除")
                    }
                }
                if (refreshing) {
                    Box(Modifier.size(48.dp), contentAlignment = Alignment.Center) {
                        LoadingIndicator(modifier = Modifier.size(24.dp))
                    }
                } else {
                    IconButton(onClick = onRefresh) {
                        Icon(Icons.Rounded.Refresh, contentDescription = "刷新")
                    }
                }
            }
        },
        placeholder = { Text("搜索进程、包名或 PID") },
        keyboardOptions = KeyboardOptions(imeAction = ImeAction.Search),
        keyboardActions = KeyboardActions(onSearch = { onSubmit() }),
    )
}

/** Opt-in switch for the one-shot actions that can hurt: OOM adjust, RT sched, cpuset, freeze. */
@Composable
private fun ExperimentalTile(
    enabled: Boolean,
    features: Set<String>,
    onToggle: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val scheme = MaterialTheme.colorScheme
    val container = if (enabled) scheme.tertiaryContainer else scheme.surfaceContainerHigh
    val content = if (enabled) scheme.onTertiaryContainer else scheme.onSurface
    val available = features.any {
        it == "oom_score_adj" || it == "sched" || it == "cpuset" || it == "freeze"
    }

    Surface(
        onClick = onToggle,
        modifier = modifier.fillMaxWidth(),
        shape = MaterialTheme.shapes.extraLarge,
        color = container,
        contentColor = content,
    ) {
        Row(
            modifier = Modifier.padding(horizontal = 16.dp, vertical = 14.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(14.dp),
        ) {
            Box(
                modifier = Modifier
                    .size(44.dp)
                    .clip(MaterialShapes.Cookie9Sided.toShape())
                    .background(if (enabled) scheme.tertiary else scheme.secondaryContainer),
                contentAlignment = Alignment.Center,
            ) {
                Icon(
                    Icons.Rounded.Tune,
                    contentDescription = null,
                    tint = if (enabled) scheme.onTertiary else scheme.onSecondaryContainer,
                    modifier = Modifier.size(22.dp),
                )
            }
            Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(2.dp)) {
                Text("实验功能", style = MaterialTheme.typography.titleMediumEmphasized)
                Text(
                    buildString {
                        append("OOM 调节、实时调度、cpuset 绑核、冻结")
                        if (!available) append("（本机不可用）")
                    },
                    style = MaterialTheme.typography.bodySmall,
                    color = content.copy(alpha = 0.75f),
                )
            }
            Switch(checked = enabled, onCheckedChange = { onToggle() })
        }
    }
}

// ---- rows ------------------------------------------------------------------------------------

@Composable
private fun ProcessRow(
    entry: ProcessEntry,
    icon: ImageBitmap?,
    shape: Shape,
    usage: Float,
    selectionMode: Boolean,
    selected: Boolean,
    onLongClick: () -> Unit,
    onToggleSelect: () -> Unit,
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val scheme = MaterialTheme.colorScheme
    val accent = MaterialTheme.sysFlowColors.containerAccent
    val container = when {
        selected -> scheme.secondaryContainer
        entry.foreground -> scheme.primaryContainer
        else -> scheme.surfaceContainerHigh
    }
    val content = when {
        selected -> scheme.onSecondaryContainer
        entry.foreground -> scheme.onPrimaryContainer
        else -> scheme.onSurface
    }
    val secondary = content.copy(alpha = 0.75f)

    Surface(
        modifier = modifier
            .fillMaxWidth()
            .clip(shape)
            .combinedClickable(
                onClick = {
                    if (selectionMode) {
                        if (!entry.protected) onToggleSelect()
                    } else {
                        onClick()
                    }
                },
                onLongClick = {
                    if (!selectionMode && !entry.protected) onLongClick()
                },
            ),
        shape = shape,
        color = container,
        contentColor = content,
    ) {
        Row(
            modifier = Modifier.padding(horizontal = 16.dp, vertical = 14.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(14.dp),
        ) {
            if (selectionMode) {
                Checkbox(
                    checked = selected,
                    onCheckedChange = null,
                    enabled = !entry.protected,
                )
            }
            AppIcon(icon = icon, size = 48.dp, highlight = entry.foreground)
            Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(2.dp)) {
                Row(
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(8.dp),
                ) {
                    Text(
                        entry.displayName,
                        modifier = Modifier.weight(1f, fill = false),
                        style = MaterialTheme.typography.titleMediumEmphasized,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                    )
                    if (entry.foreground) {
                        InfoPill(
                            text = "前台",
                            container = scheme.onPrimaryContainer,
                            content = scheme.primaryContainer,
                        )
                    }
                    if (entry.frozen == true) {
                        InfoPill(
                            text = "已冻结",
                            container = scheme.tertiaryContainer,
                            content = scheme.onTertiaryContainer,
                        )
                    }
                    if (entry.protected) {
                        Icon(
                            Icons.Rounded.Lock,
                            contentDescription = entry.protectedReason ?: "受保护",
                            modifier = Modifier.size(16.dp),
                            tint = secondary,
                        )
                    }
                }
                Text(
                    processSubtitle(entry),
                    style = MaterialTheme.typography.bodySmall,
                    color = secondary,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
                UsageBar(
                    fraction = usage,
                    color = if (entry.foreground) accent else scheme.primary,
                    track = if (entry.foreground) {
                        scheme.onPrimaryContainer.copy(alpha = 0.16f)
                    } else {
                        scheme.surfaceContainerHighest
                    },
                    modifier = Modifier.padding(top = 6.dp),
                )
            }
            Column(
                horizontalAlignment = Alignment.End,
                verticalArrangement = Arrangement.spacedBy(2.dp),
            ) {
                Text(
                    formatProcessCpu(entry.cpuPct),
                    style = MaterialTheme.typography.titleMediumEmphasized,
                    maxLines = 1,
                )
                Text(
                    formatProcessKb(entry.rssKb),
                    style = MaterialTheme.typography.bodySmall,
                    color = secondary,
                    maxLines = 1,
                )
            }
        }
    }
}

// ---- helpers ---------------------------------------------------------------------------------

/** The bar follows the sort key: memory for 内存, CPU for everything else. */
private fun usageFraction(entry: ProcessEntry, sort: String, maxCpu: Double, maxRss: Long): Float =
    if (sort == "rss") {
        if (maxRss <= 0L) 0f else ((entry.rssKb ?: 0L).toDouble() / maxRss).toFloat()
    } else {
        if (maxCpu <= 0.0) 0f else ((entry.cpuPct ?: 0.0) / maxCpu).toFloat()
    }

private fun filterLabel(value: String) = when (value) {
    "user" -> "用户"
    "system" -> "系统"
    "protected" -> "受保护"
    else -> "全部"
}

private fun filterIcon(value: String) = when (value) {
    "user" -> Icons.Rounded.Person
    "system" -> Icons.Rounded.Memory
    "protected" -> Icons.Rounded.Lock
    else -> Icons.Rounded.Apps
}

private fun sortLabel(value: String) = when (value) {
    "rss" -> "内存"
    "name" -> "名称"
    "pid" -> "PID"
    else -> "CPU"
}
