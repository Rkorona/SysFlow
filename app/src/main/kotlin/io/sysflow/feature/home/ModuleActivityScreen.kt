@file:OptIn(
    ExperimentalMaterial3Api::class,
    ExperimentalMaterial3ExpressiveApi::class,
)

package io.sysflow.feature.home

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.Build
import androidx.compose.material.icons.rounded.ChevronRight
import androidx.compose.material.icons.rounded.Close
import androidx.compose.material.icons.rounded.Delete
import androidx.compose.material.icons.rounded.Info
import androidx.compose.material.icons.rounded.Timeline
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.ExperimentalMaterial3ExpressiveApi
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.LoadingIndicator
import androidx.compose.material3.MaterialShapes
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.material3.toShape
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.Shape
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import io.sysflow.core.event.SystemEventEntity
import io.sysflow.core.ui.components.InfoPill
import io.sysflow.core.ui.components.SectionLabel
import io.sysflow.core.ui.components.StatusPill
import io.sysflow.core.ui.components.segmentedShape
import kotlinx.coroutines.launch
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale

/**
 * Every module that produced an event, newest activity first.
 *
 * The list is derived from the events themselves — there is no module registry to keep in sync, so a
 * module appears here the moment it emits its first event.
 *
 * Layout (Material 3 Expressive): a tonal overview card, one connected "segmented" list of modules,
 * and a second connected list for maintenance actions. Diagnostics opens in a bottom sheet because a
 * CLI transcript is long and needs to scroll; clearing keeps a confirmation dialog.
 */
