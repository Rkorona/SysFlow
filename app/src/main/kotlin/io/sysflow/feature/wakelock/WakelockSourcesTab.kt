@file:OptIn(ExperimentalMaterial3ExpressiveApi::class)

package io.sysflow.feature.wakelock

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.LazyListScope
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.AccessTime
import androidx.compose.material.icons.rounded.Alarm
import androidx.compose.material.icons.rounded.BatteryChargingFull
import androidx.compose.material.icons.rounded.Bluetooth
import androidx.compose.material.icons.rounded.Bolt
import androidx.compose.material.icons.rounded.Refresh
import androidx.compose.material.icons.rounded.CameraAlt
import androidx.compose.material.icons.rounded.Cable
import androidx.compose.material.icons.rounded.Category
import androidx.compose.material.icons.rounded.Extension
import androidx.compose.material.icons.rounded.GraphicEq
import androidx.compose.material.icons.rounded.Hub
import androidx.compose.material.icons.rounded.LightMode
import androidx.compose.material.icons.rounded.Nfc
import androidx.compose.material.icons.rounded.Sensors
import androidx.compose.material.icons.rounded.Settings
import androidx.compose.material.icons.rounded.Storage
import androidx.compose.material.icons.rounded.TouchApp
import androidx.compose.material.icons.rounded.Usb
import androidx.compose.material.icons.rounded.Wifi
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.ExperimentalMaterial3ExpressiveApi
import androidx.compose.material3.FilledTonalButton
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.LoadingIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import io.sysflow.core.model.WakelockSummary
import io.sysflow.core.model.WakelockWakeup
import io.sysflow.core.model.formatAgeSeconds
import io.sysflow.core.model.formatAwakeMs
import io.sysflow.core.model.rankingGroups
import io.sysflow.core.model.sourceGroups
import io.sysflow.core.model.wakeRankHint
import io.sysflow.core.ui.components.ExpressiveShapes
import io.sysflow.core.ui.components.IconBadge
import io.sysflow.core.ui.components.InfoPill
import io.sysflow.core.ui.components.StatusPill
import io.sysflow.core.ui.components.StatusTone
import io.sysflow.core.ui.components.WavyMeter
import io.sysflow.core.ui.components.cardPalette
import io.sysflow.core.ui.components.segmentedShape
import kotlin.math.roundToInt

/** One wakeup source as the list shows it, whether it came from the 24 h ranking or the totals. */
private data class SourceLine(val name: String, val metric: Long, val active: Boolean)

/** A driver family with its rows, already ordered by time. */
private data class SourceBlock(
    val category: String,
    val label: String,
    /** Sum of [SourceLine.metric] across the block, in whatever unit the ranking is showing. */
    val totalMetric: Long,
    val lines: List<SourceLine>,
)

