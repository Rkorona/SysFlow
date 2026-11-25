@file:OptIn(ExperimentalMaterial3ExpressiveApi::class)

package io.sysflow.feature.home

import androidx.compose.animation.core.LinearEasing
import androidx.compose.animation.core.RepeatMode
import androidx.compose.animation.core.animateFloat
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.animation.core.infiniteRepeatable
import androidx.compose.animation.core.rememberInfiniteTransition
import androidx.compose.animation.core.tween
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.asPaddingValues
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.navigationBars
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.AutoGraph
import androidx.compose.material.icons.rounded.Refresh
import androidx.compose.material3.ExperimentalMaterial3ExpressiveApi
import androidx.compose.material3.FilledTonalIconButton
import androidx.compose.material3.Icon
import androidx.compose.material3.LoadingIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.SegmentedButton
import androidx.compose.material3.SegmentedButtonDefaults
import androidx.compose.material3.SingleChoiceSegmentedButtonRow
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.State
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.Shape
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import io.sysflow.core.model.BatteryAppEntry
import io.sysflow.core.model.BatteryApps
import io.sysflow.core.ui.components.AppIcon
import io.sysflow.core.ui.components.AppIdentity
import io.sysflow.core.ui.components.ExpressiveShapes
import io.sysflow.core.ui.components.segmentedShape
import java.util.Locale

/**
 * Full BatteryStats per-UID ranking, a page of its own (no sheet, no modal).
 *
 * Layout, top to bottom: a summary card (total estimate + a stacked share bar whose colours match
 * the rank badges below), a filter row, then one connected list of apps. Each row's bar is scaled
 * to the largest visible share so differences stay readable; the printed percentage is always the
 * true share of the reported total.
 */
@Composable
fun BatteryRankingScreen(loadApps: suspend (refresh: Boolean) -> BatteryApps?) {
    val state = rememberBatteryAppsState(loadApps)
    val infos = rememberAppInfos(state.entries)
    var showSystem by rememberSaveable { mutableStateOf(true) }

    val entries = state.entries
    val visible = remember(entries, showSystem) {
        if (showSystem) entries else entries.filterNot { it.isSystemUid }
    }
    val maxShare = remember(visible) { visible.maxOfOrNull { it.share } ?: 0.0 }
    val page = state.page
    val bottomInset = WindowInsets.navigationBars.asPaddingValues().calculateBottomPadding()

    LazyColumn(
        modifier = Modifier.fillMaxSize(),
        contentPadding = PaddingValues(start = 16.dp, end = 16.dp, top = 4.dp, bottom = 16.dp + bottomInset),
        verticalArrangement = Arrangement.spacedBy(4.dp),
    ) {
        if (page != null && page.available && entries.isNotEmpty()) {
            item(key = "summary") {
                RankingSummary(
                    page = page,
                    top = entries.take(3),
                    infos = infos,
                    modifier = Modifier.padding(bottom = 8.dp),
                )
            }
        }

        item(key = "controls") {
            RankingControls(
                showSystem = showSystem,
                onShowSystem = { showSystem = it },
                refreshing = state.refreshing,
                refreshEnabled = !state.refreshing && !state.loading,
                onRefresh = state.refresh,
                modifier = Modifier.padding(top = 4.dp, bottom = 8.dp),
            )
        }

        when {
            state.loading -> item(key = "state") {
                MessageTile("正在读取 BatteryStats…", loading = true)
            }
            state.failed -> item(key = "state") {
                MessageTile("读取失败：无法获取 BatteryStats 数据", error = true)
            }
            page?.available == false -> item(key = "state") {
                MessageTile(page.reason ?: "本机没有暴露 BatteryStats 的功耗估算")
            }
            visible.isEmpty() -> item(key = "state") {
                MessageTile("暂无耗电记录")
            }
            else -> {
                itemsIndexed(visible, key = { _, entry -> entry.uid }) { index, entry ->
                    AppPowerItem(
                        entry = entry,
                        rank = index + 1,
                        info = infos[entry.uid] ?: AppIdentity(entry.name ?: entry.uid, null),
                        shape = segmentedShape(index, visible.size),
                        relative = if (maxShare > 0.0) (entry.share / maxShare).toFloat() else 0f,
                    )
                }
                item(key = "footer") {
                    Text(
                        "数据来自 BatteryStats，为系统估算值，仅供参考",
                        modifier = Modifier.fillMaxWidth().padding(top = 12.dp),
                        style = MaterialTheme.typography.labelSmall,
                        color = MaterialTheme.colorScheme.outline,
                        textAlign = TextAlign.Center,
                    )
                }
            }
        }
    }
}