@Composable
fun ModuleActivityScreen(
    events: List<SystemEventEntity>,
    onOpenModule: (String) -> Unit,
    onClearEvents: suspend () -> ClearEventsResult = { ClearEventsResult(null, null, 0, null, null, null) },
    /** Read-only CLI transcript, shown in the diagnostics sheet. */
    onDiagnostics: suspend () -> String = { "未实现" },
) {
    val groups = remember(events) { buildModuleActivity(events, 1) }
    val stats = remember(events) { moduleStats(events) }
    var confirmClear by remember { mutableStateOf(false) }
    var showDiagnostics by remember { mutableStateOf(false) }
    var clearing by remember { mutableStateOf(false) }
    var diagnostics by remember { mutableStateOf<String?>(null) }
    var clearedMessage by remember { mutableStateOf<String?>(null) }
    val scope = rememberCoroutineScope()

    // The transcript is fetched when the sheet is opened, not on every recomposition, and it is kept
    // afterwards so closing and reopening the sheet shows the same snapshot.
    LaunchedEffect(showDiagnostics) {
        if (showDiagnostics && diagnostics == null) diagnostics = onDiagnostics()
    }

    LazyColumn(
        modifier = Modifier.fillMaxSize(),
        contentPadding = PaddingValues(start = 20.dp, end = 20.dp, top = 8.dp, bottom = 32.dp),
        verticalArrangement = Arrangement.spacedBy(2.dp),
    ) {
        // The result message must survive the list emptying, so it sits outside the "has groups" guard.
        clearedMessage?.let { message ->
            item(key = "cleared") {
                ResultCard(
                    message = message,
                    onDismiss = { clearedMessage = null },
                    modifier = Modifier.padding(bottom = 12.dp),
                )
            }
        }

        if (groups.isEmpty()) {
            item(key = "empty") { EmptyState(modifier = Modifier.padding(vertical = 56.dp)) }
        } else {
            item(key = "overview") {
                OverviewCard(
                    moduleCount = groups.size,
                    eventCount = events.size,
                    attention = stats.values.sumOf { it.attention },
                    hasError = stats.values.any { it.hasError },
                    latestMs = groups.first().latestMs,
                )
            }
            item(key = "modules-title") {
                SectionLabel(
                    "模块 · ${groups.size}",
                    modifier = Modifier.padding(start = 8.dp, top = 20.dp, bottom = 8.dp),
                )
            }
            itemsIndexed(groups, key = { _, group -> "module-${group.module}" }) { index, group ->
                ModuleRow(
                    group = group,
                    stats = stats[group.module] ?: ModuleStats(0, 0, false),
                    index = index,
                    count = groups.size,
                    onOpen = { onOpenModule(group.module) },
                    modifier = Modifier.animateItem(),
                )
            }
        }

        // Maintenance is deliberately a separate list at the end: the destructive action must be
        // deliberate, never a mis-tap while browsing modules.
        if (groups.isNotEmpty()) {
            item(key = "maintenance-title") {
                SectionLabel(
                    "维护",
                    modifier = Modifier.padding(start = 8.dp, top = 20.dp, bottom = 8.dp),
                )
            }
            item(key = "action-diagnostics") {
                ActionRow(
                    icon = Icons.Rounded.Build,
                    title = "事件诊断",
                    subtitle = "查看模块事件期刊的读取记录",
                    index = 0,
                    count = 2,
                    iconContainer = MaterialTheme.colorScheme.secondaryContainer,
                    iconTint = MaterialTheme.colorScheme.onSecondaryContainer,
                    titleColor = MaterialTheme.colorScheme.onSurface,
                    enabled = !clearing,
                    busy = false,
                    onClick = { showDiagnostics = true },
                )
            }
            item(key = "action-clear") {
                ActionRow(
                    icon = Icons.Rounded.Delete,
                    title = "清除全部事件",
                    subtitle = if (clearing) "正在清除…" else "删除期刊与本地缓存，不可撤销",
                    index = 1,
                    count = 2,
                    iconContainer = MaterialTheme.colorScheme.errorContainer,
                    iconTint = MaterialTheme.colorScheme.onErrorContainer,
                    titleColor = MaterialTheme.colorScheme.error,
                    enabled = !clearing,
                    busy = clearing,
                    onClick = { confirmClear = true },
                )
            }
        }
    }

    if (showDiagnostics) {
        ModalBottomSheet(
            onDismissRequest = { showDiagnostics = false },
            sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true),
        ) {
            Column(
                modifier = Modifier
                    .fillMaxWidth()
                    .padding(start = 24.dp, end = 24.dp, bottom = 24.dp),
                verticalArrangement = Arrangement.spacedBy(12.dp),
            ) {
                Text("事件诊断", style = MaterialTheme.typography.titleLargeEmphasized)
                val transcript = diagnostics
                if (transcript == null) {
                    Box(
                        modifier = Modifier.fillMaxWidth().padding(vertical = 32.dp),
                        contentAlignment = Alignment.Center,
                    ) { LoadingIndicator() }
                } else {
                    Surface(
                        modifier = Modifier.fillMaxWidth(),
                        shape = MaterialTheme.shapes.large,
                        color = MaterialTheme.colorScheme.surfaceContainerHighest,
                    ) {
                        SelectionContainer {
                            Text(
                                transcript,
                                modifier = Modifier
                                    .heightIn(max = 420.dp)
                                    .verticalScroll(rememberScrollState())
                                    .padding(16.dp),
                                style = MaterialTheme.typography.labelSmall,
                                fontFamily = FontFamily.Monospace,
                                color = MaterialTheme.colorScheme.onSurfaceVariant,
                            )
                        }
                    }
                }
            }
        }
    }

    if (confirmClear) {
        AlertDialog(
            onDismissRequest = { confirmClear = false },
            icon = {
                Icon(
                    Icons.Rounded.Delete,
                    contentDescription = null,
                    tint = MaterialTheme.colorScheme.error,
                )
            },
            title = { Text("清除全部事件？") },
            text = {
                Text(
                    "将删除模块事件期刊中的全部记录，并清空本机的本地缓存。此操作不可撤销，" +
                        "之后的系统动态只显示新产生的事件。",
                )
            },
            confirmButton = {
                TextButton(
                    onClick = {
                        confirmClear = false
                        clearing = true
                        scope.launch {
                            clearedMessage = clearMessage(onClearEvents())
                            clearing = false
                        }
                    },
                ) {
                    Text("清除", color = MaterialTheme.colorScheme.error)
                }
            },
            dismissButton = {
                TextButton(onClick = { confirmClear = false }) { Text("取消") }
            },
        )
    }
}

