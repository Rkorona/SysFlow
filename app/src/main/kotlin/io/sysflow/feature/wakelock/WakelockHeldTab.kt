@file:OptIn(ExperimentalMaterial3ExpressiveApi::class)

package io.sysflow.feature.wakelock

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyListScope
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.Block
import androidx.compose.material.icons.rounded.Check
import androidx.compose.material.icons.rounded.Lock
import androidx.compose.material.icons.rounded.Refresh
import androidx.compose.material.icons.rounded.Timeline
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.ExperimentalMaterial3ExpressiveApi
import androidx.compose.material3.FilledTonalButton
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.LoadingIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import io.sysflow.core.model.WakelockHeldLock
import io.sysflow.core.model.formatAwakeMs
import io.sysflow.core.model.isProtectedPackage
import io.sysflow.core.ui.components.AppIcon
import io.sysflow.core.ui.components.AppIdentity
import io.sysflow.core.ui.components.ExpressiveShapes
import io.sysflow.core.ui.components.IconBadge
import io.sysflow.core.ui.components.StatusPill
import io.sysflow.core.ui.components.StatusTone
import io.sysflow.core.ui.components.cardPalette
import io.sysflow.core.ui.components.segmentedShape

/** 持锁: the userspace locks held right now (`dumpsys power`), refreshed on demand. */
internal fun LazyListScope.heldTab(
    state: WakelockViewModel.UiState,
    identities: Map<String, AppIdentity>,
    protectedPackages: Set<String> = emptySet(),
    onRefresh: () -> Unit,
    onBlock: (String, Boolean) -> Unit = { _, _ -> },
) {
    val held = state.held

    item(key = "held-hero") {
        HeldHero(state = state, modifier = Modifier.animateItem())
    }

    item(key = "held-actions") {
        FilledTonalButton(
            onClick = onRefresh,
            enabled = !state.heldLoading,
            shapes = ButtonDefaults.shapes(),
            modifier = Modifier
                .animateItem()
                .fillMaxWidth()
                .padding(top = 8.dp),
        ) {
            if (state.heldLoading) {
                LoadingIndicator(modifier = Modifier.size(ButtonDefaults.IconSize + 6.dp))
            } else {
                Icon(
                    Icons.Rounded.Refresh,
                    contentDescription = null,
                    modifier = Modifier.size(ButtonDefaults.IconSize),
                )
            }
            Spacer(Modifier.width(ButtonDefaults.IconSpacing))
            Text(if (state.heldLoading) "读取中…" else "刷新持锁列表")
        }
    }

    item(key = "held-list-head") {
        SectionHeader(
            title = "持锁列表",
            caption = "用户态 wakelock（dumpsys power）",
            modifier = Modifier.animateItem(),
        )
    }

    when {
        held == null -> item(key = "held-empty-unknown") {
            WakelockEmptyState(
                icon = Icons.Rounded.Timeline,
                title = if (state.heldLoading) "正在读取…" else "暂无持锁数据",
                body = "该列表来自一次 dumpsys 调用，按需读取。",
                badgeShape = ExpressiveShapes.Cookie9,
                loading = state.heldLoading,
                modifier = Modifier.animateItem(),
            )
        }
        held.locks.isEmpty() -> item(key = "held-empty-none") {
            WakelockEmptyState(
                icon = Icons.Rounded.Check,
                title = "没有持锁",
                body = "当前没有用户态 wakelock，设备可以安心休眠。",
                badgeShape = ExpressiveShapes.Clover4,
                modifier = Modifier.animateItem(),
            )
        }
        else -> itemsIndexed(
            items = held.locks,
            // Tags repeat (one app can hold the same tag twice), so the index keeps keys unique.
            key = { index, lock -> "held-$index-${lock.tag}" },
        ) { index, lock ->
            HeldLockRow(
                lock = lock,
                identity = lock.packageName?.let(identities::get),
                blocked = lock.packageName != null && lock.packageName in state.blocked,
                protected = lock.packageName != null &&
                    (isProtectedPackage(lock.packageName, protectedPackages) ||
                        lock.packageName in state.exempt),
                pending = lock.packageName != null && state.pending == lock.packageName,
                shape = segmentedShape(index, held.locks.size),
                onBlock = { packageName -> onBlock(packageName, true) },
                modifier = Modifier.animateItem(),
            )
        }
    }
}

@Composable
private fun HeldHero(state: WakelockViewModel.UiState, modifier: Modifier = Modifier) {
    val palette = cardPalette(0)
    val held = state.held
    WakelockHero(
        palette = palette,
        label = "当前持锁",
        value = held?.count?.toString() ?: "—",
        unit = held?.let { "个用户态锁" },
        badgeShape = ExpressiveShapes.Clover4,
        badgeIcon = Icons.Rounded.Lock,
        loading = state.heldLoading && held == null,
        modifier = modifier,
    ) {
        HeroPills(
            palette = palette,
            items = buildList {
                add("来自 dumpsys power")
                if (held?.cached == true) add("缓存数据")
            },
        )
    }
}

/** One held lock: owner icon (or a lock badge when no package is known), tag, owner and held time. */
@Composable
private fun HeldLockRow(
    lock: WakelockHeldLock,
    identity: AppIdentity?,
    blocked: Boolean,
    protected: Boolean,
    pending: Boolean,
    shape: androidx.compose.ui.graphics.Shape,
    onBlock: (String) -> Unit,
    modifier: Modifier = Modifier,
) {
    val scheme = MaterialTheme.colorScheme
    ConnectedItem(shape = shape, modifier = modifier) {
        if (lock.packageName != null) {
            AppIcon(icon = identity?.icon, size = 44.dp)
        } else {
            IconBadge(
                icon = Icons.Rounded.Lock,
                container = scheme.secondaryContainer,
                content = scheme.onSecondaryContainer,
                size = 44.dp,
            )
        }
        Column(
            modifier = Modifier.weight(1f),
            verticalArrangement = Arrangement.spacedBy(2.dp),
        ) {
            Text(
                lock.tag,
                style = MaterialTheme.typography.titleSmall,
                maxLines = 2,
                overflow = TextOverflow.Ellipsis,
            )
            Text(
                listOfNotNull(
                    identity?.label ?: lock.owner ?: lock.packageName,
                    lock.kind,
                ).joinToString(" · "),
                style = MaterialTheme.typography.labelSmall,
                color = scheme.onSurfaceVariant,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
        }
        Column(
            horizontalAlignment = androidx.compose.ui.Alignment.End,
            verticalArrangement = Arrangement.spacedBy(4.dp),
        ) {
            Text(
                formatAwakeMs(lock.heldMs),
                style = MaterialTheme.typography.titleSmallEmphasized,
                color = scheme.primary,
            )
            // Block the owner straight from the lock it is holding right now: that is the moment
            // the user can actually see who is responsible.
            val packageName = lock.packageName
            when {
                packageName == null || protected -> Unit
                blocked -> StatusPill("已拦截", StatusTone.Warning)
                pending -> LoadingIndicator(modifier = Modifier.size(18.dp))
                else -> IconButton(
                    onClick = { onBlock(packageName) },
                    modifier = Modifier.size(28.dp),
                ) {
                    Icon(
                        Icons.Rounded.Block,
                        contentDescription = "拦截 $packageName 的唤醒锁",
                        tint = scheme.primary,
                        modifier = Modifier.size(18.dp),
                    )
                }
            }
        }
    }
}