/** Colours for rank 1–3 (shared with the stacked bar) and a quiet fallback for the rest. */
internal data class RankTone(val badge: Color, val onBadge: Color, val bar: Color)

@Composable
internal fun rankTone(rank: Int): RankTone {
    val scheme = MaterialTheme.colorScheme
    return when (rank) {
        1 -> RankTone(scheme.primary, scheme.onPrimary, scheme.primary)
        2 -> RankTone(scheme.tertiary, scheme.onTertiary, scheme.tertiary)
        3 -> RankTone(scheme.secondary, scheme.onSecondary, scheme.secondary)
        else -> RankTone(
            scheme.surfaceContainerHighest,
            scheme.onSurfaceVariant,
            scheme.primary.copy(alpha = 0.5f),
        )
    }
}

@Composable
private fun RankingSummary(
    page: BatteryApps,
    top: List<BatteryAppEntry>,
    infos: Map<String, AppIdentity>,
    modifier: Modifier = Modifier,
) {
    val scheme = MaterialTheme.colorScheme
    val total = page.totalMah
    val capacity = page.capacityMah
    val ofBattery = if (total != null && capacity != null && capacity > 0.0) total / capacity * 100 else null

    Surface(
        modifier = modifier.fillMaxWidth(),
        shape = MaterialTheme.shapes.extraExtraLarge,
        color = scheme.surfaceContainerLow,
    ) {
        Column(
            modifier = Modifier.padding(24.dp),
            verticalArrangement = Arrangement.spacedBy(20.dp),
        ) {
            Row(
                modifier = Modifier.fillMaxWidth(),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(16.dp),
            ) {
                Column(modifier = Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(2.dp)) {
                    Text(
                        "估算总耗电",
                        style = MaterialTheme.typography.labelLarge,
                        color = scheme.onSurfaceVariant,
                    )
                    Row(verticalAlignment = Alignment.Bottom) {
                        Text(
                            total?.let { String.format(Locale.ROOT, "%.1f", it) } ?: "—",
                            style = MaterialTheme.typography.displayMediumEmphasized,
                            color = scheme.onSurface,
                            maxLines = 1,
                        )
                        Text(
                            " mAh",
                            modifier = Modifier.padding(bottom = 8.dp),
                            style = MaterialTheme.typography.titleMediumEmphasized,
                            color = scheme.onSurfaceVariant,
                        )
                    }
                    Text(
                        buildString {
                            append("共 ${page.count} 项")
                            if (ofBattery != null) {
                                append(" · 约占电池容量 ${String.format(Locale.ROOT, "%.0f", ofBattery)}%")
                            }
                        },
                        style = MaterialTheme.typography.bodySmall,
                        color = scheme.onSurfaceVariant,
                    )
                }
                ShapeBadge(
                    shape = ExpressiveShapes.Cookie12,
                    container = scheme.primaryContainer,
                    content = scheme.onPrimaryContainer,
                    icon = Icons.Rounded.AutoGraph,
                    size = 64.dp,
                )
            }

            ShareStack(top = top, infos = infos)
        }
    }
}

