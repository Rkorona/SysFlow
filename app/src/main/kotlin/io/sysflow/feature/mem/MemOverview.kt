@file:OptIn(ExperimentalMaterial3ExpressiveApi::class)

package io.sysflow.feature.mem

import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.IntrinsicSize
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.Info
import androidx.compose.material.icons.rounded.SwapVert
import androidx.compose.material.icons.rounded.Warning
import androidx.compose.material3.CircularWavyProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3ExpressiveApi
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.Shape
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import io.sysflow.core.ui.components.IconBadge
import io.sysflow.core.ui.components.InfoPill
import io.sysflow.core.ui.components.LineChart
import io.sysflow.core.ui.components.SegmentedValueList
import io.sysflow.core.ui.components.WavyMeter
import io.sysflow.core.ui.components.segmentedShape
import io.sysflow.core.ui.theme.sysFlowColors

// ---- hero ----------------------------------------------------------------------------------

/**
 * The one number people come for: how much memory is still available.
 *
 * A wavy ring carries the share, the big figure carries the amount. "Used" is `total - available`
 * (the way Android reports it), so the two pills under the figure always add up to the total.
 */
@Composable
internal fun MemoryHero(snapshot: MemSnapshot, modifier: Modifier = Modifier) {
    val scheme = MaterialTheme.colorScheme
    val accent = MaterialTheme.sysFlowColors.containerAccent
    val target = if (snapshot.totalKb <= 0) 0f else snapshot.availableKb.toFloat() / snapshot.totalKb
    val progress by animateFloatAsState(
        targetValue = target.coerceIn(0f, 1f),
        animationSpec = MaterialTheme.motionScheme.defaultEffectsSpec(),
        label = "memAvailable",
    )

    Surface(
        modifier = modifier.fillMaxWidth(),
        shape = MaterialTheme.shapes.extraExtraLarge,
        color = scheme.primaryContainer,
        contentColor = scheme.onPrimaryContainer,
    ) {
        Column(
            modifier = Modifier.padding(20.dp),
            verticalArrangement = Arrangement.spacedBy(20.dp),
        ) {
            Row(
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(16.dp),
            ) {
                Box(
                    modifier = Modifier
                        .size(120.dp)
                        .semantics { contentDescription = "可用内存占 ${snapshot.availablePercent}%" },
                    contentAlignment = Alignment.Center,
                ) {
                    CircularWavyProgressIndicator(
                        progress = { progress },
                        modifier = Modifier.size(120.dp),
                        color = accent,
                        trackColor = scheme.onPrimaryContainer.copy(alpha = 0.16f),
                    )
                    Column(horizontalAlignment = Alignment.CenterHorizontally) {
                        Text(
                            "${snapshot.availablePercent}%",
                            style = MaterialTheme.typography.titleLargeEmphasized,
                        )
                        Text(
                            "可用",
                            style = MaterialTheme.typography.labelMedium,
                            color = scheme.onPrimaryContainer.copy(alpha = 0.75f),
                        )
                    }
                }
                Column(
                    modifier = Modifier.weight(1f),
                    verticalArrangement = Arrangement.spacedBy(4.dp),
                ) {
                    Text(
                        "可用内存",
                        style = MaterialTheme.typography.labelLarge,
                        color = scheme.onPrimaryContainer.copy(alpha = 0.75f),
                    )
                    Text(
                        formatKb(snapshot.availableKb),
                        // 32 sp: the ring leaves ~150 dp on a 360 dp phone, and "10.25 GB" must not ellipsize.
                        style = MaterialTheme.typography.headlineLargeEmphasized,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                    )
                }
            }
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                HeroPill("共 ${formatKb(snapshot.totalKb)}")
                HeroPill("已用 ${formatKb(snapshot.usedKb)}")
            }
        }
    }
}

@Composable
private fun HeroPill(text: String) {
    InfoPill(
        text = text,
        container = MaterialTheme.colorScheme.onPrimaryContainer.copy(alpha = 0.12f),
        content = MaterialTheme.colorScheme.onPrimaryContainer,
    )
}

// ---- composition ---------------------------------------------------------------------------

