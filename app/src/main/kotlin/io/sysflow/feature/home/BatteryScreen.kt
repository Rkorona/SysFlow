@file:OptIn(ExperimentalMaterial3ExpressiveApi::class)

package io.sysflow.feature.home

import androidx.compose.animation.core.LinearEasing
import androidx.compose.animation.core.RepeatMode
import androidx.compose.animation.core.animateFloat
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.animation.core.infiniteRepeatable
import androidx.compose.animation.core.rememberInfiniteTransition
import androidx.compose.animation.core.tween
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.BatteryFull
import androidx.compose.material.icons.rounded.Bolt
import androidx.compose.material.icons.rounded.Info
import androidx.compose.material.icons.rounded.Speed
import androidx.compose.material.icons.rounded.Thermostat
import androidx.compose.material.icons.rounded.Timeline
import androidx.compose.material.icons.rounded.Warning
import androidx.compose.material3.ExperimentalMaterial3ExpressiveApi
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.State
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.Shape
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import io.sysflow.core.model.BatteryApps
import io.sysflow.core.model.BatteryDevice
import io.sysflow.core.model.BatteryHealth
import io.sysflow.core.model.BatteryHistory
import io.sysflow.core.model.BatterySnapshot
import io.sysflow.core.model.BatteryTempTone
import io.sysflow.core.model.batteryHealthLabel
import io.sysflow.core.model.batteryStatusLabel
import io.sysflow.core.model.batteryTempTone
import io.sysflow.core.model.capacityLossMah
import io.sysflow.core.model.chargerLabel
import io.sysflow.core.model.estimateRuntime
import io.sysflow.core.model.formatDuration
import io.sysflow.core.model.formatMah
import io.sysflow.core.model.formatMilliAmp
import io.sysflow.core.model.formatMilliVolt
import io.sysflow.core.model.formatPowerMw
import io.sysflow.core.model.formatTemp
import io.sysflow.core.model.remainingMah
import io.sysflow.core.ui.components.CardPalette
import io.sysflow.core.ui.components.ExpressiveCard
import io.sysflow.core.ui.components.ExpressiveShapes
import io.sysflow.core.ui.components.InfoPill
import io.sysflow.core.ui.components.SegmentedValueList
import io.sysflow.core.ui.components.StatusPill
import io.sysflow.core.ui.components.WavyMeter
import io.sysflow.core.ui.components.cardPalette
import io.sysflow.core.ui.components.Sparkline
import io.sysflow.core.ui.theme.sysFlowColors
import java.util.Locale

/** Session-only buffer: one point per snapshot refresh, dropped when the page leaves composition. */
private const val SERIES_POINTS = 60
private const val SNAPSHOT_INTERVAL_S = 2

/**
 * Battery detail page. Every item is read-only (protocol §3.15), so there is nothing to apply and
 * no pending state — this is a pure view of the module's snapshot.
 *
 * One scrolling column, ordered by how often each part is looked at: the hero (level, charge state,
 * time left), live readings as a connected 2×2 tile grid with session sparklines, the 24 h curve,
 * the ranking preview (full list lives on its own page), then health. Everything is a tonal
 * Expressive surface; there are no modal layers on this page.
 */
