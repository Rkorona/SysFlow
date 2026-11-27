@file:OptIn(ExperimentalMaterial3ExpressiveApi::class)

package io.sysflow.feature.home

import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.ExperimentalMaterial3ExpressiveApi
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.Path
import androidx.compose.ui.graphics.PathEffect
import androidx.compose.ui.graphics.Shape
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.StrokeJoin
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import io.sysflow.core.model.BatteryHistory
import io.sysflow.core.model.BatteryHistoryPoint
import io.sysflow.core.model.drainSplit
import io.sysflow.core.ui.components.Sparkline
import io.sysflow.core.ui.theme.sysFlowColors
import java.util.Locale

/**
 * Battery charts and the drain statistics derived from the daemon's history file.
 *
 * Split from `BatteryScreen.kt` because these are read-only visualisations over the recorded series:
 * the screen owns layout and interaction, this file owns drawing and the numbers shown next to it.
 */

private fun fmt1(value: Double): String = String.format(Locale.ROOT, "%.1f", value)

/**
 * Charge curve over the last day, plus the drain since the last charge and the on/off split.
 *
 * Reads what the daemon recorded; when it has not sampled yet the section explains itself instead of
 * drawing an invented line.
 */
@Composable
internal fun HistoryChart(history: BatteryHistory?, currentLevel: Int?) {
    val scheme = MaterialTheme.colorScheme
    if (history == null || history.levels.size < 2) {
        Text(
            "暂无历史采样。守护进程每 60 秒记录一次，运行一段时间后这里会出现电量曲线。",
            style = MaterialTheme.typography.bodyMedium,
            color = scheme.onSurfaceVariant,
        )
        return
    }

    val startLevel = history.points.firstOrNull { it.level != null }?.level ?: 0
    val endLevel = currentLevel ?: history.levels.last()
    val hasCharging = history.points.any { it.charging }

    Column(verticalArrangement = Arrangement.spacedBy(14.dp)) {
        Row(
            modifier = Modifier.fillMaxWidth(),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.SpaceBetween,
        ) {
            Text(
                "$startLevel% → $endLevel%",
                style = MaterialTheme.typography.titleMediumEmphasized,
                color = scheme.onSurface,
            )
            Text(
                "${history.points.size} 个采样点 · 步长 ${history.stepSeconds / 3600}h",
                style = MaterialTheme.typography.labelSmall,
                color = scheme.outline,
            )
        }

        LevelChart(points = history.points)

        Row(
            modifier = Modifier.fillMaxWidth(),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.SpaceBetween,
        ) {
            Text("24 小时前", style = MaterialTheme.typography.labelSmall, color = scheme.outline)
            if (hasCharging) {
                Row(
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(6.dp),
                ) {
                    Box(
                        modifier = Modifier
                            .size(10.dp)
                            .clip(CircleShape)
                            .background(MaterialTheme.sysFlowColors.success.copy(alpha = 0.5f)),
                    )
                    Text("充电中", style = MaterialTheme.typography.labelSmall, color = scheme.outline)
                }
            }
            Text("现在", style = MaterialTheme.typography.labelSmall, color = scheme.outline)
        }

        DrainStats(history)

        if (!history.daemonRunning) {
            Text(
                "守护进程未运行，曲线不会继续增长。",
                style = MaterialTheme.typography.labelSmall,
                color = scheme.outline,
            )
        }
    }
}

