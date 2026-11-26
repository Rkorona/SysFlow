package io.sysflow.core.ui.components

import androidx.compose.foundation.Canvas
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.Path
import androidx.compose.ui.graphics.PathEffect
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.StrokeJoin
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import kotlin.math.max
import kotlin.math.min

/**
 * Chart primitives shared by the feature screens.
 *
 * These live in `core/ui` rather than in a feature package because more than one feature draws the
 * same shapes: the battery page's tiles and its 24 h curve, and the memory page's pressure curve.
 * Everything here is presentation only — the caller owns the data, the axes, and the copy.
 */

/**
 * Filled trend line used inside live metric tiles.
 *
 * Draws a dashed baseline until two samples exist, so a tile never looks broken while the session
 * buffer is still filling.
 */
@Composable
fun Sparkline(
    values: List<Int>,
    color: Color,
    modifier: Modifier = Modifier,
    height: Dp = 28.dp,
) {
    Canvas(modifier = modifier.fillMaxWidth().height(height)) {
        val inset = 4.dp.toPx()
        val width = size.width - inset * 2
        if (values.size < 2) {
            drawLine(
                color = color.copy(alpha = 0.4f),
                start = Offset(inset, size.height / 2f),
                end = Offset(inset + width, size.height / 2f),
                strokeWidth = 2.dp.toPx(),
                cap = StrokeCap.Round,
                pathEffect = PathEffect.dashPathEffect(floatArrayOf(10f, 12f)),
            )
            return@Canvas
        }
        val min = values.min()
        val max = values.max()
        val span = (max - min).toFloat()
        val stepX = width / (values.size - 1)
        fun yAt(value: Int): Float =
            if (span <= 0f) size.height / 2f
            else size.height * 0.88f - ((value - min) / span) * size.height * 0.76f

        val line = Path()
        val area = Path()
        values.forEachIndexed { index, value ->
            val x = inset + index * stepX
            val y = yAt(value)
            if (index == 0) {
                line.moveTo(x, y)
                area.moveTo(x, size.height)
                area.lineTo(x, y)
            } else {
                line.lineTo(x, y)
                area.lineTo(x, y)
            }
        }
        area.lineTo(inset + width, size.height)
        area.close()

        drawPath(
            path = area,
            brush = Brush.verticalGradient(
                colors = listOf(color.copy(alpha = 0.30f), color.copy(alpha = 0f)),
                startY = 0f,
                endY = size.height,
            ),
        )
        drawPath(
            path = line,
            color = color,
            style = Stroke(width = 2.5.dp.toPx(), cap = StrokeCap.Round, join = StrokeJoin.Round),
        )
        drawCircle(
            color = color,
            radius = 3.5.dp.toPx(),
            center = Offset(inset + (values.size - 1) * stepX, yAt(values.last())),
        )
    }
}

/** One point of a time series: a wall-clock second and the value recorded there. */
data class ChartPoint(val x: Long, val y: Float)

/**
 * Filled line over a time series, for curves that may have gaps.
 *
 * Points are laid out evenly rather than by their timestamps: a curve drawn from a daemon that was
 * asleep for an hour would otherwise collapse that hour into the same pixels as a five-minute gap,
 * and every point here is already a *sample*, not a claim about the minutes between samples.
 *
 * `minY` / `maxY` pin the vertical range (a percentage axis should not rescale itself), otherwise
 * the data decides and a flat series is drawn through the middle instead of as a full-height line.
 */
@Composable
fun LineChart(
    points: List<ChartPoint>,
    color: Color,
    modifier: Modifier = Modifier,
    height: Dp = 120.dp,
    minY: Float? = null,
    maxY: Float? = null,
    fill: Boolean = true,
) {
    Canvas(modifier = modifier.fillMaxWidth().height(height)) {
        val inset = 4.dp.toPx()
        val width = size.width - inset * 2
        if (points.size < 2) {
            drawLine(
                color = color.copy(alpha = 0.4f),
                start = Offset(inset, size.height / 2f),
                end = Offset(inset + width, size.height / 2f),
                strokeWidth = 2.dp.toPx(),
                cap = StrokeCap.Round,
                pathEffect = PathEffect.dashPathEffect(floatArrayOf(10f, 12f)),
            )
            return@Canvas
        }

        val lowest = minY ?: points.minOf { it.y }
        val highest = maxY ?: points.maxOf { it.y }
        val span = (highest - lowest).takeIf { it > 0f } ?: 1f
        val stepX = width / (points.size - 1)
        fun yAt(value: Float): Float =
            size.height * 0.90f - ((value - lowest) / span) * size.height * 0.80f

        val line = Path()
        val area = Path()
        points.forEachIndexed { index, point ->
            val x = inset + index * stepX
            val y = yAt(point.y.coerceIn(lowest, highest))
            if (index == 0) {
                line.moveTo(x, y)
                area.moveTo(x, size.height)
                area.lineTo(x, y)
            } else {
                line.lineTo(x, y)
                area.lineTo(x, y)
            }
        }
        area.lineTo(inset + width, size.height)
        area.close()

        if (fill) {
            drawPath(
                path = area,
                brush = Brush.verticalGradient(
                    colors = listOf(color.copy(alpha = 0.30f), color.copy(alpha = 0f)),
                    startY = 0f,
                    endY = size.height,
                ),
            )
        }
        drawPath(
            path = line,
            color = color,
            style = Stroke(width = 2.5.dp.toPx(), cap = StrokeCap.Round, join = StrokeJoin.Round),
        )
        drawCircle(
            color = color,
            radius = 3.5.dp.toPx(),
            center = Offset(inset + width, yAt(points.last().y.coerceIn(lowest, highest))),
        )
    }
}

/**
 * First and last value of a series, for the caption under a chart.
 *
 * Kept here so two screens cannot disagree about what "the latest sample" means.
 */
internal fun chartBounds(points: List<ChartPoint>): Pair<Float, Float>? {
    if (points.isEmpty()) return null
    return min(points.minOf { it.y }, points.first().y) to max(points.maxOf { it.y }, points.last().y)
}