/** Top-3 shares plus "其他" as one segmented bar; the legend names each segment. */
@Composable
private fun ShareStack(top: List<BatteryAppEntry>, infos: Map<String, AppIdentity>) {
    val scheme = MaterialTheme.colorScheme
    val tones = listOf(rankTone(1).bar, rankTone(2).bar, rankTone(3).bar)
    val shares = top.map { it.share.toFloat().coerceAtLeast(0.002f) }
    val other = (1f - top.sumOf { it.share }.toFloat()).coerceAtLeast(0f)
    val showOther = other > 0.005f

    Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
        Row(
            modifier = Modifier.fillMaxWidth().height(18.dp),
            horizontalArrangement = Arrangement.spacedBy(3.dp),
        ) {
            shares.forEachIndexed { index, weight ->
                Box(
                    modifier = Modifier
                        .weight(weight)
                        .fillMaxHeight()
                        .clip(CircleShape)
                        .background(tones[index]),
                )
            }
            if (showOther) {
                Box(
                    modifier = Modifier
                        .weight(other)
                        .fillMaxHeight()
                        .clip(CircleShape)
                        .background(scheme.outlineVariant),
                )
            }
        }
        Row(
            modifier = Modifier.fillMaxWidth(),
            horizontalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            top.forEachIndexed { index, entry ->
                val label = infos[entry.uid]?.label ?: entry.name ?: entry.uid
                LegendItem(color = tones[index], text = label, modifier = Modifier.weight(1f, fill = false))
            }
            if (showOther) {
                LegendItem(color = scheme.outlineVariant, text = "其他", modifier = Modifier.weight(1f, fill = false))
            }
        }
    }
}

@Composable
private fun LegendItem(color: Color, text: String, modifier: Modifier = Modifier) {
    Row(
        modifier = modifier,
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(6.dp),
    ) {
        Box(Modifier.size(8.dp).clip(CircleShape).background(color))
        Text(
            text,
            style = MaterialTheme.typography.labelMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            maxLines = 1,
            overflow = TextOverflow.Ellipsis,
        )
    }
}

@Composable
private fun RankingControls(
    showSystem: Boolean,
    onShowSystem: (Boolean) -> Unit,
    refreshing: Boolean,
    refreshEnabled: Boolean,
    onRefresh: () -> Unit,
    modifier: Modifier = Modifier,
) {
    Row(
        modifier = modifier.fillMaxWidth(),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        SingleChoiceSegmentedButtonRow(modifier = Modifier.weight(1f)) {
            SegmentedButton(
                selected = showSystem,
                onClick = { onShowSystem(true) },
                shape = SegmentedButtonDefaults.itemShape(index = 0, count = 2),
                label = { Text("全部") },
            )
            SegmentedButton(
                selected = !showSystem,
                onClick = { onShowSystem(false) },
                shape = SegmentedButtonDefaults.itemShape(index = 1, count = 2),
                label = { Text("仅应用") },
            )
        }
        val spin: State<Float>? = if (refreshing) {
            rememberInfiniteTransition(label = "refreshSpin").animateFloat(
                initialValue = 0f,
                targetValue = 360f,
                animationSpec = infiniteRepeatable(tween(900, easing = LinearEasing), RepeatMode.Restart),
                label = "refreshSpinAngle",
            )
        } else {
            null
        }
        FilledTonalIconButton(
            onClick = onRefresh,
            enabled = refreshEnabled,
            modifier = Modifier.size(48.dp),
        ) {
            Icon(
                Icons.Rounded.Refresh,
                contentDescription = "刷新",
                modifier = Modifier.graphicsLayer { rotationZ = spin?.value ?: 0f },
            )
        }
    }
}

/**
 * One app in the ranking: rank badge, icon, name, relative share bar and the absolute numbers.
 * Used by the full list and by the three-row preview on the battery page.
 */