/** 唤醒源: hero summary, then the rolling 24 h kernel ranking grouped by driver family. */
internal fun LazyListScope.sourcesTab(
    state: WakelockViewModel.UiState,
    byCount: Boolean = false,
    onToggleMetric: () -> Unit = {},
    onRefreshWakeup: () -> Unit = {},
) {
    val data = state.state

    item(key = "sources-hero") {
        SourcesHero(state = state, modifier = Modifier.animateItem())
    }

    item(key = "sources-wakeup") {
        LastWakeupCard(
            wakeup = state.wakeup,
            loading = state.wakeupLoading,
            onRefresh = onRefreshWakeup,
            modifier = Modifier
                .animateItem()
                .padding(top = 8.dp),
        )
    }

    item(key = "sources-trend") {
        WakeupTrendCard(
            history = state.history,
            loading = state.historyLoading,
            byCount = byCount,
            modifier = Modifier
                .animateItem()
                .padding(top = 12.dp),
        )
    }

    // Only offered when the kernel table actually reports counts: the legacy /proc/wakelocks
    // layout has no wakeup counter, and a toggle that always shows zero would be a lie.
    if (data?.ranked.orEmpty().any { it.wakeups > 0 }) {
        item(key = "sources-metric") {
            FilledTonalButton(
                onClick = onToggleMetric,
                shapes = ButtonDefaults.shapes(),
                modifier = Modifier
                    .animateItem()
                    .padding(top = 4.dp),
            ) {
                Text(if (byCount) "当前按唤醒次数排序 · 切换为时长" else "当前按唤醒时长排序 · 切换为次数")
            }
        }
    }

    item(key = "sources-ranking-head") {
        SectionHeader(
            title = "24 小时唤醒排行",
            caption = "按驱动类别归组（不是应用）：内核只记录硬件活动，无法归因到具体 App",
            modifier = Modifier.animateItem(),
        )
    }

    if (data?.categorySupported == false) {
        item(key = "sources-legacy-module") {
            NoticeTile(
                text = "当前模块版本较旧，返回数据里没有驱动类别，只能全部显示为「其它驱动」。" +
                    "更新模块（重刷 dist 里的 zip）后即可按网络/蓝牙/定时器等类别归组，APK 已就绪。",
                kind = NoticeKind.Warning,
                modifier = Modifier
                    .animateItem()
                    .padding(bottom = 8.dp),
            )
        }
    }

    val ranked = data?.ranked.orEmpty()
    if (ranked.isEmpty()) {
        item(key = "sources-empty") {
            WakelockEmptyState(
                icon = Icons.Rounded.AccessTime,
                title = if (data == null) "正在读取…" else "排行暂时为空",
                body = data?.summary?.let { wakeRankHint(it) } ?: "正在读取内核唤醒源",
                badgeShape = ExpressiveShapes.Cookie9,
                loading = data == null && state.loading,
                modifier = Modifier.animateItem(),
            )
        }

        // Fallback: the window has no *new* time yet, but cumulative counters are already there.
        val cumulative = data?.sources.orEmpty().filter { it.totalMs > 0 }
        if (cumulative.isNotEmpty()) {
            item(key = "sources-cumulative-head") {
                SectionHeader(
                    title = "自开机累计",
                    caption = "窗口内暂无增量，以下为开机以来累计唤醒时间",
                    modifier = Modifier.animateItem(),
                )
            }
            val blocks = cumulative.sourceGroups().map { group ->
                SourceBlock(
                    category = group.category,
                    label = group.label,
                    totalMetric = group.totalMs,
                    lines = group.rows.map { SourceLine(it.name, it.totalMs, it.active) },
                )
            }
            sourceBlocks(prefix = "cum", blocks = blocks)
        }
        return
    }

    val blocks = ranked.rankingGroups(byCount).map { group ->
        SourceBlock(
            category = group.category,
            label = group.label,
            totalMetric = group.totalMs,
            lines = group.rows.map {
                SourceLine(
                    name = it.name,
                    metric = if (byCount) it.wakeups else it.awakeMs,
                    active = false,
                )
            },
        )
    }
    sourceBlocks(prefix = "rank", blocks = blocks, byCount = byCount)
}

/** Category header followed by its connected list of sources; bars show the share of all sources. */
private fun LazyListScope.sourceBlocks(
    prefix: String,
    blocks: List<SourceBlock>,
    byCount: Boolean = false,
) {
    val grandTotal = blocks.sumOf { it.totalMetric }.coerceAtLeast(1L)
    blocks.forEachIndexed { blockIndex, block ->
        item(key = "$prefix-head-${block.category}") {
            CategoryHeader(
                block = block,
                paletteIndex = blockIndex,
                byCount = byCount,
                modifier = Modifier.animateItem(),
            )
        }
        itemsIndexed(
            items = block.lines,
            key = { _, line -> "$prefix-${block.category}-${line.name}" },
        ) { index, line ->
            SourceRow(
                line = line,
                fraction = line.metric.toFloat() / grandTotal,
                byCount = byCount,
                paletteIndex = blockIndex,
                shape = segmentedShape(index, block.lines.size),
                modifier = Modifier.animateItem(),
            )
        }
    }
}