@Composable
private fun LevelChart(points: List<BatteryHistoryPoint>, modifier: Modifier = Modifier) {
    val scheme = MaterialTheme.colorScheme
    val lineColor = scheme.primary
    val gridColor = scheme.outlineVariant
    val chargingColor = MaterialTheme.sysFlowColors.success
    val dotInner = scheme.surfaceContainerLow
    val ordered = remember(points) { points.filter { it.level != null }.sortedBy { it.ts } }

    Canvas(modifier = modifier.fillMaxWidth().height(148.dp)) {
        if (ordered.size < 2) return@Canvas
        val padV = 8.dp.toPx()
        val padH = 6.dp.toPx()
        val w = size.width - padH * 2
        val h = size.height - padV * 2
        val t0 = ordered.first().ts
        val span = (ordered.last().ts - t0).coerceAtLeast(1L).toFloat()
        fun xAt(ts: Long): Float = padH + (ts - t0) / span * w
        fun yAt(level: Int): Float = padV + (1f - level.coerceIn(0, 100) / 100f) * h

        val dash = PathEffect.dashPathEffect(floatArrayOf(10f, 10f))
        listOf(0, 50, 100).forEach { level ->
            val y = yAt(level)
            drawLine(
                color = gridColor.copy(alpha = 0.6f),
                start = Offset(padH, y),
                end = Offset(padH + w, y),
                strokeWidth = 1.dp.toPx(),
                pathEffect = dash,
            )
        }

        for (i in 1 until ordered.size) {
            val a = ordered[i - 1]
            val b = ordered[i]
            if (a.charging && b.charging) {
                drawRect(
                    color = chargingColor.copy(alpha = 0.16f),
                    topLeft = Offset(xAt(a.ts), padV),
                    size = Size(xAt(b.ts) - xAt(a.ts), h),
                )
            }
        }

        val line = Path()
        val area = Path()
        ordered.forEachIndexed { index, point ->
            val x = xAt(point.ts)
            val y = yAt(point.level ?: 0)
            if (index == 0) {
                line.moveTo(x, y)
                area.moveTo(x, padV + h)
                area.lineTo(x, y)
            } else {
                line.lineTo(x, y)
                area.lineTo(x, y)
            }
        }
        area.lineTo(xAt(ordered.last().ts), padV + h)
        area.close()

        drawPath(
            path = area,
            brush = Brush.verticalGradient(
                colors = listOf(lineColor.copy(alpha = 0.28f), lineColor.copy(alpha = 0f)),
                startY = padV,
                endY = padV + h,
            ),
        )
        drawPath(
            path = line,
            color = lineColor,
            style = Stroke(width = 3.dp.toPx(), cap = StrokeCap.Round, join = StrokeJoin.Round),
        )
        val last = ordered.last()
        val center = Offset(xAt(last.ts), yAt(last.level ?: 0))
        drawCircle(color = lineColor, radius = 6.dp.toPx(), center = center)
        drawCircle(color = dotInner, radius = 2.5.dp.toPx(), center = center)
    }
}

private fun statShape(topStart: Boolean, topEnd: Boolean, bottomStart: Boolean, bottomEnd: Boolean) =
    RoundedCornerShape(
        topStart = if (topStart) 24.dp else 8.dp,
        topEnd = if (topEnd) 24.dp else 8.dp,
        bottomStart = if (bottomStart) 24.dp else 8.dp,
        bottomEnd = if (bottomEnd) 24.dp else 8.dp,
    )

/** Drain since the last charge as a wide tile, with the screen-on / screen-off split beneath it. */
@Composable
private fun DrainStats(history: BatteryHistory) {
    val scheme = MaterialTheme.colorScheme
    val drain = history.drainSinceLastCharge()
    val split = history.drainSplit()

    if (drain == null && !split.hasData) {
        Text(
            "亮屏/熄屏掉电拆分需要守护进程在采样时记录屏幕状态，更新后过一段时间再来看。",
            style = MaterialTheme.typography.labelSmall,
            color = scheme.outline,
        )
        return
    }

    val splitTiles = listOfNotNull(
        split.screenOn?.let { "亮屏掉电" to "${fmt1(it.percentPerHour)}%/h" },
        split.screenOff?.let { "熄屏掉电" to "${fmt1(it.percentPerHour)}%/h" },
    )

    Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
        if (drain != null) {
            val joined = splitTiles.isNotEmpty()
            StatTile(
                label = "上次充电以来",
                value = "-${drain.droppedPercent}%",
                detail = "${fmt1(drain.hours)} 小时 · 约 ${fmt1(drain.percentPerHour)}%/h",
                shape = statShape(true, true, !joined, !joined),
                modifier = Modifier.fillMaxWidth(),
            )
        }
        if (splitTiles.isNotEmpty()) {
            Row(horizontalArrangement = Arrangement.spacedBy(4.dp)) {
                splitTiles.forEachIndexed { index, (label, value) ->
                    val first = index == 0
                    val last = index == splitTiles.lastIndex
                    StatTile(
                        label = label,
                        value = value,
                        shape = statShape(
                            topStart = first && drain == null,
                            topEnd = last && drain == null,
                            bottomStart = first,
                            bottomEnd = last,
                        ),
                        modifier = Modifier.weight(1f),
                    )
                }
            }
        }
    }
}

@Composable
private fun StatTile(
    label: String,
    value: String,
    shape: Shape,
    modifier: Modifier = Modifier,
    detail: String? = null,
) {
    Surface(
        modifier = modifier,
        shape = shape,
        color = MaterialTheme.colorScheme.surfaceContainerHigh,
    ) {
        Row(
            modifier = Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 12.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            Column(modifier = Modifier.weight(1f)) {
                Text(
                    label,
                    style = MaterialTheme.typography.labelMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
                Text(
                    value,
                    style = MaterialTheme.typography.titleLargeEmphasized,
                    color = MaterialTheme.colorScheme.onSurface,
                    maxLines = 1,
                )
            }
            if (detail != null) {
                Text(
                    detail,
                    style = MaterialTheme.typography.labelMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
        }
    }
}
