@file:OptIn(ExperimentalMaterial3ExpressiveApi::class)

package io.sysflow.feature.wakelock

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyListScope
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.Apps
import androidx.compose.material.icons.rounded.Block
import androidx.compose.material.icons.rounded.Check
import androidx.compose.material.icons.rounded.Lock
import androidx.compose.material.icons.rounded.LockOpen
import androidx.compose.material.icons.rounded.Refresh
import androidx.compose.material.icons.rounded.Timeline
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.ExperimentalMaterial3ExpressiveApi
import androidx.compose.material3.FilledTonalButton
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.LoadingIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Switch
import androidx.compose.material3.SwitchDefaults
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Shape
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import io.sysflow.core.model.WakelockAppRow
import io.sysflow.core.model.formatAwakeMs
import io.sysflow.core.ui.components.AppIcon
import io.sysflow.core.ui.components.AppIdentity
import io.sysflow.core.ui.components.ExpressiveShapes
import io.sysflow.core.ui.components.StatusPill
import io.sysflow.core.ui.components.StatusTone
import io.sysflow.core.ui.components.cardPalette
import io.sysflow.core.ui.components.segmentedShape

/** 应用: BatteryStats wake-lock time and the AppOps block switch in one connected list. */
internal fun LazyListScope.appsTab(
    state: WakelockViewModel.UiState,
    rows: List<WakelockAppRow>,
    identities: Map<String, AppIdentity>,
    onLoadApps: () -> Unit,
    onOpenPicker: (WakelockPickerMode) -> Unit,
    onToggle: (String, Boolean) -> Unit,
    onExempt: (String, Boolean) -> Unit,
    onReleaseAll: () -> Unit,
) {
    item(key = "apps-hero") {
        AppsHero(state = state, modifier = Modifier.animateItem())
    }

    item(key = "apps-actions") {
        Column(
            modifier = Modifier
                .animateItem()
                .fillMaxWidth()
                .padding(top = 8.dp),
            verticalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                Button(
                    onClick = { onOpenPicker(WakelockPickerMode.Block) },
                    shapes = ButtonDefaults.shapes(),
                    modifier = Modifier.weight(1f),
                ) {
                    Icon(
                        Icons.Rounded.Block,
                        contentDescription = null,
                        modifier = Modifier.size(ButtonDefaults.IconSize),
                    )
                    Spacer(Modifier.width(ButtonDefaults.IconSpacing))
                    Text("选择拦截应用", maxLines = 1)
                }
                FilledTonalButton(
                    onClick = { onOpenPicker(WakelockPickerMode.Exempt) },
                    shapes = ButtonDefaults.shapes(),
                    modifier = Modifier.weight(1f),
                ) {
                    Icon(
                        Icons.Rounded.Lock,
                        contentDescription = null,
                        modifier = Modifier.size(ButtonDefaults.IconSize),
                    )
                    Spacer(Modifier.width(ButtonDefaults.IconSpacing))
                    Text("保护应用", maxLines = 1)
                }
            }
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                FilledTonalButton(
                    onClick = onLoadApps,
                    enabled = !state.appsLoading,
                    shapes = ButtonDefaults.shapes(),
                    modifier = Modifier.weight(1f),
                ) {
                    if (state.appsLoading) {
                        LoadingIndicator(modifier = Modifier.size(ButtonDefaults.IconSize + 6.dp))
                    } else {
                        Icon(
                            Icons.Rounded.Refresh,
                            contentDescription = null,
                            modifier = Modifier.size(ButtonDefaults.IconSize),
                        )
                    }
                    Spacer(Modifier.width(ButtonDefaults.IconSpacing))
                    Text(if (state.appsLoading) "统计中" else "重新统计", maxLines = 1)
                }
                FilledTonalButton(
                    onClick = onReleaseAll,
                    enabled = !state.busy,
                    shapes = ButtonDefaults.shapes(),
                    modifier = Modifier.weight(1f),
                ) {
                    if (state.busy) {
                        LoadingIndicator(modifier = Modifier.size(ButtonDefaults.IconSize + 6.dp))
                    } else {
                        Icon(
                            Icons.Rounded.Check,
                            contentDescription = null,
                            modifier = Modifier.size(ButtonDefaults.IconSize),
                        )
                    }
                    Spacer(Modifier.width(ButtonDefaults.IconSpacing))
                    Text(if (state.busy) "恢复中" else "全部恢复", maxLines = 1)
                }
            }
        }
    }

    item(key = "apps-list-head") {
        SectionHeader(
            title = "应用列表",
            caption = "打开开关即拦截该应用的唤醒锁；按持锁时长排序",
            modifier = Modifier.animateItem(),
        )
    }

    if (rows.isEmpty()) {
        item(key = "apps-empty") {
            val loading = state.appsLoading
            val startAction: (@Composable () -> Unit)? = if (!loading && state.apps == null) {
                @Composable {
                    FilledTonalButton(onClick = onLoadApps, shapes = ButtonDefaults.shapes()) {
                        Text("开始统计")
                    }
                }
            } else {
                null
            }
            WakelockEmptyState(
                icon = if (state.apps == null) Icons.Rounded.Timeline else Icons.Rounded.Check,
                title = when {
                    loading -> "正在统计…"
                    state.apps == null -> "尚未统计"
                    else -> "没有应用持锁"
                },
                body = when {
                    loading -> "需要读取一次 batterystats，可能要几秒钟"
                    state.apps == null -> "该项需要一次 dumpsys batterystats 调用，按需触发。"
                    else -> "这个统计窗口内没有应用持锁。"
                },
                badgeShape = ExpressiveShapes.Sunny,
                loading = loading,
                modifier = Modifier.animateItem(),
                action = startAction,
            )
        }
    } else {
        val maxMs = rows.maxOfOrNull { it.totalMs ?: 0L } ?: 0L
        itemsIndexed(rows, key = { _, row -> "app-${row.key}" }) { index, row ->
            AppWakeRow(
                row = row,
                identity = row.packageName?.let(identities::get),
                relative = row.totalMs?.let { if (maxMs > 0) it.toFloat() / maxMs else 0f },
                pending = row.packageName != null && state.pending == row.packageName,
                shape = segmentedShape(index, rows.size),
                onToggle = { want -> row.packageName?.let { onToggle(it, want) } },
                modifier = Modifier.animateItem(),
            )
        }
    }

    if (state.exempt.isNotEmpty()) {
        item(key = "exempt-head") {
            SectionHeader(
                title = "保护名单",
                caption = "这些应用永远不会被拦截；系统关键应用无需添加",
                modifier = Modifier.animateItem(),
            )
        }
        val packages = state.exempt.sorted()
        itemsIndexed(packages, key = { _, packageName -> "exempt-$packageName" }) { index, packageName ->
            ExemptRow(
                packageName = packageName,
                identity = identities[packageName],
                shape = segmentedShape(index, packages.size),
                pending = state.pending == packageName,
                onRemove = { onExempt(packageName, false) },
                modifier = Modifier.animateItem(),
            )
        }
    }

    item(key = "apps-footnote") {
        NoticeTile(
            text = "拦截会推迟该应用的唤醒任务，可能导致通知、后台同步不及时。系统关键应用不允许拦截。",
            kind = NoticeKind.Warning,
            modifier = Modifier
                .animateItem()
                .padding(top = 12.dp),
        )
    }
}

