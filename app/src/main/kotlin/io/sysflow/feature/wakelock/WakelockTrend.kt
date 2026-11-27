package io.sysflow.feature.wakelock

import androidx.compose.foundation.Canvas
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.size
import androidx.compose.material3.ExperimentalMaterial3ExpressiveApi
import androidx.compose.material3.LoadingIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Path
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.StrokeJoin
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.unit.dp
import io.sysflow.core.model.WakelockHistory
import io.sysflow.core.model.WakelockTrendPoint
import io.sysflow.core.model.formatAwakeMs
import io.sysflow.core.ui.components.InfoPill

/**
 * The recorded wake-lock trend.
 *
 * The daemon samples every five minutes, so a fresh install has nothing to draw for a while; the
 * caption says which of "not sampled yet" and "daemon stopped" applies instead of showing an empty
 * box. [byCount] follows the ranking toggle so both views talk about the same dimension.
 */
@OptIn(ExperimentalMaterial3ExpressiveApi::class)
@Composable
internal fun WakeupTrendCard(
    history: WakelockHistory?,
    loading: Boolean,
    byCount: Boolean,
    modifier: Modifier = Modifier,
) {
    val scheme = MaterialTheme.colorScheme
    val points = history?.points.orEmpty()
    Column(
        modifier = modifier.fillMaxWidth(),
        verticalArrangement = Arrangement.spacedBy(6.dp),
    ) {
        Row(
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            Text(
                "唤醒趋势",
                modifier = Modifier.weight(1f),
                style = MaterialTheme.typography.titleMediumEmphasized,
            )
            if (loading) {
                LoadingIndicator(modifier = Modifier.size(18.dp))
            } else {
                InfoPill(
                    text = if (byCount) "按次数" else "按时长",
                    container = scheme.secondaryContainer,
                    content = scheme.onSecondaryContainer,
                )
            }
        }
        Text(
            when {
                points.size >= 2 -> "最近 24 小时 · ${points.size} 个采样点"
                history?.daemonRunning == false -> "守护进程未运行，暂时没有新采样"
                else -> "守护进程每 5 分钟采样一次，运行一段时间后这里会出现曲线"
            },
            style = MaterialTheme.typography.bodySmall,
            color = scheme.onSurfaceVariant,
        )
        if (points.size >= 2) {
            WakelockTrendChart(
                points = points,
                byCount = byCount,
                modifier = Modifier
                    .fillMaxWidth()
                    .height(120.dp),
            )
            val values = points.map { if (byCount) it.wakeups else it.topMs }
            val peak = values.maxOrNull() ?: 0L
            Text(
                text = "峰值 " + if (byCount) "$peak 次" else formatAwakeMs(peak),
                style = MaterialTheme.typography.labelSmall,
                color = scheme.onSurfaceVariant,
            )
        }
    }
}

@Composable
private fun WakelockTrendChart(
    points: List<WakelockTrendPoint>,
    byCount: Boolean,
    modifier: Modifier = Modifier,
) {
    val scheme = MaterialTheme.colorScheme
    val values = points.map {
        (if (byCount) it.wakeups else it.topMs).toFloat().coerceAtLeast(0f)
    }
    val peak = values.maxOrNull() ?: 0f
    Canvas(modifier = modifier) {
        if (values.size < 2 || peak <= 0f) return@Canvas
        val stepX = size.width / (values.size - 1).toFloat()
        val path = Path()
        values.forEachIndexed { index, value ->
            val x = index * stepX
            val y = size.height - (value / peak) * size.height
            if (index == 0) path.moveTo(x, y) else path.lineTo(x, y)
        }
        // Baseline, so a flat line still reads as "nothing happened" rather than "no chart".
        drawLine(
            color = scheme.outlineVariant.copy(alpha = 0.5f),
            start = Offset(0f, size.height),
            end = Offset(size.width, size.height),
            strokeWidth = 1.dp.toPx(),
        )
        drawPath(
            path = path,
            color = scheme.primary,
            style = Stroke(
                width = 2.5.dp.toPx(),
                cap = StrokeCap.Round,
                join = StrokeJoin.Round,
            ),
        )
        drawCircle(
            color = scheme.primary,
            radius = 3.dp.toPx(),
            center = Offset(size.width, size.height - (values.last() / peak) * size.height),
        )
    }
}
