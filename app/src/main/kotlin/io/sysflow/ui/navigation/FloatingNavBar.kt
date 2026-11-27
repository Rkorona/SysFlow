package io.sysflow.ui.navigation

import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.animateColorAsState
import androidx.compose.animation.expandHorizontally
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.shrinkHorizontally
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.selection.selectable
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material3.ExperimentalMaterial3ExpressiveApi
import androidx.compose.material3.FloatingToolbarScrollBehavior
import androidx.compose.material3.HorizontalFloatingToolbar
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp

object FloatingNavBarDefaults {
    /** Space the content should reserve at the bottom for the floating bar. */
    val ContentBottomInset: Dp = 96.dp
}

/**
 * Material 3 Expressive floating navigation bar.
 *
 * The selected destination expands into an "icon + label" pill; all other destinations show only
 * the icon. [scrollBehavior] is shared with the root `nestedScroll` modifier so the bar slides away
 * while the user scrolls down and comes back when they scroll up.
 */
@OptIn(ExperimentalMaterial3ExpressiveApi::class)
@Composable
fun FloatingNavBar(
    current: Destination,
    onSelect: (Destination) -> Unit,
    scrollBehavior: FloatingToolbarScrollBehavior,
    modifier: Modifier = Modifier,
) {
    HorizontalFloatingToolbar(
        expanded = true,
        modifier = modifier,
        scrollBehavior = scrollBehavior,
    ) {
        Destination.entries.forEach { destination ->
            FloatingNavItem(
                destination = destination,
                selected = destination == current,
                onClick = { onSelect(destination) },
                modifier = Modifier.padding(horizontal = 2.dp),
            )
        }
    }
}

@OptIn(ExperimentalMaterial3ExpressiveApi::class)
@Composable
private fun FloatingNavItem(
    destination: Destination,
    selected: Boolean,
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val motion = MaterialTheme.motionScheme
    val colors = MaterialTheme.colorScheme
    val label = stringResource(destination.label)

    val containerColor by animateColorAsState(
        targetValue = if (selected) colors.primaryContainer else Color.Transparent,
        animationSpec = motion.defaultEffectsSpec(),
        label = "navItemContainer",
    )
    val contentColor by animateColorAsState(
        targetValue = if (selected) colors.onPrimaryContainer else colors.onSurfaceVariant,
        animationSpec = motion.defaultEffectsSpec(),
        label = "navItemContent",
    )

    Row(
        modifier = modifier
            .clip(CircleShape)
            .background(containerColor)
            .selectable(selected = selected, role = Role.Tab, onClick = onClick)
            .padding(horizontal = 16.dp, vertical = 12.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Icon(
            imageVector = if (selected) destination.selectedIcon else destination.icon,
            // When selected, the label is visible and carries the accessible name; otherwise the
            // icon needs its own content description.
            contentDescription = if (selected) null else label,
            tint = contentColor,
        )
        AnimatedVisibility(
            visible = selected,
            enter = expandHorizontally(motion.defaultSpatialSpec()) + fadeIn(motion.defaultEffectsSpec()),
            exit = shrinkHorizontally(motion.defaultSpatialSpec()) + fadeOut(motion.defaultEffectsSpec()),
        ) {
            Text(
                text = label,
                modifier = Modifier.padding(start = 8.dp),
                color = contentColor,
                style = MaterialTheme.typography.labelLarge,
                maxLines = 1,
            )
        }
    }
}