/** What woke the device last; refreshed on demand because it costs a `dumpsys power`. */
@Composable
private fun LastWakeupCard(
    wakeup: WakelockWakeup?,
    loading: Boolean,
    onRefresh: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val scheme = MaterialTheme.colorScheme
    ConnectedItem(
        shape = ExpressiveShapes.Cookie9,
        modifier = modifier,
        container = scheme.surfaceContainerHigh,
    ) {
        IconBadge(
            icon = Icons.Rounded.Bolt,
            container = scheme.secondaryContainer,
            content = scheme.onSecondaryContainer,
            size = 40.dp,
        )
        Column(
            modifier = Modifier.weight(1f),
            verticalArrangement = Arrangement.spacedBy(2.dp),
        ) {
            Text("上次唤醒", style = MaterialTheme.typography.titleSmallEmphasized)
            Text(
                when {
                    loading && wakeup == null -> "读取中…"
                    wakeup == null -> "尚未读取"
                    !wakeup.available -> wakeup.note ?: "此 ROM 未提供上次唤醒原因"
                    else -> buildString {
                        append(wakeup.reason ?: "未知")
                        wakeup.ageMs?.let { append(" · ${formatAwakeMs(it)} 前") }
                        if (wakeup.cached) append(" · 缓存")
                    }
                },
                style = MaterialTheme.typography.bodySmall,
                color = scheme.onSurfaceVariant,
                maxLines = 3,
                overflow = TextOverflow.Ellipsis,
            )
        }
        if (loading) {
            LoadingIndicator(modifier = Modifier.size(28.dp))
        } else {
            IconButton(onClick = onRefresh) {
                Icon(
                    Icons.Rounded.Refresh,
                    contentDescription = "刷新上次唤醒",
                    tint = scheme.primary,
                )
            }
        }
    }
}

@Composable
private fun SourcesHero(state: WakelockViewModel.UiState, modifier: Modifier = Modifier) {
    val palette = cardPalette(2)
    val summary = state.state?.summary
    WakelockHero(
        palette = palette,
        label = "活跃唤醒源",
        value = summary?.active?.toString() ?: "—",
        unit = summary?.let { "/ ${it.known} 已知" },
        badgeShape = ExpressiveShapes.Cookie12,
        badgeIcon = Icons.Rounded.Bolt,
        loading = summary == null && state.loading,
        modifier = modifier,
    ) {
        if (summary == null) return@WakelockHero
        Column(verticalArrangement = Arrangement.spacedBy(14.dp)) {
            if (!summary.available) {
                Text(
                    text = buildString {
                        append("内核唤醒源不可读：")
                        append(summary.reason ?: "debugfs 未挂载或无权限")
                        append("。已尝试自动挂载 debugfs；应用与持锁两页仍可正常使用。")
                    },
                    style = MaterialTheme.typography.bodyMedium,
                )
            } else {
                if (summary.known > 0) {
                    WavyMeter(
                        progress = summary.active.toFloat() / summary.known,
                        color = palette.accent,
                        trackColor = palette.content.copy(alpha = 0.16f),
                    )
                }
                HeroPills(palette = palette, items = summaryFacts(summary))
            }
        }
    }
}

private fun summaryFacts(summary: WakelockSummary): List<String> = buildList {
    val window = formatAwakeMs(summary.windowSeconds * 1000)
    if (summary.windowSeconds > 0) {
        // A window that restarted at a reboot must not be labelled "24 h".
        add(if (summary.windowReason == "reboot") "重启以来 $window" else "统计窗口 $window")
    } else {
        add(if (summary.windowReason == "reboot") "重启后重新统计" else "统计窗口尚未建立")
    }
    if (summary.measuredMs > 0) {
        add("窗口内唤醒 ${formatAwakeMs(summary.measuredMs)}")
    }
    if (summary.updatedSeconds > 0) {
        val now = System.currentTimeMillis() / 1000
        add("最近采样 ${formatAgeSeconds((now - summary.updatedSeconds).coerceAtLeast(0))}")
    }
}