/**
 * Where the used memory actually goes.
 *
 * Measured slices come first, then vendor pools, then the remainder. Pools are capacities, not
 * proven usage, so they are drawn dimmer and captioned as pools — see `docs/modules/mem.md`.
 * The bar and the legend share one index → colour mapping, so a slice can always be found in the list.
 */
@Composable
internal fun CompositionSection(snapshot: MemSnapshot, modifier: Modifier = Modifier) {
    val slices = snapshot.fullSegments.filter { it.kb > 0 }
    val whole = snapshot.totalKb.coerceAtLeast(1)

    Column(modifier.fillMaxWidth(), verticalArrangement = Arrangement.spacedBy(12.dp)) {
        SectionHeader("内存构成", supporting = "这台设备的内存被谁占着")
        SectionSurface {
            Column(
                modifier = Modifier.padding(20.dp),
                verticalArrangement = Arrangement.spacedBy(14.dp),
            ) {
                Text(
                    "设备内存 ${formatKb(snapshot.totalKb)}",
                    style = MaterialTheme.typography.titleMediumEmphasized,
                )
                SegmentBar(slices)
            }
        }
        Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
            slices.forEachIndexed { index, segment ->
                LegendRow(
                    segment = segment,
                    color = segmentColor(segment.kind, index),
                    share = percent(segment.kb, whole),
                    shape = segmentedShape(index, slices.size),
                )
            }
        }
        NoteTile(
            "已用 = 匿名页 + Slab + 内核 + GPU + 厂商池 + 其他；「缓存」可随时回收，不计入已用；「空闲」是真正未被占用的部分。",
        )
    }
}

/** Slices separated by small gaps; only the two ends are fully round, like a connected button group. */
@Composable
private fun SegmentBar(slices: List<MemSegment>, modifier: Modifier = Modifier) {
    val total = slices.sumOf { it.kb }.coerceAtLeast(1)
    Row(
        modifier = modifier
            .fillMaxWidth()
            .height(40.dp),
        horizontalArrangement = Arrangement.spacedBy(3.dp),
    ) {
        slices.forEachIndexed { index, segment ->
            val start = if (index == 0) 20.dp else 6.dp
            val end = if (index == slices.lastIndex) 20.dp else 6.dp
            Box(
                Modifier
                    .weight(segment.kb.toFloat() / total)
                    .fillMaxHeight()
                    .clip(
                        RoundedCornerShape(
                            topStart = start,
                            bottomStart = start,
                            topEnd = end,
                            bottomEnd = end,
                        ),
                    )
                    .background(segmentColor(segment.kind, index)),
            )
        }
    }
}

@Composable
private fun LegendRow(segment: MemSegment, color: Color, share: String, shape: Shape) {
    val scheme = MaterialTheme.colorScheme
    val faint = segment.kind == MemKind.Idle || segment.kind == MemKind.Reclaimable
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .clip(shape)
            .background(scheme.surfaceContainerHigh)
            .padding(horizontal = 16.dp, vertical = 12.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(14.dp),
    ) {
        Box(
            Modifier
                .size(20.dp)
                .clip(CircleShape)
                .background(color)
                // Faint slices would disappear on the row; an outline keeps the swatch findable.
                .then(if (faint) Modifier.border(1.5.dp, scheme.outlineVariant, CircleShape) else Modifier),
        )
        Column(modifier = Modifier.weight(1f)) {
            Text(segment.label, style = MaterialTheme.typography.bodyLarge)
            segment.kind.caption()?.let { caption ->
                Text(
                    caption,
                    style = MaterialTheme.typography.bodySmall,
                    color = scheme.onSurfaceVariant,
                )
            }
        }
        Column(horizontalAlignment = Alignment.End) {
            Text(formatKbShort(segment.kb), style = MaterialTheme.typography.titleSmallEmphasized)
            Text(
                share,
                style = MaterialTheme.typography.labelMedium,
                color = scheme.onSurfaceVariant,
            )
        }
    }
}

private fun MemKind.caption(): String? = when (this) {
    MemKind.Measured -> null
    MemKind.Pool -> "厂商池 · 容量，非实测用量"
    MemKind.Other -> "已用但未能归因的部分"
    MemKind.Reclaimable -> "可随时回收，不计入已用"
    MemKind.Idle -> "真正未被占用"
}