@Composable
fun BatteryScreen(
    battery: BatterySnapshot?,
    refreshTick: Long = 0L,
    loadApps: suspend (refresh: Boolean) -> BatteryApps? = { null },
    loadHealth: suspend (refresh: Boolean) -> BatteryHealth? = { null },
    loadHistory: suspend () -> BatteryHistory? = { null },
    onOpenRanking: () -> Unit = {},
) {
    if (battery == null) {
        EmptyBatteryState()
        return
    }

    // Framework-reported values outrank anything we shell out for.
    val context = LocalContext.current
    val deviceChargeMah = remember { BatteryDevice.chargeCounterMah(context) }
    val deviceCycles = remember { BatteryDevice.cycleCount(context) }
    val series = rememberSessionSeries(battery, refreshTick)
    var health by remember { mutableStateOf<BatteryHealth?>(null) }
    var history by remember { mutableStateOf<BatteryHistory?>(null) }
    LaunchedEffect(Unit) {
        health = loadHealth(false)
        history = loadHistory()
    }
    val appsState = rememberBatteryAppsState(loadApps)
    val appInfos = rememberAppInfos(appsState.entries)

    val percent = battery.capacity?.coerceIn(0, 100)
    val warn = percent != null && percent <= 15 && !battery.charging
    val tempTone = batteryTempTone(battery.tempDeciC)
    val runtime = estimateRuntime(battery)
    val remaining = deviceChargeMah ?: remainingMah(battery)
    val loss = capacityLossMah(battery)

    Column(
        modifier = Modifier
            .fillMaxSize()
            .verticalScroll(rememberScrollState())
            .padding(horizontal = 16.dp),
        verticalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        BatteryHero(
            battery = battery,
            percent = percent,
            warn = warn,
            runtimeMinutes = runtime?.minutes,
            charging = runtime?.charging ?: battery.charging,
            remaining = remaining,
        )

        MetricGrid(battery = battery, series = series, tempTone = tempTone)
        when (tempTone) {
            BatteryTempTone.Danger -> NoticeRow(
                icon = Icons.Rounded.Warning,
                text = "温度过高，建议停止充电",
                container = MaterialTheme.colorScheme.errorContainer,
                content = MaterialTheme.colorScheme.onErrorContainer,
            )
            BatteryTempTone.Warn -> NoticeRow(
                icon = Icons.Rounded.Thermostat,
                text = "温度偏高，注意散热",
                container = MaterialTheme.sysFlowColors.warningContainer,
                content = MaterialTheme.sysFlowColors.onWarningContainer,
            )
            BatteryTempTone.Normal -> Unit
        }
        Text(
            "趋势线为最近 ${series.voltage.size.coerceAtLeast(1) * SNAPSHOT_INTERVAL_S} 秒的读数，只在页面打开期间记录，不写入磁盘。",
            modifier = Modifier.padding(horizontal = 8.dp),
            style = MaterialTheme.typography.labelSmall,
            color = MaterialTheme.colorScheme.outline,
        )

        BatterySectionHeader("24 小时电量")
        ExpressiveCard {
            HistoryChart(history = history, currentLevel = percent)
        }

        BatterySectionHeader(
            title = "应用耗电排行",
            trailing = appsState.page?.totalMah?.let {
                "估算 ${String.format(Locale.ROOT, "%.1f", it)} mAh"
            },
        )
        RankingPreview(
            state = appsState,
            infos = appInfos,
            onOpenAll = onOpenRanking,
        )

        BatterySectionHeader("健康度")
        HealthSection(
            battery = battery,
            health = health,
            cycles = deviceCycles ?: health?.cycleCount ?: battery.cycleCount,
            loss = loss,
        )

        NoticeRow(
            icon = Icons.Rounded.Info,
            text = "本模块为只读：不写入任何节点，因此不参与开机恢复、漂移修复与安全模式。充电限流等高风险管理项尚未开放。",
            container = MaterialTheme.colorScheme.surfaceContainerLow,
            content = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        Spacer(Modifier.navigationBarsPadding().height(12.dp))
    }
}

// region hero

@Composable
private fun BatteryHero(
    battery: BatterySnapshot,
    percent: Int?,
    warn: Boolean,
    runtimeMinutes: Int?,
    charging: Boolean,
    remaining: Int?,
) {
    val scheme = MaterialTheme.colorScheme
    val extended = MaterialTheme.sysFlowColors
    val container = if (warn) scheme.errorContainer else scheme.primaryContainer
    val onContainer = if (warn) scheme.onErrorContainer else scheme.onPrimaryContainer

    // The badge only spins while charging, and only then does the infinite transition exist.
    val angle: State<Float>? = if (battery.charging) {
        rememberInfiniteTransition(label = "heroSpin").animateFloat(
            initialValue = 0f,
            targetValue = 360f,
            animationSpec = infiniteRepeatable(tween(14_000, easing = LinearEasing), RepeatMode.Restart),
            label = "heroSpinAngle",
        )
    } else {
        null
    }

    Surface(
        modifier = Modifier.fillMaxWidth(),
        shape = MaterialTheme.shapes.extraExtraLarge,
        color = container,
        contentColor = onContainer,
    ) {
        Column(
            modifier = Modifier.padding(24.dp),
            verticalArrangement = Arrangement.spacedBy(18.dp),
        ) {
            Row(
                modifier = Modifier.fillMaxWidth(),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(14.dp),
            ) {
                ShapeBadge(
                    shape = ExpressiveShapes.Cookie9,
                    container = if (warn) scheme.error else scheme.primary,
                    content = if (warn) scheme.onError else scheme.onPrimary,
                    icon = if (battery.charging) Icons.Rounded.Bolt else Icons.Rounded.BatteryFull,
                    size = 52.dp,
                    rotation = angle,
                )
                Column(modifier = Modifier.weight(1f)) {
                    Text("电池", style = MaterialTheme.typography.titleMediumEmphasized)
                    Text(
                        chargerLabel(battery),
                        style = MaterialTheme.typography.bodySmall,
                        color = onContainer.copy(alpha = 0.78f),
                    )
                }
                StatusPill(
                    text = batteryStatusLabel(battery.status),
                    container = if (battery.charging) extended.successContainer
                    else if (warn) scheme.error
                    else scheme.surfaceContainerHighest,
                    content = if (battery.charging) extended.onSuccessContainer
                    else if (warn) scheme.onError
                    else scheme.onSurfaceVariant,
                )
            }

            Row(verticalAlignment = Alignment.Bottom) {
                Text(
                    text = percent?.toString() ?: "—",
                    style = MaterialTheme.typography.displayLargeEmphasized,
                    maxLines = 1,
                )
                if (percent != null) {
                    Text(
                        "%",
                        modifier = Modifier.padding(start = 4.dp, bottom = 12.dp),
                        style = MaterialTheme.typography.headlineMediumEmphasized,
                        color = onContainer.copy(alpha = 0.78f),
                    )
                }
            }

            if (percent != null) {
                WavyMeter(
                    progress = percent / 100f,
                    color = onContainer,
                    trackColor = onContainer.copy(alpha = 0.2f),
                )
            }

            Row(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                HeroStat(
                    label = "剩余电量",
                    value = formatMah(remaining),
                    background = onContainer.copy(alpha = 0.12f),
                    modifier = Modifier.weight(1f),
                )
                HeroStat(
                    label = if (charging) "预计充满" else "预计可用",
                    value = formatDuration(runtimeMinutes),
                    background = onContainer.copy(alpha = 0.12f),
                    modifier = Modifier.weight(1f),
                )
            }
            if (runtimeMinutes != null) {
                Text(
                    "预计时间按当前电流粗估，仅供参考",
                    style = MaterialTheme.typography.labelSmall,
                    color = onContainer.copy(alpha = 0.7f),
                )
            }
        }
    }
}

@Composable
private fun HeroStat(label: String, value: String, background: Color, modifier: Modifier = Modifier) {
    Column(
        modifier = modifier
            .clip(MaterialTheme.shapes.large)
            .background(background)
            .padding(horizontal = 16.dp, vertical = 14.dp),
        verticalArrangement = Arrangement.spacedBy(2.dp),
    ) {
        Text(label, style = MaterialTheme.typography.labelMedium)
        Text(value, style = MaterialTheme.typography.titleMediumEmphasized, maxLines = 1)
    }
}

/** An Expressive polygon with a glyph inside; [rotation] is read at draw time, never recomposed. */
@Composable
internal fun ShapeBadge(
    shape: Shape,
    container: Color,
    content: Color,
    icon: ImageVector,
    modifier: Modifier = Modifier,
    size: Dp = 44.dp,
    rotation: State<Float>? = null,
) {
    Box(modifier = modifier.size(size), contentAlignment = Alignment.Center) {
        Box(
            modifier = Modifier
                .matchParentSize()
                .graphicsLayer { rotationZ = rotation?.value ?: 0f }
                .clip(shape)
                .background(container),
        )
        Icon(icon, contentDescription = null, tint = content, modifier = Modifier.size(size * 0.5f))
    }
}

// endregion

// region live readings

/**
 * Voltage / current / power / temperature as one connected 2×2 grid: only the outer corners of the
 * group are large, the seams between tiles are tight. Each tile carries its own session sparkline,
 * which replaces the separate "trend" card the page used to have.
 */
@Composable
private fun MetricGrid(battery: BatterySnapshot, series: SessionSeries, tempTone: BatteryTempTone) {
    val scheme = MaterialTheme.colorScheme
    val extended = MaterialTheme.sysFlowColors
    val tempPalette = when (tempTone) {
        BatteryTempTone.Danger -> CardPalette(scheme.errorContainer, scheme.onErrorContainer, scheme.error)
        BatteryTempTone.Warn -> CardPalette(extended.warningContainer, extended.onWarningContainer, extended.warning)
        BatteryTempTone.Normal -> cardPalette(0)
    }

    Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
        Row(horizontalArrangement = Arrangement.spacedBy(4.dp)) {
            MetricTile(
                label = "电压",
                value = formatMilliVolt(battery.voltageMv),
                icon = Icons.Rounded.Bolt,
                values = series.voltage,
                palette = cardPalette(1),
                shape = gridShape(row = 0, column = 0),
                badgeShape = ExpressiveShapes.Cookie9,
                modifier = Modifier.weight(1f),
            )
            MetricTile(
                label = "电流",
                value = formatMilliAmp(battery.currentMa),
                icon = Icons.Rounded.Speed,
                values = series.current,
                palette = cardPalette(0),
                shape = gridShape(row = 0, column = 1),
                badgeShape = ExpressiveShapes.Clover4,
                modifier = Modifier.weight(1f),
            )
        }
        Row(horizontalArrangement = Arrangement.spacedBy(4.dp)) {
            MetricTile(
                label = "功率",
                value = formatPowerMw(battery.powerMw),
                icon = Icons.Rounded.Timeline,
                values = series.power,
                palette = cardPalette(2),
                shape = gridShape(row = 1, column = 0),
                badgeShape = ExpressiveShapes.Sunny,
                modifier = Modifier.weight(1f),
            )
            MetricTile(
                label = "温度",
                value = formatTemp(battery.tempDeciC?.toLong()),
                icon = Icons.Rounded.Thermostat,
                values = series.temp,
                palette = tempPalette,
                shape = gridShape(row = 1, column = 1),
                badgeShape = ExpressiveShapes.Cookie12,
                modifier = Modifier.weight(1f),
            )
        }
    }
}

