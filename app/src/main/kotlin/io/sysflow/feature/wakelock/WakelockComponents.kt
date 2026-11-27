@file:OptIn(ExperimentalMaterial3ExpressiveApi::class, ExperimentalLayoutApi::class)

package io.sysflow.feature.wakelock

import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.RowScope
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.Close
import androidx.compose.material.icons.rounded.ErrorOutline
import androidx.compose.material.icons.rounded.Info
import androidx.compose.material.icons.rounded.WarningAmber
import androidx.compose.material3.ExperimentalMaterial3ExpressiveApi
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.LoadingIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.Shape
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import io.sysflow.core.ui.components.CardPalette
import io.sysflow.core.ui.components.InfoPill
import io.sysflow.core.ui.theme.sysFlowColors

/*
 * Building blocks shared by the three wake-lock pages. Everything here is Material 3 Expressive:
 * large tonal surfaces, Expressive shapes for the focal badge, connected list items, emphasized
 * type and the wavy/loading indicators. Pages compose these instead of styling raw M3 widgets.
 */

/**
 * Hero surface that opens every page: eyebrow label, one big number, an Expressive-shaped badge
 * and a free slot for meters, pills or notes.
 */
@Composable
internal fun WakelockHero(
    palette: CardPalette,
    label: String,
    value: String,
    badgeShape: Shape,
    badgeIcon: ImageVector,
    modifier: Modifier = Modifier,
    unit: String? = null,
    loading: Boolean = false,
    content: @Composable () -> Unit = {},
) {
    Surface(
        modifier = modifier.fillMaxWidth(),
        shape = MaterialTheme.shapes.extraExtraLarge,
        color = palette.container,
        contentColor = palette.content,
    ) {
        Column(
            modifier = Modifier.padding(24.dp),
            verticalArrangement = Arrangement.spacedBy(18.dp),
        ) {
            Row(
                modifier = Modifier.fillMaxWidth(),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(16.dp),
            ) {
                Column(modifier = Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(2.dp)) {
                    Text(
                        label,
                        style = MaterialTheme.typography.labelLarge,
                        color = palette.content.copy(alpha = 0.8f),
                    )
                    Row(verticalAlignment = Alignment.Bottom) {
                        Text(
                            value,
                            style = MaterialTheme.typography.displayMediumEmphasized,
                            maxLines = 1,
                        )
                        if (unit != null) {
                            Text(
                                unit,
                                modifier = Modifier.padding(start = 8.dp, bottom = 8.dp),
                                style = MaterialTheme.typography.titleMediumEmphasized,
                                color = palette.content.copy(alpha = 0.8f),
                                maxLines = 1,
                            )
                        }
                    }
                }
                Box(
                    modifier = Modifier
                        .size(72.dp)
                        .clip(badgeShape)
                        .background(palette.accent),
                    contentAlignment = Alignment.Center,
                ) {
                    if (loading) {
                        LoadingIndicator(modifier = Modifier.size(44.dp), color = palette.container)
                    } else {
                        Icon(
                            imageVector = badgeIcon,
                            contentDescription = null,
                            tint = palette.container,
                            modifier = Modifier.size(34.dp),
                        )
                    }
                }
            }
            content()
        }
    }
}

/** Quiet pills for facts that sit under a hero number (window, cache state, counts). */
@Composable
internal fun HeroPills(palette: CardPalette, items: List<String>) {
    if (items.isEmpty()) return
    FlowRow(
        horizontalArrangement = Arrangement.spacedBy(8.dp),
        verticalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        items.forEach { text ->
            InfoPill(
                text = text,
                container = palette.content.copy(alpha = 0.12f),
                content = palette.content,
            )
        }
    }
}