/**
 * The visual weight follows the meaning: measured slices are solid, pools are muted, cache is
 * translucent, and "other"/"idle" are neutral rather than pretending to be a category.
 */
@Composable
private fun segmentColor(kind: MemKind, index: Int): Color {
    val scheme = MaterialTheme.colorScheme
    val extended = MaterialTheme.sysFlowColors
    val base = when (index % 5) {
        0 -> scheme.primary
        1 -> scheme.tertiary
        2 -> scheme.secondary
        3 -> extended.info
        else -> extended.success
    }
    return when (kind) {
        MemKind.Measured -> base.copy(alpha = 0.85f)
        MemKind.Pool -> base.copy(alpha = 0.45f)
        MemKind.Reclaimable -> scheme.secondary.copy(alpha = 0.30f)
        MemKind.Other -> scheme.outlineVariant
        MemKind.Idle -> scheme.surfaceContainerHighest
    }
}

// ---- pressure ------------------------------------------------------------------------------

/** PSI now (two tiles) and over the last 24 h (one chart with both series). */
@Composable
internal fun PressureSection(
    snapshot: MemSnapshot,
    history: MemHistory,
    historyLoading: Boolean,
    onRefresh: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val extended = MaterialTheme.sysFlowColors
    Column(modifier.fillMaxWidth(), verticalArrangement = Arrangement.spacedBy(12.dp)) {
        SectionHeader("内存压力", supporting = "PSI · 最近 10 秒平均")
        Row(
            modifier = Modifier.height(IntrinsicSize.Max),
            horizontalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            StatTile(
                label = "full",
                value = formatPsi(snapshot.psiFullPct),
                supporting = "所有任务同时被卡住",
                container = extended.warningContainer,
                content = extended.onWarningContainer,
                modifier = Modifier
                    .weight(1f)
                    .fillMaxHeight(),
            )
            StatTile(
                label = "some",
                value = formatPsi(snapshot.psiSomePct),
                supporting = "至少一个任务被卡住",
                container = extended.infoContainer,
                content = extended.onInfoContainer,
                modifier = Modifier
                    .weight(1f)
                    .fillMaxHeight(),
            )
        }
        PressureHistoryCard(history, historyLoading, onRefresh)
    }
}

