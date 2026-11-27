@file:OptIn(ExperimentalMaterial3ExpressiveApi::class)

package io.sysflow.feature.process

import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.core.animateDpAsState
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.animation.animateColorAsState
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.relocation.BringIntoViewRequester
import androidx.compose.foundation.relocation.bringIntoViewRequester
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.rounded.ExpandMore
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.remember
import androidx.compose.ui.draw.rotate
import kotlinx.coroutines.delay
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.Close
import androidx.compose.material.icons.rounded.Info
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.ButtonGroupDefaults
import androidx.compose.material3.ExperimentalMaterial3ExpressiveApi
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialShapes
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.ToggleButton
import androidx.compose.material3.toShape
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp

/**
 * Building blocks shared by the process page's list and detail sheet.
 *
 * They stay private to this feature on purpose, like the memory page's: the process page has its
 * own small vocabulary (section header, tonal stat tile, note, usage bar, connected choice row)
 * and promoting it to `core/ui` before a second page asks for it would only freeze a shape nobody
 * else needs.
 */

/** Section title with an optional one-line explanation and a trailing action. */
@Composable
internal fun SectionHeader(
    title: String,
    modifier: Modifier = Modifier,
    supporting: String? = null,
    action: (@Composable () -> Unit)? = null,
) {
    Row(
        modifier = modifier
            .fillMaxWidth()
            .padding(start = 8.dp, top = 12.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        Column(modifier = Modifier.weight(1f)) {
            Text(
                title,
                style = MaterialTheme.typography.titleLargeEmphasized,
                color = MaterialTheme.colorScheme.onSurface,
            )
            if (supporting != null) {
                Text(
                    supporting,
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
        }
        action?.invoke()
    }
}

/** Large resting surface that groups the content of one section. */
@Composable
internal fun SectionSurface(
    modifier: Modifier = Modifier,
    container: Color = MaterialTheme.colorScheme.surfaceContainerHigh,
    content: @Composable () -> Unit,
) {
    Surface(
        modifier = modifier.fillMaxWidth(),
        shape = MaterialTheme.shapes.extraLarge,
        color = container,
        content = content,
    )
}

/** A short explanation that must stay next to the controls it qualifies. */
@Composable
internal fun NoteTile(
    text: String,
    modifier: Modifier = Modifier,
    icon: ImageVector = Icons.Rounded.Info,
    container: Color = MaterialTheme.colorScheme.surfaceContainerHigh,
    content: Color = MaterialTheme.colorScheme.onSurfaceVariant,
    onDismiss: (() -> Unit)? = null,
) {
    Surface(
        modifier = modifier.fillMaxWidth(),
        shape = MaterialTheme.shapes.large,
        color = container,
        contentColor = content,
    ) {
        Row(
            modifier = Modifier.padding(
                start = 16.dp,
                top = 12.dp,
                bottom = 12.dp,
                end = if (onDismiss == null) 16.dp else 4.dp,
            ),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            Icon(icon, contentDescription = null, modifier = Modifier.size(20.dp))
            Text(
                text,
                modifier = Modifier.weight(1f),
                style = MaterialTheme.typography.bodySmall,
            )
            if (onDismiss != null) {
                IconButton(onClick = onDismiss) {
                    Icon(Icons.Rounded.Close, contentDescription = "关闭", modifier = Modifier.size(18.dp))
                }
            }
        }
    }
}

/** Label, one number, optional qualifier — on a tonal container. */
@Composable
internal fun StatTile(
    label: String,
    value: String,
    container: Color,
    content: Color,
    modifier: Modifier = Modifier,
    supporting: String? = null,
) {
    Surface(
        modifier = modifier,
        shape = MaterialTheme.shapes.large,
        color = container,
        contentColor = content,
    ) {
        Column(
            modifier = Modifier.padding(horizontal = 16.dp, vertical = 14.dp),
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
            )
            if (supporting != null) {
                Text(
                    supporting,
                    style = MaterialTheme.typography.bodySmall,
                    color = content.copy(alpha = 0.75f),
                    maxLines = 1,
                )
            }
        }
    }
}

/** Nothing to show, and the only way forward is a tap. */
@Composable
internal fun EmptyActionTile(
    icon: ImageVector,
    text: String,
    actionLabel: String,
    onAction: () -> Unit,
    modifier: Modifier = Modifier,
) {
    SectionSurface(modifier) {
        Column(
            modifier = Modifier
                .fillMaxWidth()
                .padding(24.dp),
            horizontalAlignment = Alignment.CenterHorizontally,
            verticalArrangement = Arrangement.spacedBy(16.dp),
        ) {
            Box(
                modifier = Modifier
                    .size(72.dp)
                    .clip(MaterialShapes.Cookie9Sided.toShape())
                    .background(MaterialTheme.colorScheme.secondaryContainer),
                contentAlignment = Alignment.Center,
            ) {
                Icon(
                    icon,
                    contentDescription = null,
                    tint = MaterialTheme.colorScheme.onSecondaryContainer,
                    modifier = Modifier.size(32.dp),
                )
            }
            Text(
                text,
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                textAlign = TextAlign.Center,
            )
            Button(onClick = onAction, shapes = ButtonDefaults.shapes()) {
                Text(actionLabel, maxLines = 1)
            }
        }
    }
}

/** Thin rounded bar; [fraction] is relative to the busiest row on screen, not to 100%. */
@Composable
internal fun UsageBar(
    fraction: Float,
    color: Color,
    track: Color,
    modifier: Modifier = Modifier,
) {
    val animated by animateFloatAsState(
        targetValue = fraction.coerceIn(0f, 1f),
        animationSpec = MaterialTheme.motionScheme.defaultEffectsSpec(),
        label = "processUsage",
    )
    Box(
        modifier = modifier
            .fillMaxWidth()
            .height(4.dp)
            .clip(CircleShape)
            .background(track),
    ) {
        // A visible stub for non-zero values, so a tiny share does not vanish.
        if (animated > 0f) {
            Box(
                modifier = Modifier
                    .fillMaxHeight()
                    .fillMaxWidth(animated.coerceAtLeast(0.03f))
                    .clip(CircleShape)
                    .background(color),
            )
        }
    }
}

/**
 * Connected button group acting as a single-choice control.
 *
 * The checked segment morphs into a full pill while its neighbours keep soft inner corners — the
 * M3 Expressive replacement for the classic segmented control. [compact] uses the extra-small
 * button metrics so five numeric values still fit on a 360 dp phone.
 */
@Composable
internal fun <T> ConnectedChoiceRow(
    options: List<T>,
    selected: T?,
    label: (T) -> String,
    onSelect: (T) -> Unit,
    modifier: Modifier = Modifier,
    enabled: Boolean = true,
    compact: Boolean = false,
    /** Pads a short row with empty space so its segments match the rows above. */
    slots: Int = options.size,
    icon: ((T) -> ImageVector?)? = null,
) {
    Row(
        modifier = modifier.fillMaxWidth(),
        horizontalArrangement = Arrangement.spacedBy(ButtonGroupDefaults.ConnectedSpaceBetween),
    ) {
        options.forEachIndexed { index, option ->
            val shapes = when (index) {
                0 -> ButtonGroupDefaults.connectedLeadingButtonShapes()
                options.lastIndex -> ButtonGroupDefaults.connectedTrailingButtonShapes()
                else -> ButtonGroupDefaults.connectedMiddleButtonShapes()
            }
            val glyph = icon?.invoke(option)
            val body: @Composable androidx.compose.foundation.layout.RowScope.() -> Unit = {
                if (glyph != null) {
                    Icon(
                        glyph,
                        contentDescription = null,
                        modifier = Modifier.size(ButtonDefaults.IconSize),
                    )
                    Spacer(Modifier.size(ButtonDefaults.IconSpacing))
                }
                Text(label(option), maxLines = 1, overflow = TextOverflow.Ellipsis)
            }
            if (compact) {
                ToggleButton(
                    checked = option == selected,
                    onCheckedChange = { onSelect(option) },
                    modifier = Modifier
                        .weight(1f)
                        .heightIn(min = ButtonDefaults.ExtraSmallContainerHeight),
                    enabled = enabled,
                    shapes = shapes,
                    contentPadding = ButtonDefaults.ExtraSmallContentPadding,
                    content = body,
                )
            } else {
                ToggleButton(
                    checked = option == selected,
                    onCheckedChange = { onSelect(option) },
                    modifier = Modifier.weight(1f),
                    enabled = enabled,
                    shapes = shapes,
                    content = body,
                )
            }
        }
        if (slots > options.size) {
            Spacer(Modifier.weight((slots - options.size).toFloat()))
        }
    }
}

/** Icon badge + title + one-line explanation, the head of every action group. */
@Composable
internal fun ActionGroupHeader(
    icon: ImageVector,
    title: String,
    supporting: String,
    modifier: Modifier = Modifier,
    container: Color = MaterialTheme.colorScheme.secondaryContainer,
    content: Color = MaterialTheme.colorScheme.onSecondaryContainer,
) {
    Row(
        modifier = modifier.fillMaxWidth(),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        Box(
            modifier = Modifier
                .size(40.dp)
                .clip(MaterialShapes.Cookie9Sided.toShape())
                .background(container),
            contentAlignment = Alignment.Center,
        ) {
            Icon(icon, contentDescription = null, tint = content, modifier = Modifier.size(22.dp))
        }
        Column(Modifier.weight(1f)) {
            Text(title, style = MaterialTheme.typography.titleMediumEmphasized)
            Text(
                supporting,
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
    }
}

/**
 * One collapsible action group in the detail sheet's connected list.
 *
 * Collapsed it is a single 72 dp-ish row (badge, title, current value, chevron), so six groups cost
 * about one screen instead of five. Expanding morphs the card's corners to fully rounded and
 * reveals the controls; [position]/[count] give neighbours the M3E connected-list shape (large
 * outer corners, small inner ones). The sheet scrolls the opened card into view.
 */
@Composable
internal fun ExpandableActionCard(
    expanded: Boolean,
    onToggle: () -> Unit,
    icon: ImageVector,
    title: String,
    supporting: String,
    position: Int,
    count: Int,
    modifier: Modifier = Modifier,
    badgeContainer: Color = MaterialTheme.colorScheme.secondaryContainer,
    badgeContent: Color = MaterialTheme.colorScheme.onSecondaryContainer,
    content: @Composable ColumnScope.() -> Unit,
) {
    val scheme = MaterialTheme.colorScheme
    val outer = 28.dp
    val inner = 6.dp
    val spec = MaterialTheme.motionScheme.defaultEffectsSpec<androidx.compose.ui.unit.Dp>()
    val top by animateDpAsState(if (expanded || position == 0) outer else inner, spec, label = "actionTop")
    val bottom by animateDpAsState(
        if (expanded || position == count - 1) outer else inner,
        spec,
        label = "actionBottom",
    )
    val container by animateColorAsState(
        if (expanded) scheme.surfaceContainerHighest else scheme.surfaceContainerHigh,
        MaterialTheme.motionScheme.defaultEffectsSpec(),
        label = "actionContainer",
    )
    val rotation by animateFloatAsState(
        if (expanded) 180f else 0f,
        MaterialTheme.motionScheme.defaultEffectsSpec(),
        label = "actionChevron",
    )
    val requester = remember { BringIntoViewRequester() }
    LaunchedEffect(expanded) {
        if (expanded) {
            delay(240) // let the expand animation lay the body out first
            requester.bringIntoView()
        }
    }

    Surface(
        modifier = modifier
            .fillMaxWidth()
            .bringIntoViewRequester(requester),
        shape = RoundedCornerShape(topStart = top, topEnd = top, bottomStart = bottom, bottomEnd = bottom),
        color = container,
    ) {
        Column {
            Row(
                modifier = Modifier
                    .fillMaxWidth()
                    .clickable(onClick = onToggle)
                    .padding(horizontal = 16.dp, vertical = 14.dp),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                ActionGroupHeader(
                    icon = icon,
                    title = title,
                    supporting = supporting,
                    modifier = Modifier.weight(1f),
                    container = badgeContainer,
                    content = badgeContent,
                )
                Icon(
                    Icons.Rounded.ExpandMore,
                    contentDescription = if (expanded) "收起" else "展开",
                    tint = scheme.onSurfaceVariant,
                    modifier = Modifier.rotate(rotation),
                )
            }
            AnimatedVisibility(visible = expanded) {
                Column(
                    modifier = Modifier
                        .fillMaxWidth()
                        .padding(start = 16.dp, end = 16.dp, bottom = 16.dp),
                    verticalArrangement = Arrangement.spacedBy(12.dp),
                    content = content,
                )
            }
        }
    }
}