// ───────────────────────────── Overview ─────────────────────────────

@Composable
private fun OverviewCard(
    moduleCount: Int,
    eventCount: Int,
    attention: Int,
    hasError: Boolean,
    latestMs: Long,
    modifier: Modifier = Modifier,
) {
    val scheme = MaterialTheme.colorScheme
    val latest = remember(latestMs) { SimpleDateFormat("HH:mm", Locale.getDefault()).format(Date(latestMs)) }
    val (attentionContainer, attentionContent) = eventSeverityColors(
        if (hasError) "error" else "warning",
        neutralForIconBox = false,
    )

    Surface(
        modifier = modifier.fillMaxWidth(),
        shape = MaterialTheme.shapes.extraLarge,
        color = scheme.primaryContainer,
        contentColor = scheme.onPrimaryContainer,
    ) {
        Column(
            modifier = Modifier.padding(horizontal = 20.dp, vertical = 18.dp),
            verticalArrangement = Arrangement.spacedBy(14.dp),
        ) {
            Row(
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(16.dp),
            ) {
                Box(
                    modifier = Modifier
                        .size(60.dp)
                        .clip(MaterialShapes.Cookie12Sided.toShape())
                        .background(scheme.primary),
                    contentAlignment = Alignment.Center,
                ) {
                    Icon(
                        Icons.Rounded.Timeline,
                        contentDescription = null,
                        tint = scheme.onPrimary,
                        modifier = Modifier.size(28.dp),
                    )
                }
                Column(verticalArrangement = Arrangement.spacedBy(2.dp)) {
                    Text("活跃模块", style = MaterialTheme.typography.labelLarge)
                    Row(
                        verticalAlignment = Alignment.Bottom,
                        horizontalArrangement = Arrangement.spacedBy(4.dp),
                    ) {
                        Text("$moduleCount", style = MaterialTheme.typography.headlineLargeEmphasized)
                        Text(
                            "个",
                            style = MaterialTheme.typography.titleSmall,
                            modifier = Modifier.padding(bottom = 6.dp),
                        )
                    }
                    Text(
                        "近期共 $eventCount 条事件 · 最近 $latest",
                        style = MaterialTheme.typography.bodySmall,
                        color = scheme.onPrimaryContainer.copy(alpha = 0.72f),
                    )
                }
            }
            if (attention > 0) {
                StatusPill(
                    text = "$attention 条需关注",
                    container = attentionContainer,
                    content = attentionContent,
                )
            }
        }
    }
}

// ───────────────────────────── Module list ─────────────────────────────