@Composable
private fun AppsHero(state: WakelockViewModel.UiState, modifier: Modifier = Modifier) {
    val palette = cardPalette(1)
    val apps = state.apps
    WakelockHero(
        palette = palette,
        label = "应用持锁总时长",
        value = apps?.let { formatAwakeMs(it.totalMs) } ?: "—",
        badgeShape = ExpressiveShapes.Sunny,
        badgeIcon = Icons.Rounded.Apps,
        loading = state.appsLoading && apps == null,
        modifier = modifier,
    ) {
        HeroPills(
            palette = palette,
            items = buildList {
                add("自上次充电")
                if (apps != null) add("已统计 ${apps.entries.size} 个应用")
                add("已拦截 ${state.blocked.size} 个")
                if (state.exempt.isNotEmpty()) add("已保护 ${state.exempt.size} 个")
                if (apps?.cached == true) add("缓存数据")
            },
        )
    }
}

/**
 * One app: launcher icon, name, caption, relative wake-lock time bar and the block switch.
 *
 * Blocked rows switch to the tertiary container so a glance down the list shows what is held back;
 * protected rows keep a lock glyph and a disabled switch instead of letting the tap fail.
 */
/** One whitelist entry: the only thing to do here is take it off the list. */
@Composable
private fun ExemptRow(
    packageName: String,
    identity: AppIdentity?,
    shape: Shape,
    pending: Boolean,
    onRemove: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val scheme = MaterialTheme.colorScheme
    ConnectedItem(
        shape = shape,
        modifier = modifier,
        container = scheme.secondaryContainer,
    ) {
        AppIcon(icon = identity?.icon, size = 44.dp)
        Column(modifier = Modifier.weight(1f)) {
            Text(
                identity?.label ?: packageName,
                style = MaterialTheme.typography.titleMedium,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
            Text(
                packageName,
                style = MaterialTheme.typography.labelSmall,
                color = scheme.onSecondaryContainer.copy(alpha = 0.8f),
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
        }
        if (pending) {
            LoadingIndicator(modifier = Modifier.size(28.dp))
        } else {
            IconButton(onClick = onRemove) {
                Icon(
                    Icons.Rounded.LockOpen,
                    contentDescription = "取消保护 $packageName",
                    tint = scheme.onSecondaryContainer,
                )
            }
        }
    }
}

@Composable
private fun AppWakeRow(
    row: WakelockAppRow,
    identity: AppIdentity?,
    relative: Float?,
    pending: Boolean,
    shape: Shape,
    onToggle: (Boolean) -> Unit,
    modifier: Modifier = Modifier,
) {
    val scheme = MaterialTheme.colorScheme
    ConnectedItem(
        shape = shape,
        modifier = modifier,
        container = if (row.blocked) scheme.tertiaryContainer else scheme.surfaceContainerHigh,
    ) {
        AppIcon(icon = identity?.icon, size = 44.dp, highlight = row.blocked)
        Column(
            modifier = Modifier.weight(1f),
            verticalArrangement = Arrangement.spacedBy(4.dp),
        ) {
            Row(
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                Text(
                    identity?.label ?: row.title,
                    modifier = Modifier.weight(1f, fill = false),
                    style = MaterialTheme.typography.titleMedium,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
                if (row.protected) {
                    Icon(
                        Icons.Rounded.Lock,
                        contentDescription = "系统关键应用",
                        tint = scheme.onSurfaceVariant,
                        modifier = Modifier.size(16.dp),
                    )
                }
                if (row.exempt) StatusPill("已保护", StatusTone.Brand)
                if (row.blocked) StatusPill("已拦截", StatusTone.Warning)
            }
            Text(
                when {
                    row.exempt -> "${row.packageName ?: row.title} · 已加入保护名单，不会被拦截"
                    row.protected -> "${row.packageName ?: row.title} · 系统关键应用，不可拦截"
                    row.caption != null -> row.caption
                    else -> row.packageName ?: ""
                },
                style = MaterialTheme.typography.labelSmall,
                color = if (row.blocked) {
                    scheme.onTertiaryContainer.copy(alpha = 0.8f)
                } else {
                    scheme.onSurfaceVariant
                },
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
            if (relative != null) {
                WakelockShareBar(
                    fraction = relative,
                    color = if (row.blocked) scheme.tertiary else scheme.primary,
                    height = 4.dp,
                    modifier = Modifier.padding(top = 2.dp),
                )
            }
        }
        Column(
            horizontalAlignment = Alignment.End,
            verticalArrangement = Arrangement.spacedBy(2.dp),
        ) {
            Text(
                row.totalMs?.let(::formatAwakeMs) ?: "—",
                style = MaterialTheme.typography.titleSmallEmphasized,
                color = if (row.blocked) scheme.onTertiaryContainer else scheme.primary,
            )
            if (pending) {
                // Same footprint as the switch so the row does not jump while the write is in flight.
                Box(Modifier.size(width = 52.dp, height = 32.dp), contentAlignment = Alignment.Center) {
                    LoadingIndicator(modifier = Modifier.size(28.dp))
                }
            } else {
                BlockSwitch(
                    blocked = row.blocked,
                    enabled = row.packageName != null && !row.protected && !row.exempt,
                    onToggle = onToggle,
                )
            }
        }
    }
}

/** Switch whose thumb says what the state means: a block glyph when on, a check when allowed. */
@Composable
internal fun BlockSwitch(
    blocked: Boolean,
    enabled: Boolean,
    onToggle: (Boolean) -> Unit,
    modifier: Modifier = Modifier,
) {
    Switch(
        checked = blocked,
        onCheckedChange = onToggle,
        modifier = modifier,
        enabled = enabled,
        thumbContent = {
            Icon(
                imageVector = if (blocked) Icons.Rounded.Block else Icons.Rounded.Check,
                contentDescription = null,
                modifier = Modifier.size(SwitchDefaults.IconSize),
            )
        },
    )
}
