@file:OptIn(ExperimentalMaterial3ExpressiveApi::class)

package io.sysflow.core.ui.components

import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.ExperimentalMaterial3ExpressiveApi
import androidx.compose.material3.Icon
import androidx.compose.material3.LinearWavyProgressIndicator
import androidx.compose.material3.MaterialShapes
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.SwitchColors
import androidx.compose.material3.SwitchDefaults
import androidx.compose.material3.Text
import androidx.compose.material3.toShape
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.Shape
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import io.sysflow.core.ui.theme.sysFlowColors

/** Standard large Expressive surface used by every feature card. */
@Composable
fun ExpressiveCard(
    modifier: Modifier = Modifier,
    containerColor: Color = MaterialTheme.colorScheme.surfaceContainerLow,
    contentColor: Color = MaterialTheme.colorScheme.onSurface,
    content: @Composable ColumnScope.() -> Unit,
) {
    Surface(
        modifier = modifier.fillMaxWidth(),
        color = containerColor,
        contentColor = contentColor,
        shape = MaterialTheme.shapes.extraLarge,
        tonalElevation = 1.dp,
    ) {
        Column(
            modifier = Modifier.padding(20.dp),
            verticalArrangement = Arrangement.spacedBy(12.dp),
            content = content,
        )
    }
}

@Composable
fun SectionLabel(text: String, modifier: Modifier = Modifier) {
    Text(
        text = text,
        modifier = modifier,
        style = MaterialTheme.typography.titleMediumEmphasized,
        color = MaterialTheme.colorScheme.onSurfaceVariant,
    )
}

@Composable
fun StatusPill(
    text: String,
    container: Color = MaterialTheme.colorScheme.secondaryContainer,
    content: Color = MaterialTheme.colorScheme.onSecondaryContainer,
    modifier: Modifier = Modifier,
) {
    Text(
        text = text,
        modifier = modifier
            .clip(CircleShape)
            .background(container)
            .padding(horizontal = 12.dp, vertical = 6.dp),
        style = MaterialTheme.typography.labelMedium,
        color = content,
        maxLines = 1,
    )
}

/** Rounded meter used for CPU load / temperature. */
@Composable
fun WavyMeter(
    progress: Float,
    modifier: Modifier = Modifier,
    color: Color = MaterialTheme.colorScheme.primary,
    trackColor: Color = MaterialTheme.colorScheme.surfaceContainerHighest,
) {
    val animated by animateFloatAsState(
        targetValue = progress.coerceIn(0f, 1f),
        animationSpec = MaterialTheme.motionScheme.defaultEffectsSpec(),
        label = "sysflowMeter",
    )
    LinearWavyProgressIndicator(
        progress = { animated },
        modifier = modifier.fillMaxWidth(),
        color = color,
        trackColor = trackColor,
        // The default amplitude collapses to a flat line at 0% and 100%; keep the wave alive.
        amplitude = { 1f },
    )
}

/** Cookie-shaped index badge ("01", "02"...) shared by every section heading. */
@Composable
fun IndexBadge(index: String, modifier: Modifier = Modifier) {
    Surface(
        modifier = modifier,
        shape = MaterialShapes.Cookie9Sided.toShape(),
        color = MaterialTheme.colorScheme.secondaryContainer,
        contentColor = MaterialTheme.colorScheme.onSecondaryContainer,
    ) {
        Text(
            index,
            modifier = Modifier.padding(horizontal = 9.dp, vertical = 6.dp),
            style = MaterialTheme.typography.labelMedium,
            fontWeight = FontWeight.Bold,
        )
    }
}

/** A number inside an Expressive shape with a short label below. */
@Composable
fun ShapeStat(
    value: String,
    label: String,
    shape: Shape,
    container: Color,
    content: Color,
    modifier: Modifier = Modifier,
) {
    Column(modifier = modifier, horizontalAlignment = Alignment.CenterHorizontally) {
        Box(
            modifier = Modifier
                .size(82.dp)
                .clip(shape)
                .background(container),
            contentAlignment = Alignment.Center,
        ) {
            Text(
                text = value,
                style = MaterialTheme.typography.titleLargeEmphasized,
                color = content,
                maxLines = 1,
            )
        }
        Spacer(Modifier.height(8.dp))
        Text(
            text = label,
            style = MaterialTheme.typography.labelMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            textAlign = TextAlign.Center,
            maxLines = 2,
        )
    }
}