private fun gridShape(row: Int, column: Int): Shape {
    val outer = 32.dp
    val inner = 8.dp
    return RoundedCornerShape(
        topStart = if (row == 0 && column == 0) outer else inner,
        topEnd = if (row == 0 && column == 1) outer else inner,
        bottomStart = if (row == 1 && column == 0) outer else inner,
        bottomEnd = if (row == 1 && column == 1) outer else inner,
    )
}

@Composable
private fun MetricTile(
    label: String,
    value: String,
    icon: ImageVector,
    values: List<Int>,
    palette: CardPalette,
    shape: Shape,
    badgeShape: Shape,
    modifier: Modifier = Modifier,
) {
    Surface(
        modifier = modifier,
        shape = shape,
        color = palette.container,
        contentColor = palette.content,
    ) {
        Column(
            modifier = Modifier.padding(16.dp),
            verticalArrangement = Arrangement.spacedBy(10.dp),
        ) {
            Row(
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(10.dp),
            ) {
                ShapeBadge(
                    shape = badgeShape,
                    container = palette.accent,
                    content = palette.container,
                    icon = icon,
                    size = 36.dp,
                )
                Text(
                    label,
                    style = MaterialTheme.typography.labelLarge,
                    color = palette.content.copy(alpha = 0.8f),
                )
            }
            Text(
                value,
                style = MaterialTheme.typography.headlineSmallEmphasized,
                maxLines = 1,
            )
            Sparkline(values = values, color = palette.accent, height = 36.dp)
        }
    }
}