@Composable
internal fun AppPowerItem(
    entry: BatteryAppEntry,
    rank: Int,
    info: AppIdentity,
    shape: Shape,
    relative: Float,
    modifier: Modifier = Modifier,
) {
    val scheme = MaterialTheme.colorScheme
    val tone = rankTone(rank)
    Surface(
        modifier = modifier.fillMaxWidth(),
        shape = shape,
        color = scheme.surfaceContainerHigh,
    ) {
        Row(
            modifier = Modifier.padding(start = 14.dp, end = 18.dp, top = 14.dp, bottom = 14.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            RankBadge(rank = rank, tone = tone)
            AppIcon(icon = info.icon, size = 44.dp, highlight = rank <= 3)
            Column(modifier = Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                Row(
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(8.dp),
                ) {
                    Text(
                        info.label,
                        modifier = Modifier.weight(1f, fill = false),
                        style = MaterialTheme.typography.titleMedium,
                        color = scheme.onSurface,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                    )
                    if (entry.isSystemUid) {
                        Text(
                            "系统",
                            modifier = Modifier
                                .clip(CircleShape)
                                .background(scheme.surfaceContainerHighest)
                                .padding(horizontal = 8.dp, vertical = 2.dp),
                            style = MaterialTheme.typography.labelSmall,
                            color = scheme.onSurfaceVariant,
                        )
                    }
                }
                ShareBar(fraction = relative, color = tone.bar)
            }
            Column(horizontalAlignment = Alignment.End) {
                Text(
                    String.format(Locale.ROOT, "%.1f%%", entry.share * 100),
                    style = MaterialTheme.typography.titleMediumEmphasized,
                    color = scheme.onSurface,
                )
                Text(
                    String.format(Locale.ROOT, "%.1f mAh", entry.powerMah),
                    style = MaterialTheme.typography.labelSmall,
                    color = scheme.onSurfaceVariant,
                )
            }
        }
    }
}

@Composable
private fun RankBadge(rank: Int, tone: RankTone) {
    val shape: Shape = when (rank) {
        1 -> ExpressiveShapes.Cookie9
        2 -> ExpressiveShapes.Clover4
        3 -> ExpressiveShapes.Sunny
        else -> CircleShape
    }
    Box(
        modifier = Modifier.size(32.dp).clip(shape).background(tone.badge),
        contentAlignment = Alignment.Center,
    ) {
        Text(
            "$rank",
            style = MaterialTheme.typography.labelLarge,
            color = tone.onBadge,
            maxLines = 1,
        )
    }
}

/** Thick rounded bar that grows in once when the row first appears. */
@Composable
private fun ShareBar(fraction: Float, color: Color) {
    var target by remember { mutableFloatStateOf(0f) }
    LaunchedEffect(fraction) { target = fraction.coerceIn(0f, 1f) }
    // Effects spec on purpose: a spatial spring overshoots, and `fillMaxWidth` rejects fractions > 1.
    val animated by animateFloatAsState(
        targetValue = target,
        animationSpec = MaterialTheme.motionScheme.defaultEffectsSpec(),
        label = "shareBar",
    )
    Box(
        modifier = Modifier
            .fillMaxWidth()
            .height(8.dp)
            .clip(CircleShape)
            .background(MaterialTheme.colorScheme.surfaceContainerHighest),
    ) {
        Box(
            modifier = Modifier
                .fillMaxWidth(animated.coerceIn(0f, 1f))
                .fillMaxHeight()
                .clip(CircleShape)
                .background(color),
        )
    }
}

/** Loading / empty / error placeholder in the same tonal tile language as the list. */
@Composable
internal fun MessageTile(
    text: String,
    modifier: Modifier = Modifier,
    error: Boolean = false,
    loading: Boolean = false,
    shape: Shape = MaterialTheme.shapes.extraLarge,
    action: (@Composable () -> Unit)? = null,
) {
    val scheme = MaterialTheme.colorScheme
    Surface(
        modifier = modifier.fillMaxWidth(),
        shape = shape,
        color = if (error) scheme.errorContainer else scheme.surfaceContainerHigh,
        contentColor = if (error) scheme.onErrorContainer else scheme.onSurfaceVariant,
    ) {
        Row(
            modifier = Modifier.padding(horizontal = 20.dp, vertical = 18.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(14.dp),
        ) {
            if (loading) LoadingIndicator()
            Text(
                text,
                modifier = Modifier.weight(1f),
                style = MaterialTheme.typography.bodyMedium,
            )
            action?.invoke()
        }
    }
}