@Composable
private fun ModuleRow(
    group: ModuleActivity,
    stats: ModuleStats,
    index: Int,
    count: Int,
    onOpen: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val scheme = MaterialTheme.colorScheme
    val (iconContainer, iconTint) = moduleColors(group.tone)
    val (attentionContainer, attentionContent) = eventSeverityColors(
        if (stats.hasError) "error" else "warning",
        neutralForIconBox = false,
    )
    val latest = remember(group.latestMs) {
        SimpleDateFormat("HH:mm", Locale.getDefault()).format(Date(group.latestMs))
    }
    val summary = remember(group) {
        group.entries.firstOrNull()?.let { entry ->
            if (entry.detail.isBlank()) entry.label else "${entry.label} · ${entry.detail}"
        }.orEmpty()
    }

    Surface(
        onClick = onOpen,
        modifier = modifier.fillMaxWidth(),
        shape = segmentedShape(index, count),
        color = scheme.surfaceContainerLow,
    ) {
        Row(
            modifier = Modifier.padding(16.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(14.dp),
        ) {
            Box(
                modifier = Modifier
                    .size(52.dp)
                    .clip(moduleShape(group.tone))
                    .background(iconContainer),
                contentAlignment = Alignment.Center,
            ) {
                Icon(
                    moduleIcon(group.module),
                    contentDescription = null,
                    tint = iconTint,
                    modifier = Modifier.size(26.dp),
                )
            }
            Column(modifier = Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(4.dp)) {
                Text(
                    group.label,
                    style = MaterialTheme.typography.titleMediumEmphasized,
                    color = scheme.onSurface,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
                if (summary.isNotBlank()) {
                    Text(
                        summary,
                        style = MaterialTheme.typography.bodySmall,
                        color = scheme.onSurfaceVariant,
                        maxLines = 2,
                        overflow = TextOverflow.Ellipsis,
                    )
                }
                Row(
                    modifier = Modifier.padding(top = 4.dp),
                    horizontalArrangement = Arrangement.spacedBy(6.dp),
                ) {
                    InfoPill(
                        text = "近期 ${stats.total} 条",
                        container = scheme.surfaceContainerHighest,
                        content = scheme.onSurfaceVariant,
                    )
                    if (stats.attention > 0) {
                        StatusPill(
                            text = "${stats.attention} 条需关注",
                            container = attentionContainer,
                            content = attentionContent,
                        )
                    }
                }
            }
            Column(horizontalAlignment = Alignment.End, verticalArrangement = Arrangement.spacedBy(8.dp)) {
                Text(
                    latest,
                    style = MaterialTheme.typography.labelSmall,
                    color = scheme.outline,
                )
                Icon(
                    Icons.Rounded.ChevronRight,
                    contentDescription = null,
                    tint = scheme.outline,
                    modifier = Modifier.size(20.dp),
                )
            }
        }
    }
}

@Composable
internal fun moduleShape(tone: ModuleTone): Shape = when (tone) {
    ModuleTone.CPU -> MaterialShapes.Cookie9Sided.toShape()
    ModuleTone.MEMORY -> MaterialShapes.Clover4Leaf.toShape()
    ModuleTone.POWER -> MaterialShapes.Sunny.toShape()
    ModuleTone.NEUTRAL -> MaterialShapes.Cookie12Sided.toShape()
}

/** Per-module numbers over the fetched window, counted from raw events (not collapsed rows). */
internal data class ModuleStats(val total: Int, val attention: Int, val hasError: Boolean)

private val ATTENTION_SEVERITIES = setOf("warning", "error", "critical")
private val ERROR_SEVERITIES = setOf("error", "critical")

internal fun moduleStats(events: List<SystemEventEntity>): Map<String, ModuleStats> =
    events.groupBy { it.module }.mapValues { (_, list) ->
        ModuleStats(
            total = list.size,
            attention = list.count { it.severity in ATTENTION_SEVERITIES },
            hasError = list.any { it.severity in ERROR_SEVERITIES },
        )
    }

// ───────────────────────────── Maintenance ─────────────────────────────

@Composable
private fun ActionRow(
    icon: ImageVector,
    title: String,
    subtitle: String,
    index: Int,
    count: Int,
    iconContainer: Color,
    iconTint: Color,
    titleColor: Color,
    enabled: Boolean,
    busy: Boolean,
    onClick: () -> Unit,
) {
    val scheme = MaterialTheme.colorScheme
    Surface(
        onClick = onClick,
        enabled = enabled,
        modifier = Modifier.fillMaxWidth(),
        shape = segmentedShape(index, count),
        color = scheme.surfaceContainerLow,
    ) {
        Row(
            modifier = Modifier.padding(16.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(14.dp),
        ) {
            Box(
                modifier = Modifier
                    .size(44.dp)
                    .clip(MaterialShapes.Pill.toShape())
                    .background(iconContainer),
                contentAlignment = Alignment.Center,
            ) {
                if (busy) {
                    LoadingIndicator(modifier = Modifier.size(28.dp), color = iconTint)
                } else {
                    Icon(icon, contentDescription = null, tint = iconTint, modifier = Modifier.size(22.dp))
                }
            }
            Column(modifier = Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(2.dp)) {
                Text(
                    title,
                    style = MaterialTheme.typography.titleMediumEmphasized,
                    color = titleColor,
                )
                Text(
                    subtitle,
                    style = MaterialTheme.typography.bodySmall,
                    color = scheme.onSurfaceVariant,
                )
            }
        }
    }
}

@Composable
private fun ResultCard(message: String, onDismiss: () -> Unit, modifier: Modifier = Modifier) {
    val scheme = MaterialTheme.colorScheme
    Surface(
        modifier = modifier.fillMaxWidth(),
        shape = MaterialTheme.shapes.large,
        color = scheme.secondaryContainer,
        contentColor = scheme.onSecondaryContainer,
    ) {
        Row(
            modifier = Modifier.padding(start = 16.dp, top = 8.dp, bottom = 8.dp, end = 4.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            Icon(Icons.Rounded.Info, contentDescription = null, modifier = Modifier.size(22.dp))
            Text(
                message,
                modifier = Modifier.weight(1f).padding(vertical = 8.dp),
                style = MaterialTheme.typography.bodySmall,
            )
            IconButton(onClick = onDismiss) {
                Icon(Icons.Rounded.Close, contentDescription = "关闭")
            }
        }
    }
}

// ───────────────────────────── Empty ─────────────────────────────

@Composable
private fun EmptyState(modifier: Modifier = Modifier) {
    Column(
        modifier = modifier.fillMaxWidth(),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        Box(
            modifier = Modifier
                .size(96.dp)
                .clip(MaterialShapes.Cookie12Sided.toShape())
                .background(MaterialTheme.colorScheme.surfaceContainerHighest),
            contentAlignment = Alignment.Center,
        ) {
            Icon(
                Icons.Rounded.Timeline,
                contentDescription = null,
                tint = MaterialTheme.colorScheme.onSurfaceVariant,
                modifier = Modifier.size(40.dp),
            )
        }
        Text(
            "暂无模块动态",
            style = MaterialTheme.typography.titleMediumEmphasized,
            modifier = Modifier.padding(top = 8.dp),
        )
        Text(
            "Root 模块产生事件后，对应模块会自动出现在这里",
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.outline,
            textAlign = TextAlign.Center,
        )
    }
}

/**
 * Human-readable outcome of a journal clear.
 *
 * Every number is stated, and a non-zero remainder is *explained* rather than left as a scare: the
 * background daemon keeps writing (battery sampling alone is once a minute), so records legitimately
 * reappear within seconds of a clear. Without that sentence the result reads as "the delete failed",
 * which is exactly the wrong conclusion.
 */
private fun clearMessage(result: ClearEventsResult): String {
    result.error?.let { return "清除失败：$it" }
    val deleted = result.cleared ?: 0
    val remaining = result.journalRemaining
    val journal = when {
        remaining == null -> "期刊删除 $deleted 条，剩余数量读取失败"
        remaining == 0 && deleted == 0 -> "期刊已经是空的"
        remaining == 0 -> "期刊删除 $deleted 条，已清空"
        else -> "期刊删除 $deleted 条，仍有 $remaining 条（后台守护进程在清除后继续记录）"
    }
    val mirror = if (result.probeError != null) {
        "本地缓存删除 ${result.localDeleted} 条，读取失败：${result.probeError}"
    } else {
        "本地缓存删除 ${result.localDeleted} 条，剩余 ${result.localRemaining ?: 0} 条"
    }
    val directory = result.directory?.let { "\n期刊目录：$it" }.orEmpty()
    return "$journal · $mirror$directory"
}