private data class SessionSeries(
    val voltage: List<Int> = emptyList(),
    val current: List<Int> = emptyList(),
    val power: List<Int> = emptyList(),
    val temp: List<Int> = emptyList(),
) {
    fun append(battery: BatterySnapshot) = SessionSeries(
        voltage = voltage.plusCapped(battery.voltageMv),
        current = current.plusCapped(battery.currentMa),
        power = power.plusCapped(battery.powerMw),
        temp = temp.plusCapped(battery.tempDeciC),
    )
}

private fun List<Int>.plusCapped(value: Int?): List<Int> {
    if (value == null) return this
    val next = this + value
    return if (next.size > SERIES_POINTS) next.takeLast(SERIES_POINTS) else next
}

/**
 * Rolling in-memory buffer fed by the periodic snapshot refresh.
 *
 * Keyed on the snapshot timestamp rather than on the snapshot value: two consecutive refreshes can
 * carry identical readings, and a flat stretch of the curve is real data that must still advance
 * the series instead of being collapsed away.
 */
@Composable
private fun rememberSessionSeries(battery: BatterySnapshot, refreshTick: Long): SessionSeries {
    var series by remember { mutableStateOf(SessionSeries()) }
    LaunchedEffect(refreshTick) { series = series.append(battery) }
    return series
}