/** Driver family, how many sources it has and their combined time. */
@Composable
private fun CategoryHeader(
    block: SourceBlock,
    paletteIndex: Int,
    byCount: Boolean,
    modifier: Modifier = Modifier,
) {
    val palette = cardPalette(paletteIndex)
    Row(
        modifier = modifier
            .fillMaxWidth()
            .padding(start = 4.dp, end = 4.dp, top = 16.dp, bottom = 8.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        IconBadge(
            icon = wakeCategoryIcon(block.category),
            container = palette.container,
            content = palette.content,
            size = 40.dp,
        )
        Column(modifier = Modifier.weight(1f)) {
            Text(
                block.label,
                style = MaterialTheme.typography.titleMediumEmphasized,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
            Text(
                "${block.lines.size} 项",
                style = MaterialTheme.typography.labelMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
        InfoPill(
            text = formatMetric(block.totalMetric, byCount),
            container = palette.container,
            content = palette.content,
        )
    }
}

/** One wakeup source: raw driver name (unavoidable), duration, share of the total and a share bar. */
@Composable
private fun SourceRow(
    line: SourceLine,
    fraction: Float,
    byCount: Boolean,
    paletteIndex: Int,
    shape: androidx.compose.ui.graphics.Shape,
    modifier: Modifier = Modifier,
) {
    val scheme = MaterialTheme.colorScheme
    ConnectedItem(
        shape = shape,
        modifier = modifier,
        container = if (line.active) scheme.primaryContainer else scheme.surfaceContainerHigh,
    ) {
        Column(
            modifier = Modifier.weight(1f),
            verticalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            Row(
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                Text(
                    line.name,
                    modifier = Modifier.weight(1f, fill = false),
                    style = MaterialTheme.typography.bodyLarge,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
                if (line.active) StatusPill("正在持锁", StatusTone.Brand)
            }
            WakelockShareBar(
                fraction = fraction,
                color = if (line.active) scheme.primary else accentFor(paletteIndex),
            )
        }
        Column(horizontalAlignment = Alignment.End) {
            Text(
                formatMetric(line.metric, byCount),
                style = MaterialTheme.typography.titleSmallEmphasized,
            )
            Text(
                formatShare(fraction),
                style = MaterialTheme.typography.labelSmall,
                color = if (line.active) {
                    scheme.onPrimaryContainer.copy(alpha = 0.8f)
                } else {
                    scheme.onSurfaceVariant
                },
            )
        }
    }
}

/** Ranking rows show either a duration or a wakeup count; the blocks sum the same unit. */
private fun formatMetric(value: Long, byCount: Boolean): String =
    if (byCount) "$value 次" else formatAwakeMs(value)

private fun formatShare(fraction: Float): String {
    val percent = fraction * 100f
    return if (percent > 0f && percent < 1f) "<1%" else "${percent.roundToInt()}%"
}

/** Icon for a wakeup-source category (protocol §3.18); unknown identifiers get a neutral glyph. */
private fun wakeCategoryIcon(category: String): ImageVector = when (category) {
    "system" -> Icons.Rounded.Settings
    "bluetooth" -> Icons.Rounded.Bluetooth
    "nfc" -> Icons.Rounded.Nfc
    "network" -> Icons.Rounded.Wifi
    "timer" -> Icons.Rounded.Alarm
    "touch" -> Icons.Rounded.TouchApp
    "display" -> Icons.Rounded.LightMode
    "power" -> Icons.Rounded.BatteryChargingFull
    "audio" -> Icons.Rounded.GraphicEq
    "camera" -> Icons.Rounded.CameraAlt
    "sensor" -> Icons.Rounded.Sensors
    "storage" -> Icons.Rounded.Storage
    "usb" -> Icons.Rounded.Usb
    "accessory" -> Icons.Rounded.Extension
    "subsystem" -> Icons.Rounded.Hub
    "serial" -> Icons.Rounded.Cable
    else -> Icons.Rounded.Category
}