@Composable
private fun PressureHistoryCard(
    history: MemHistory,
    loading: Boolean,
    onRefresh: () -> Unit,
) {
    val extended = MaterialTheme.sysFlowColors
    SectionSurface {
        Column(
            modifier = Modifier.padding(20.dp),
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            Row(
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(12.dp),
            ) {
                Column(modifier = Modifier.weight(1f)) {
                    Text("近 24 小时", style = MaterialTheme.typography.titleMediumEmphasized)
                    Text(
                        "纵轴单位 0.01 %，越高表示越多时间卡在回收上",
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
                RefreshAction(loading = loading, description = "刷新曲线", onClick = onRefresh)
            }
            if (history.psiFull.isEmpty() && history.psiSome.isEmpty()) {
                NoteTile("还没有历史采样。守护进程每 5 分钟记录一次，开启后这里会显示 24 小时曲线。")
            } else {
                // One shared scale, so the two curves can be compared by eye.
                val peak = (history.psiFull + history.psiSome).maxOfOrNull { it.y } ?: 0f
                val ceiling = (peak * 1.15f).coerceAtLeast(100f)
                Box {
                    if (history.psiSome.isNotEmpty()) {
                        LineChart(
                            points = history.psiSome,
                            color = extended.info,
                            height = 140.dp,
                            minY = 0f,
                            maxY = ceiling,
                            fill = false,
                        )
                    }
                    if (history.psiFull.isNotEmpty()) {
                        LineChart(
                            points = history.psiFull,
                            color = extended.warning,
                            height = 140.dp,
                            minY = 0f,
                            maxY = ceiling,
                        )
                    }
                }
                Row(horizontalArrangement = Arrangement.spacedBy(16.dp)) {
                    ChartKey("full", extended.warning)
                    ChartKey("some", extended.info)
                }
            }
        }
    }
}

@Composable
private fun ChartKey(label: String, color: Color) {
    Row(
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(6.dp),
    ) {
        Box(
            Modifier
                .size(10.dp)
                .clip(CircleShape)
                .background(color),
        )
        Text(
            label,
            style = MaterialTheme.typography.labelMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
    }
}

// ---- swap ----------------------------------------------------------------------------------

/** Swap usage as a wavy meter, with the zRAM facts as a connected list underneath. */
@Composable
internal fun SwapSection(snapshot: MemSnapshot, modifier: Modifier = Modifier) {
    val total = snapshot.swapTotalKb
    val used = snapshot.swapUsedKb
    val scheme = MaterialTheme.colorScheme

    Column(modifier.fillMaxWidth(), verticalArrangement = Arrangement.spacedBy(12.dp)) {
        SectionHeader("交换 / zRAM", supporting = "把冷页压缩或换出，换取更多可用内存")
        if (total == null || used == null) {
            NoteTile("没有交换分区。", icon = Icons.Rounded.Info)
            return@Column
        }
        SectionSurface {
            Column(
                modifier = Modifier.padding(20.dp),
                verticalArrangement = Arrangement.spacedBy(16.dp),
            ) {
                Row(
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(14.dp),
                ) {
                    IconBadge(
                        icon = Icons.Rounded.SwapVert,
                        container = scheme.secondaryContainer,
                        content = scheme.onSecondaryContainer,
                        size = 48.dp,
                    )
                    Column(modifier = Modifier.weight(1f)) {
                        Text(
                            "已用 ${formatKb(used)}",
                            style = MaterialTheme.typography.titleMediumEmphasized,
                        )
                        Text(
                            "共 ${formatKb(total)}",
                            style = MaterialTheme.typography.bodySmall,
                            color = scheme.onSurfaceVariant,
                        )
                    }
                    if (total > 0) {
                        InfoPill(
                            text = percent(used, total),
                            container = scheme.secondaryContainer,
                            content = scheme.onSecondaryContainer,
                        )
                    }
                }
                WavyMeter(
                    progress = if (total > 0) used.toFloat() / total else 0f,
                    color = scheme.secondary,
                )
            }
        }
        val facts = buildList {
            snapshot.zramRatio?.let { add("压缩比" to formatRatio(it)) }
            snapshot.zramMemUsedKb?.let { add("压缩后占用" to formatKb(it)) }
            snapshot.zramCount?.let { add("zRAM 设备" to "$it 个") }
        }
        if (facts.isNotEmpty()) SegmentedValueList(facts)
        if (snapshot.zramWriteback) {
            val extended = MaterialTheme.sysFlowColors
            NoteTile(
                "该设备启用了回写（RAM Plus）：部分页会落到闪存，压缩比与「已用」不再严格对应。",
                icon = Icons.Rounded.Warning,
                container = extended.warningContainer,
                content = extended.onWarningContainer,
            )
        }
    }
}

// ---- raw readings --------------------------------------------------------------------------

/** The numbers behind everything above, for people who want to check the arithmetic. */
@Composable
internal fun DetailsSection(
    snapshot: MemSnapshot,
    onRefresh: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val rows = buildList {
        snapshot.freeKb?.let { add("空闲" to formatKb(it)) }
        snapshot.cachedKb?.let { add("页缓存" to formatKb(it)) }
        snapshot.anonKb?.let { add("匿名页" to formatKb(it)) }
        snapshot.slabKb?.let { add("Slab" to formatKb(it)) }
        snapshot.dirtyKb?.let { add("待写回" to formatKb(it)) }
        snapshot.zramDisksizeKb?.let { add("zRAM 容量" to formatKb(it)) }
    }
    Column(modifier.fillMaxWidth(), verticalArrangement = Arrangement.spacedBy(12.dp)) {
        SectionHeader(
            "详细读数",
            action = { RefreshAction(loading = false, description = "刷新读数", onClick = onRefresh) },
        )
        if (rows.isNotEmpty()) SegmentedValueList(rows)
    }
}