// endregion

// region health

@Composable
private fun HealthSection(
    battery: BatterySnapshot,
    health: BatteryHealth?,
    cycles: Int?,
    loss: Int?,
) {
    val scheme = MaterialTheme.colorScheme
    val full = battery.chargeFullMah
    val design = battery.chargeFullDesignMah
    val healthPercent = health?.healthPercent ?: battery.healthPercent
    val ratioPercent = if (full != null && design != null && design > 0) {
        Math.round(full * 100f / design).coerceIn(0, 100)
    } else {
        null
    }
    val ringPercent = healthPercent ?: ratioPercent
    val sources = listOfNotNull(
        health?.healthSource?.let { "健康度：$it" },
        health?.cycleSource?.let { "循环：$it" },
    ).joinToString(" · ")

    Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
        Surface(
            modifier = Modifier.fillMaxWidth(),
            shape = MaterialTheme.shapes.extraLarge,
            color = scheme.tertiaryContainer,
            contentColor = scheme.onTertiaryContainer,
        ) {
            Row(
                modifier = Modifier.padding(20.dp),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(20.dp),
            ) {
                if (ringPercent != null) {
                    HealthRing(
                        percent = ringPercent,
                        color = scheme.tertiary,
                        track = scheme.onTertiaryContainer.copy(alpha = 0.14f),
                    )
                }
                Column(
                    modifier = Modifier.weight(1f),
                    verticalArrangement = Arrangement.spacedBy(6.dp),
                ) {
                    Text(
                        "健康状态",
                        style = MaterialTheme.typography.labelLarge,
                        color = scheme.onTertiaryContainer.copy(alpha = 0.8f),
                    )
                    Text(
                        batteryHealthLabel(battery.health, healthPercent),
                        style = MaterialTheme.typography.titleLargeEmphasized,
                    )
                    if (cycles != null) {
                        InfoPill(
                            text = "循环 $cycles 次",
                            container = scheme.tertiary,
                            content = scheme.onTertiary,
                        )
                    }
                    if (sources.isNotEmpty()) {
                        Text(
                            sources,
                            style = MaterialTheme.typography.labelSmall,
                            color = scheme.onTertiaryContainer.copy(alpha = 0.7f),
                        )
                    }
                }
            }
        }

        val rows = buildList {
            add("满电容量" to formatMah(full))
            add("设计容量" to formatMah(design))
            if (loss != null && loss > 0) add("相对设计容量损耗" to formatMah(loss))
        }
        SegmentedValueList(rows = rows)
    }
}