@Composable
internal fun SectionHeader(
    title: String,
    modifier: Modifier = Modifier,
    caption: String? = null,
) {
    Column(
        modifier = modifier
            .fillMaxWidth()
            .padding(start = 8.dp, end = 8.dp, top = 20.dp, bottom = 8.dp),
        verticalArrangement = Arrangement.spacedBy(2.dp),
    ) {
        Text(
            title,
            style = MaterialTheme.typography.titleLargeEmphasized,
            color = MaterialTheme.colorScheme.onSurface,
        )
        if (caption != null) {
            Text(
                caption,
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
    }
}

internal enum class NoticeKind { Info, Warning, Error }

/** Inline notice (errors, one-off messages, caveats) in a tonal pill-ish container. */
@Composable
internal fun NoticeTile(
    text: String,
    kind: NoticeKind,
    modifier: Modifier = Modifier,
    onDismiss: (() -> Unit)? = null,
) {
    val scheme = MaterialTheme.colorScheme
    val extended = MaterialTheme.sysFlowColors
    val container: Color
    val content: Color
    val icon: ImageVector
    when (kind) {
        NoticeKind.Info -> {
            container = scheme.secondaryContainer
            content = scheme.onSecondaryContainer
            icon = Icons.Rounded.Info
        }
        NoticeKind.Warning -> {
            container = extended.warningContainer
            content = extended.onWarningContainer
            icon = Icons.Rounded.WarningAmber
        }
        NoticeKind.Error -> {
            container = scheme.errorContainer
            content = scheme.onErrorContainer
            icon = Icons.Rounded.ErrorOutline
        }
    }
    Surface(
        modifier = modifier.fillMaxWidth(),
        shape = MaterialTheme.shapes.extraLarge,
        color = container,
        contentColor = content,
    ) {
        Row(
            modifier = Modifier.padding(
                start = 16.dp,
                end = if (onDismiss != null) 4.dp else 16.dp,
                top = 12.dp,
                bottom = 12.dp,
            ),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            Icon(icon, contentDescription = null, modifier = Modifier.size(22.dp))
            Text(
                text,
                modifier = Modifier.weight(1f),
                style = MaterialTheme.typography.bodyMedium,
            )
            if (onDismiss != null) {
                IconButton(onClick = onDismiss) {
                    Icon(Icons.Rounded.Close, contentDescription = "知道了")
                }
            }
        }
    }
}

/** Empty / loading placeholder: an Expressive-shaped badge, a title, one line of explanation. */
@Composable
internal fun WakelockEmptyState(
    icon: ImageVector,
    title: String,
    body: String,
    badgeShape: Shape,
    modifier: Modifier = Modifier,
    loading: Boolean = false,
    action: (@Composable () -> Unit)? = null,
) {
    val scheme = MaterialTheme.colorScheme
    Surface(
        modifier = modifier.fillMaxWidth(),
        shape = MaterialTheme.shapes.extraLarge,
        color = scheme.surfaceContainerLow,
    ) {
        Column(
            modifier = Modifier
                .fillMaxWidth()
                .padding(horizontal = 24.dp, vertical = 32.dp),
            horizontalAlignment = Alignment.CenterHorizontally,
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            Box(
                modifier = Modifier
                    .size(80.dp)
                    .clip(badgeShape)
                    .background(scheme.secondaryContainer),
                contentAlignment = Alignment.Center,
            ) {
                if (loading) {
                    LoadingIndicator(modifier = Modifier.size(48.dp))
                } else {
                    Icon(
                        imageVector = icon,
                        contentDescription = null,
                        tint = scheme.onSecondaryContainer,
                        modifier = Modifier.size(36.dp),
                    )
                }
            }
            Text(
                title,
                style = MaterialTheme.typography.titleMediumEmphasized,
                color = scheme.onSurface,
                textAlign = TextAlign.Center,
            )
            Text(
                body,
                style = MaterialTheme.typography.bodyMedium,
                color = scheme.onSurfaceVariant,
                textAlign = TextAlign.Center,
            )
            if (action != null) {
                Box(modifier = Modifier.padding(top = 4.dp)) { action() }
            }
        }
    }
}

/**
 * One tile of a connected list. Neighbouring tiles use [io.sysflow.core.ui.components.segmentedShape]
 * so the group reads as one object with soft inner corners and a 4 dp gap.
 */
@Composable
internal fun ConnectedItem(
    shape: Shape,
    modifier: Modifier = Modifier,
    container: Color = MaterialTheme.colorScheme.surfaceContainerHigh,
    onClick: (() -> Unit)? = null,
    enabled: Boolean = true,
    content: @Composable RowScope.() -> Unit,
) {
    val body: @Composable () -> Unit = {
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .padding(horizontal = 16.dp, vertical = 14.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(12.dp),
            content = content,
        )
    }
    if (onClick != null) {
        Surface(
            onClick = onClick,
            enabled = enabled,
            modifier = modifier.fillMaxWidth(),
            shape = shape,
            color = container,
            content = body,
        )
    } else {
        Surface(
            modifier = modifier.fillMaxWidth(),
            shape = shape,
            color = container,
            content = body,
        )
    }
}

/** Thick rounded share bar that grows in when the row first composes. */
@Composable
internal fun WakelockShareBar(
    fraction: Float,
    color: Color,
    modifier: Modifier = Modifier,
    height: Dp = 6.dp,
) {
    var target by remember { mutableFloatStateOf(0f) }
    LaunchedEffect(fraction) {
        // A sliver stays visible for tiny non-zero shares instead of vanishing into the track.
        target = if (fraction > 0f) fraction.coerceIn(0.02f, 1f) else 0f
    }
    // Effects spec on purpose: a spatial spring overshoots and `fillMaxWidth` rejects fractions > 1.
    val animated by animateFloatAsState(
        targetValue = target,
        animationSpec = MaterialTheme.motionScheme.defaultEffectsSpec(),
        label = "wakelockShare",
    )
    Box(
        modifier = modifier
            .fillMaxWidth()
            .height(height)
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

/** Accent that stays readable on neutral surfaces (bars, badges); rotates for adjacent groups. */
@Composable
internal fun accentFor(index: Int): Color {
    val scheme = MaterialTheme.colorScheme
    return when (index.mod(3)) {
        0 -> scheme.secondary
        1 -> scheme.tertiary
        else -> scheme.primary
    }
}