@Composable
fun KeyValueRow(label: String, value: String, modifier: Modifier = Modifier) {
    Row(
        modifier = modifier.fillMaxWidth(),
        horizontalArrangement = Arrangement.SpaceBetween,
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Text(label, style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
        Text(value, style = MaterialTheme.typography.titleSmall, color = MaterialTheme.colorScheme.onSurface)
    }
}


/** Semantic accents used by telemetry cards and control states. */
data class CardPalette(val container: Color, val content: Color, val accent: Color)

/** Rotate through secondary, tertiary and primary tonal containers for adjacent cards. */
@Composable
fun cardPalette(index: Int): CardPalette {
    val colors = MaterialTheme.colorScheme
    return when (index.mod(3)) {
        0 -> CardPalette(colors.secondaryContainer, colors.onSecondaryContainer, colors.secondary)
        1 -> CardPalette(colors.tertiaryContainer, colors.onTertiaryContainer, colors.tertiary)
        else -> CardPalette(
            colors.primaryContainer,
            colors.onPrimaryContainer,
            MaterialTheme.sysFlowColors.containerAccent,
        )
    }
}

/** Colors for a switch rendered on top of a primaryContainer surface. */
@Composable
fun onPrimaryContainerSwitchColors(): SwitchColors {
    val scheme = MaterialTheme.colorScheme
    val accent = MaterialTheme.sysFlowColors.containerAccent
    return SwitchDefaults.colors(
        checkedThumbColor = scheme.primaryContainer,
        checkedTrackColor = accent,
        checkedBorderColor = accent,
        checkedIconColor = accent,
    )
}

/** Additional semantic status tones beyond the standard Material color roles. */
enum class StatusTone { Positive, Warning, Info, Error, Brand }

/** Semantic status pill using the app-wide extended palette. */
@Composable
fun StatusPill(
    text: String,
    tone: StatusTone,
    modifier: Modifier = Modifier,
) {
    val scheme = MaterialTheme.colorScheme
    val extended = MaterialTheme.sysFlowColors
    val (container, content) = when (tone) {
        StatusTone.Positive -> extended.successContainer to extended.onSuccessContainer
        StatusTone.Warning -> extended.warningContainer to extended.onWarningContainer
        StatusTone.Info -> extended.infoContainer to extended.onInfoContainer
        StatusTone.Error -> scheme.errorContainer to scheme.onErrorContainer
        StatusTone.Brand -> scheme.primaryContainer to scheme.onPrimaryContainer
    }
    InfoPill(text, container, content, modifier)
}

/** Compact pill used where a semantic status role is needed. */
@Composable
fun InfoPill(
    text: String,
    container: Color,
    content: Color,
    modifier: Modifier = Modifier,
) {
    Text(
        text = text,
        modifier = modifier
            .clip(CircleShape)
            .background(container)
            .padding(horizontal = 12.dp, vertical = 6.dp),
        style = MaterialTheme.typography.labelMedium,
        color = content,
        maxLines = 1,
    )
}

/** Circular icon badge for feature headers and compact status rows. */
@Composable
fun IconBadge(
    icon: ImageVector,
    modifier: Modifier = Modifier,
    container: Color = MaterialTheme.colorScheme.secondaryContainer,
    content: Color = MaterialTheme.colorScheme.onSecondaryContainer,
    size: Dp = 44.dp,
) {
    Box(
        modifier = modifier
            .size(size)
            .clip(CircleShape)
            .background(container),
        contentAlignment = Alignment.Center,
    ) {
        Icon(
            imageVector = icon,
            contentDescription = null,
            tint = content,
            modifier = Modifier.size(size / 2),
        )
    }
}

/** Section heading token matching the SysFlow M3E hierarchy. */
@Composable
fun SectionTitle(text: String, modifier: Modifier = Modifier) {
    Text(
        text = text,
        modifier = modifier.padding(start = 8.dp, top = 12.dp),
        style = MaterialTheme.typography.titleLargeEmphasized,
        color = MaterialTheme.colorScheme.onSurface,
    )
}

/** Connected-list shape helper. */
fun segmentedShape(index: Int, count: Int): Shape {
    require(count > 0) { "count must be positive" }
    require(index in 0 until count) { "index must be within the item range" }
    val outer = 24.dp
    val inner = 6.dp
    val top = if (index == 0) outer else inner
    val bottom = if (index == count - 1) outer else inner
    return RoundedCornerShape(
        topStart = top,
        topEnd = top,
        bottomStart = bottom,
        bottomEnd = bottom,
    )
}

/** Compact connected value list using the shared surface and shape tokens. */
@Composable
fun SegmentedValueList(
    rows: List<Pair<String, String>>,
    modifier: Modifier = Modifier,
) {
    Column(
        modifier = modifier.fillMaxWidth(),
        verticalArrangement = Arrangement.spacedBy(4.dp),
    ) {
        rows.forEachIndexed { index, (label, value) ->
            Row(
                modifier = Modifier
                    .fillMaxWidth()
                    .clip(segmentedShape(index, rows.size))
                    .background(MaterialTheme.colorScheme.surfaceContainerHigh)
                    .padding(horizontal = 20.dp, vertical = 16.dp),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(16.dp),
            ) {
                Text(
                    text = label,
                    style = MaterialTheme.typography.bodyMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
                Text(
                    text = value,
                    modifier = Modifier.weight(1f),
                    style = MaterialTheme.typography.titleSmallEmphasized,
                    color = MaterialTheme.colorScheme.onSurface,
                    textAlign = TextAlign.End,
                )
            }
        }
    }
}

/** Common M3 Expressive shapes used for purposeful hero/status moments. */
object ExpressiveShapes {
    val Cookie9: Shape @Composable get() = MaterialShapes.Cookie9Sided.toShape()
    val Sunny: Shape @Composable get() = MaterialShapes.Sunny.toShape()
    val Clover4: Shape @Composable get() = MaterialShapes.Clover4Leaf.toShape()
    val Cookie12: Shape @Composable get() = MaterialShapes.Cookie12Sided.toShape()
}