/** 270° gauge with a rounded cap; the sweep animates with the app's effects spec (no overshoot). */
@Composable
private fun HealthRing(percent: Int, color: Color, track: Color, modifier: Modifier = Modifier) {
    val progress by animateFloatAsState(
        targetValue = percent.coerceIn(0, 100) / 100f,
        animationSpec = MaterialTheme.motionScheme.defaultEffectsSpec(),
        label = "healthRing",
    )
    Box(modifier = modifier.size(112.dp), contentAlignment = Alignment.Center) {
        Canvas(modifier = Modifier.matchParentSize()) {
            val stroke = 14.dp.toPx()
            val inset = stroke / 2f
            val arcSize = Size(size.width - stroke, size.height - stroke)
            val style = Stroke(width = stroke, cap = StrokeCap.Round)
            drawArc(
                color = track,
                startAngle = 135f,
                sweepAngle = 270f,
                useCenter = false,
                topLeft = Offset(inset, inset),
                size = arcSize,
                style = style,
            )
            drawArc(
                color = color,
                startAngle = 135f,
                sweepAngle = 270f * progress,
                useCenter = false,
                topLeft = Offset(inset, inset),
                size = arcSize,
                style = style,
            )
        }
        Column(horizontalAlignment = Alignment.CenterHorizontally) {
            Text("$percent%", style = MaterialTheme.typography.titleLargeEmphasized)
            Text("健康度", style = MaterialTheme.typography.labelSmall)
        }
    }
}

// endregion

// region shared bits

@Composable
private fun BatterySectionHeader(title: String, trailing: String? = null) {
    Row(
        modifier = Modifier.fillMaxWidth().padding(start = 8.dp, end = 8.dp, top = 12.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Text(
            title,
            modifier = Modifier.weight(1f),
            style = MaterialTheme.typography.titleLargeEmphasized,
            color = MaterialTheme.colorScheme.onSurface,
        )
        if (trailing != null) {
            Text(
                trailing,
                style = MaterialTheme.typography.labelMedium,
                color = MaterialTheme.colorScheme.outline,
            )
        }
    }
}

@Composable
private fun NoticeRow(icon: ImageVector, text: String, container: Color, content: Color) {
    Surface(
        modifier = Modifier.fillMaxWidth(),
        shape = MaterialTheme.shapes.large,
        color = container,
        contentColor = content,
    ) {
        Row(
            modifier = Modifier.padding(horizontal = 16.dp, vertical = 14.dp),
            verticalAlignment = Alignment.Top,
            horizontalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            Icon(icon, contentDescription = null, modifier = Modifier.size(20.dp))
            Text(text, modifier = Modifier.weight(1f), style = MaterialTheme.typography.bodySmall)
        }
    }
}

@Composable
private fun EmptyBatteryState() {
    Box(modifier = Modifier.fillMaxSize().padding(32.dp), contentAlignment = Alignment.Center) {
        Column(
            horizontalAlignment = Alignment.CenterHorizontally,
            verticalArrangement = Arrangement.spacedBy(14.dp),
        ) {
            ShapeBadge(
                shape = ExpressiveShapes.Cookie12,
                container = MaterialTheme.colorScheme.secondaryContainer,
                content = MaterialTheme.colorScheme.onSecondaryContainer,
                icon = Icons.Rounded.BatteryFull,
                size = 96.dp,
            )
            Text(
                "未检测到电池模块",
                style = MaterialTheme.typography.titleLargeEmphasized,
                color = MaterialTheme.colorScheme.onSurface,
            )
            Text(
                "模块未报告任何电池数据，可能是不支持该机型的电源节点",
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                textAlign = TextAlign.Center,
            )
        }
    }
}

// endregion
