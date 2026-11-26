@file:OptIn(ExperimentalMaterial3Api::class, ExperimentalMaterial3ExpressiveApi::class)

package io.sysflow.feature.process

import androidx.compose.animation.AnimatedVisibility
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.asPaddingValues
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.navigationBars
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.AccountTree
import androidx.compose.material.icons.rounded.Bedtime
import androidx.compose.material.icons.rounded.Bolt
import androidx.compose.material.icons.rounded.DeleteForever
import androidx.compose.material.icons.rounded.ExpandLess
import androidx.compose.material.icons.rounded.ExpandMore
import androidx.compose.material.icons.rounded.Info
import androidx.compose.material.icons.rounded.Lock
import androidx.compose.material.icons.rounded.PlayArrow
import androidx.compose.material.icons.rounded.Speed
import androidx.compose.material.icons.rounded.Tune
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.ExperimentalMaterial3ExpressiveApi
import androidx.compose.material3.FilledTonalButton
import androidx.compose.material3.FilledTonalIconButton
import androidx.compose.material3.Icon
import androidx.compose.material3.LoadingIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Slider
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import io.sysflow.core.ui.components.AppIcon
import io.sysflow.core.ui.components.StatusPill
import io.sysflow.core.ui.components.StatusTone
import io.sysflow.core.ui.components.rememberAppIdentities
import kotlin.math.roundToInt

/**
 * Detail sheet of one process: who it is, what it costs, how it is scheduled — and what SysFlow
 * can do to it.
 *
 * Reads top to bottom as identity → cost → scheduling → actions. Every action group shows the
 * value the process has *now* as the checked segment, so a tap is always a change, never a
 * guess. All actions stay one-shot and say so: ActivityManager may reset them afterwards.
 */
@Composable
internal fun ProcessDetailSheet(
    state: ProcessViewModel.UiState,
    onDismiss: () -> Unit,
    onDismissMessage: () -> Unit,
    onStop: (String, Int?) -> Unit,
    onOom: (String, Int) -> Unit,
    onRenice: (String, Int) -> Unit,
    onSched: (String, String, Int?) -> Unit,
    onCpuset: (String, String) -> Unit,
    onFreeze: (String, Boolean) -> Unit,
) {
    val scheme = MaterialTheme.colorScheme
    val sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true)
    val bottomInset = WindowInsets.navigationBars.asPaddingValues().calculateBottomPadding()
    val detail = state.detail

    ModalBottomSheet(
        onDismissRequest = onDismiss,
        sheetState = sheetState,
        containerColor = scheme.surfaceContainerLow,
    ) {
        if (detail == null) {
            Box(
                Modifier
                    .fillMaxWidth()
                    .padding(vertical = 48.dp),
                contentAlignment = Alignment.Center,
            ) { LoadingIndicator() }
            return@ModalBottomSheet
        }

        val identities = rememberAppIdentities(listOfNotNull(detail.packageName))
        val identity = detail.packageName?.let { identities[it] }
        val title = identity?.label ?: detail.packageName ?: detail.name.ifBlank { "pid ${detail.pid}" }
        val subtitle = detail.packageName?.takeIf { it != title } ?: detail.name.takeIf { it != title }
        var scopeByPackage by rememberSaveable(detail.pid) { mutableStateOf(false) }

        Column(
            modifier = Modifier
                .fillMaxWidth()
                .imePadding()
                .verticalScroll(rememberScrollState())
                .padding(start = 16.dp, end = 16.dp, bottom = 24.dp + bottomInset),
            verticalArrangement = Arrangement.spacedBy(16.dp),
        ) {
            // Action feedback lives here too: the page behind the scrim would hide it.
            val message = state.error ?: state.message
            if (message != null) {
                val danger = state.error != null
                NoteTile(
                    text = message,
                    icon = Icons.Rounded.Info,
                    container = if (danger) scheme.errorContainer else scheme.secondaryContainer,
                    content = if (danger) scheme.onErrorContainer else scheme.onSecondaryContainer,
                    onDismiss = onDismissMessage,
                )
            }

            DetailHeader(
                detail = detail,
                title = title,
                subtitle = subtitle,
                icon = identity?.icon,
            )

            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                StatTile(
                    label = "CPU",
                    value = formatProcessCpu(detail.cpuPct),
                    container = scheme.primaryContainer,
                    content = scheme.onPrimaryContainer,
                    modifier = Modifier.weight(1f),
                )
                StatTile(
                    label = "RSS",
                    value = formatProcessKb(detail.rssKb),
                    container = scheme.secondaryContainer,
                    content = scheme.onSecondaryContainer,
                    modifier = Modifier.weight(1f),
                )
                StatTile(
                    label = "PSS",
                    value = formatProcessKb(detail.pssKb),
                    container = scheme.tertiaryContainer,
                    content = scheme.onTertiaryContainer,
                    modifier = Modifier.weight(1f),
                )
            }

            SectionHeader(title = "进程信息")
            // Short facts go in a two-column grid (half the height of one-row-per-field); the long
            // strings (cgroup path, command line, shared packages) sit in a collapsed block so they
            // no longer squeeze their label into a sliver or push the actions off screen.
            InfoGrid(
                items = buildList {
                    add("PID / PPID" to "${detail.pid} / ${detail.ppid}")
                    add("UID / 用户" to "${detail.uid} / user ${detail.user}")
                    add("线程" to detail.threads.toString())
                    add("状态" to detail.state.ifBlank { "—" })
                    add(
                        "冻结状态" to when (detail.frozen) {
                            true -> "已冻结"
                            false -> "运行中"
                            null -> "未知"
                        },
                    )
                    add("SwapPss" to formatProcessKb(detail.swapPssKb))
                    add("oom_score_adj" to (detail.oomScoreAdj?.toString() ?: "—"))
                    add("nice" to detail.nice.toString())
                    add(
                        "调度策略" to "${schedPolicyLabel(detail.policy, detail.policyName)} (${detail.policy}) · RT ${detail.rtPriority}",
                    )
                    add("cpuset" to (detail.cpuset ?: "—"))
                },
            )
            val longRows = buildList {
                add("cgroup" to (detail.cgroup ?: "—"))
                if (detail.cmdline.isNotEmpty()) add("命令行" to detail.cmdline.joinToString(" "))
                if (detail.packages.size > 1) add("共享包名" to detail.packages.joinToString("\n"))
            }
            ExpandableInfo(
                title = "更多信息",
                summary = longRows.joinToString(" · ") { it.first },
                rows = longRows,
                resetKey = detail.pid,
            )

            DetailActions(
                detail = detail,
                scopeByPackage = scopeByPackage,
                onScopeChange = { scopeByPackage = it },
                features = state.features,
                experimental = state.experimental,
                groups = state.list?.cpusetGroups.orEmpty(),
                runningLabel = state.actionRunning,
                onStop = onStop,
                onOom = onOom,
                onRenice = onRenice,
                onSched = onSched,
                onCpuset = onCpuset,
                onFreeze = onFreeze,
            )
        }
    }
}

@Composable
private fun DetailHeader(
    detail: ProcessDetail,
    title: String,
    subtitle: String?,
    icon: ImageBitmap?,
) {
    Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
        Row(
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(16.dp),
        ) {
            AppIcon(icon = icon, size = 64.dp, highlight = detail.foreground)
            Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(2.dp)) {
                Text(
                    title,
                    style = MaterialTheme.typography.headlineSmallEmphasized,
                    maxLines = 2,
                    overflow = TextOverflow.Ellipsis,
                )
                if (subtitle != null) {
                    Text(
                        subtitle,
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                    )
                }
            }
        }
        if (detail.foreground || detail.system || detail.protected || detail.frozen == true) {
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                if (detail.foreground) StatusPill("前台", StatusTone.Brand)
                if (detail.frozen == true) StatusPill("已冻结", StatusTone.Warning)
                if (detail.system) StatusPill("系统", StatusTone.Info)
                if (detail.protected) StatusPill("受保护", StatusTone.Error)
            }
        }
    }
}

// ---- info ------------------------------------------------------------------------------------

/**
 * Two-column grid of short label/value tiles. An odd last tile and the full-width ones (long
 * values) take the whole row, so nothing is squeezed to make the grid even.
 */
@Composable
private fun InfoGrid(items: List<Pair<String, String>>, modifier: Modifier = Modifier) {
    Column(modifier.fillMaxWidth(), verticalArrangement = Arrangement.spacedBy(4.dp)) {
        // A value this long would wrap badly in a half-width tile.
        val (wide, narrow) = items.partition { it.second.length > 18 }
        narrow.chunked(2).forEach { row ->
            Row(horizontalArrangement = Arrangement.spacedBy(4.dp)) {
                row.forEach { (label, value) -> InfoTile(label, value, Modifier.weight(1f)) }
                if (row.size == 1) Spacer(Modifier.weight(1f))
            }
        }
        wide.forEach { (label, value) -> InfoTile(label, value, Modifier.fillMaxWidth()) }
    }
}

@Composable
private fun InfoTile(label: String, value: String, modifier: Modifier = Modifier) {
    Surface(
        modifier = modifier,
        shape = MaterialTheme.shapes.medium,
        color = MaterialTheme.colorScheme.surfaceContainerHigh,
    ) {
        Column(
            modifier = Modifier.padding(horizontal = 16.dp, vertical = 10.dp),
            verticalArrangement = Arrangement.spacedBy(2.dp),
        ) {
            Text(
                label,
                style = MaterialTheme.typography.labelSmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                maxLines = 1,
            )
            Text(
                value,
                style = MaterialTheme.typography.titleSmallEmphasized,
                color = MaterialTheme.colorScheme.onSurface,
            )
        }
    }
}

/**
 * Collapsed-by-default block for long values. Each row stacks label over value (monospace, wraps
 * at any character, selectable so a command line can be copied) instead of sharing one line.
 */
@Composable
private fun ExpandableInfo(
    title: String,
    summary: String,
    rows: List<Pair<String, String>>,
    resetKey: Any,
    modifier: Modifier = Modifier,
) {
    var expanded by rememberSaveable(resetKey) { mutableStateOf(false) }
    val scheme = MaterialTheme.colorScheme
    Surface(
        modifier = modifier.fillMaxWidth(),
        shape = MaterialTheme.shapes.large,
        color = scheme.surfaceContainerHigh,
    ) {
        Column {
            Row(
                modifier = Modifier
                    .fillMaxWidth()
                    .clickable { expanded = !expanded }
                    .padding(horizontal = 16.dp, vertical = 12.dp),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(12.dp),
            ) {
                Column(Modifier.weight(1f)) {
                    Text(title, style = MaterialTheme.typography.titleSmallEmphasized)
                    Text(
                        summary,
                        style = MaterialTheme.typography.bodySmall,
                        color = scheme.onSurfaceVariant,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                    )
                }
                Icon(
                    if (expanded) Icons.Rounded.ExpandLess else Icons.Rounded.ExpandMore,
                    contentDescription = if (expanded) "收起" else "展开",
                    tint = scheme.onSurfaceVariant,
                )
            }
            AnimatedVisibility(visible = expanded) {
                SelectionContainer {
                    Column(
                        modifier = Modifier
                            .fillMaxWidth()
                            .padding(start = 16.dp, end = 16.dp, bottom = 14.dp),
                        verticalArrangement = Arrangement.spacedBy(10.dp),
                    ) {
                        rows.forEach { (label, value) ->
                            Column(verticalArrangement = Arrangement.spacedBy(2.dp)) {
                                Text(
                                    label,
                                    style = MaterialTheme.typography.labelSmall,
                                    color = scheme.onSurfaceVariant,
                                )
                                Text(
                                    value,
                                    style = MaterialTheme.typography.bodySmall,
                                    fontFamily = FontFamily.Monospace,
                                    color = scheme.onSurface,
                                )
                            }
                        }
                    }
                }
            }
        }
    }
}

// ---- actions ---------------------------------------------------------------------------------

/**
 * The action area is a connected list of collapsible groups (one open at a time) instead of a stack
 * of always-open cards: with the experimental switch on there are up to five groups, and fully
 * expanded they were several screens tall. Collapsed, each group shows only its current value.
 */
@Composable
private fun DetailActions(
    detail: ProcessDetail,
    scopeByPackage: Boolean,
    onScopeChange: (Boolean) -> Unit,
    features: Set<String>,
    experimental: Boolean,
    groups: List<String>,
    runningLabel: String?,
    onStop: (String, Int?) -> Unit,
    onOom: (String, Int) -> Unit,
    onRenice: (String, Int) -> Unit,
    onSched: (String, String, Int?) -> Unit,
    onCpuset: (String, String) -> Unit,
    onFreeze: (String, Boolean) -> Unit,
) {
    val scheme = MaterialTheme.colorScheme
    val busy = runningLabel != null
    val packageTarget = detail.packageName
    val target = if (scopeByPackage && packageTarget != null) packageTarget else detail.pid.toString()
    var openGroup by rememberSaveable(detail.pid) { mutableStateOf<String?>(null) }

    val hasNice = "renice" in features
    val hasOom = experimental && "oom_score_adj" in features
    val hasSched = experimental && "sched" in features
    val hasCpuset = experimental && "cpuset" in features && groups.isNotEmpty()
    val hasFreeze = experimental && "freeze" in features
    val keys = buildList {
        if (hasNice) add("nice")
        if (hasOom) add("oom")
        if (hasSched) add("sched")
        if (hasCpuset) add("cpuset")
        if (hasFreeze) add("freeze")
    }

    Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
        SectionHeader(title = "操作", supporting = "均为一次性操作，系统可能随后改回")

        if (detail.protected) {
            NoteTile(
                text = "系统进程受保护：${detail.protectedReason ?: "系统进程"}，所有操作已禁用。",
                icon = Icons.Rounded.Lock,
                container = scheme.errorContainer,
                content = scheme.onErrorContainer,
            )
            return@Column
        }

        if (packageTarget != null) {
            SectionSurface {
                Column(
                    modifier = Modifier.padding(16.dp),
                    verticalArrangement = Arrangement.spacedBy(10.dp),
                ) {
                    Text("作用范围", style = MaterialTheme.typography.titleSmallEmphasized)
                    ConnectedChoiceRow(
                        options = listOf(false, true),
                        selected = scopeByPackage,
                        label = { if (it) "整个应用" else "仅此进程 (PID ${detail.pid})" },
                        onSelect = onScopeChange,
                        enabled = !busy,
                        compact = true,
                    )
                    if (scopeByPackage) {
                        Text(
                            "调整将同时作用于 $packageTarget 下的全部运行中进程。",
                            style = MaterialTheme.typography.bodySmall,
                            color = scheme.onSurfaceVariant,
                        )
                    }
                }
            }
        }

        if (busy) BusyTile(label = runningLabel.orEmpty())

        // Connected list: the groups touch (4 dp gaps) and only the outer corners stay large.
        Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
            if (hasNice) {
                var niceText by rememberSaveable(detail.pid, detail.nice) {
                    mutableStateOf(detail.nice.toString())
                }
                ExpandableActionCard(
                    expanded = openGroup == "nice",
                    onToggle = { openGroup = if (openGroup == "nice") null else "nice" },
                    icon = Icons.Rounded.Speed,
                    title = "调度优先级 nice",
                    supporting = "当前 ${detail.nice} · 越小越优先",
                    position = keys.indexOf("nice"),
                    count = keys.size,
                ) {
                    NumberAdjuster(
                        presets = listOf(-10, -5, 0, 5, 10),
                        current = detail.nice,
                        text = niceText,
                        onTextChange = { niceText = it },
                        range = -20..19,
                        maxLen = 3,
                        busy = busy,
                        applyEvenIfSame = scopeByPackage,
                        onApply = { onRenice(target, it) },
                    )
                }
            }

            if (hasOom) {
                var oomText by rememberSaveable(detail.pid, detail.oomScoreAdj) {
                    mutableStateOf((detail.oomScoreAdj ?: 0).toString())
                }
                ExpandableActionCard(
                    expanded = openGroup == "oom",
                    onToggle = { openGroup = if (openGroup == "oom") null else "oom" },
                    icon = Icons.Rounded.Tune,
                    title = "oom_score_adj",
                    supporting = "高风险 · 当前 ${detail.oomScoreAdj ?: "—"}",
                    position = keys.indexOf("oom"),
                    count = keys.size,
                    badgeContainer = scheme.errorContainer,
                    badgeContent = scheme.onErrorContainer,
                ) {
                    NumberAdjuster(
                        presets = listOf(-1000, 0, 100, 500),
                        current = detail.oomScoreAdj,
                        text = oomText,
                        onTextChange = { oomText = it },
                        range = -1000..1000,
                        maxLen = 5,
                        busy = busy,
                        applyEvenIfSame = scopeByPackage,
                        onApply = { onOom(target, it) },
                    )
                }
            }

            if (hasSched) {
                var selectedPolicy by rememberSaveable(detail.pid, detail.policy) {
                    mutableIntStateOf(if (detail.policy in listOf(0, 1, 2, 3, 5)) detail.policy else 0)
                }
                var rtPrioText by rememberSaveable(detail.pid, detail.rtPriority) {
                    mutableStateOf((if (detail.rtPriority in 1..99) detail.rtPriority else 10).toString())
                }
                val isRt = isRealtimeSchedPolicy(selectedPolicy)
                val parsedRtPrio = rtPrioText.toIntOrNull()
                val rtPrioValid = !isRt || (parsedRtPrio != null && parsedRtPrio in 1..99)
                val schedChanged = scopeByPackage ||
                    selectedPolicy != detail.policy ||
                    (isRt && parsedRtPrio != detail.rtPriority)

                ExpandableActionCard(
                    expanded = openGroup == "sched",
                    onToggle = { openGroup = if (openGroup == "sched") null else "sched" },
                    icon = Icons.Rounded.Bolt,
                    title = "实时调度策略",
                    supporting = "高风险 · 当前 ${schedPolicyLabel(detail.policy, detail.policyName)} (${detail.rtPriority})",
                    position = keys.indexOf("sched"),
                    count = keys.size,
                    badgeContainer = scheme.errorContainer,
                    badgeContent = scheme.onErrorContainer,
                ) {
                    Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
                        ConnectedChoiceRow(
                            options = listOf(0, 3, 5),
                            selected = selectedPolicy,
                            label = { schedOptionLabel(it) },
                            onSelect = { selectedPolicy = it },
                            enabled = !busy,
                            compact = true,
                        )
                        ConnectedChoiceRow(
                            options = listOf(1, 2),
                            selected = selectedPolicy,
                            label = { schedOptionLabel(it) },
                            onSelect = { selectedPolicy = it },
                            enabled = !busy,
                            compact = true,
                            slots = 3,
                        )
                    }
                    AnimatedVisibility(visible = isRt) {
                        Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                            ConnectedChoiceRow(
                                options = listOf(1, 10, 50, 99),
                                selected = parsedRtPrio,
                                label = { it.toString() },
                                onSelect = { rtPrioText = it.toString() },
                                enabled = !busy,
                                compact = true,
                            )
                            Slider(
                                value = (parsedRtPrio ?: 10).coerceIn(1, 99).toFloat(),
                                onValueChange = { rtPrioText = it.roundToInt().toString() },
                                valueRange = 1f..99f,
                                enabled = !busy,
                            )
                            OutlinedTextField(
                                value = rtPrioText,
                                onValueChange = { rtPrioText = it.filter(Char::isDigit).take(2) },
                                label = { Text("实时优先级 (1..99)") },
                                isError = rtPrioText.isNotEmpty() && !rtPrioValid,
                                singleLine = true,
                                enabled = !busy,
                                keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Number),
                                modifier = Modifier.fillMaxWidth(),
                            )
                        }
                    }
                    Button(
                        onClick = {
                            onSched(target, schedPolicyArg(selectedPolicy), if (isRt) parsedRtPrio else 0)
                        },
                        enabled = !busy && rtPrioValid && schedChanged,
                        modifier = Modifier.fillMaxWidth(),
                        shapes = ButtonDefaults.shapes(),
                    ) {
                        Text("应用调度策略", maxLines = 1)
                    }
                }
            }

            if (hasCpuset) {
                ExpandableActionCard(
                    expanded = openGroup == "cpuset",
                    onToggle = { openGroup = if (openGroup == "cpuset") null else "cpuset" },
                    icon = Icons.Rounded.AccountTree,
                    title = "cpuset 绑核",
                    supporting = "当前 ${detail.cpuset ?: "—"}",
                    position = keys.indexOf("cpuset"),
                    count = keys.size,
                    badgeContainer = scheme.tertiaryContainer,
                    badgeContent = scheme.onTertiaryContainer,
                ) {
                    Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
                        groups.chunked(2).forEach { row ->
                            ConnectedChoiceRow(
                                options = row,
                                selected = detail.cpuset,
                                label = { it },
                                onSelect = { group ->
                                    if (scopeByPackage || group != detail.cpuset) onCpuset(target, group)
                                },
                                enabled = !busy,
                                compact = true,
                                slots = 2,
                            )
                        }
                    }
                }
            }

            if (hasFreeze) {
                val freezeStatus = when (detail.frozen) {
                    true -> "已冻结 · 系统可能随时解冻"
                    false -> "运行中 · 冻结后应用停止运行"
                    null -> "状态未知 · 系统可能随时解冻"
                }
                ExpandableActionCard(
                    expanded = openGroup == "freeze",
                    onToggle = { openGroup = if (openGroup == "freeze") null else "freeze" },
                    icon = Icons.Rounded.Bedtime,
                    title = "冻结",
                    supporting = "高风险 · $freezeStatus",
                    position = keys.indexOf("freeze"),
                    count = keys.size,
                    badgeContainer = scheme.errorContainer,
                    badgeContent = scheme.onErrorContainer,
                ) {
                    Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                        Button(
                            onClick = { onFreeze(target, true) },
                            enabled = !busy && (scopeByPackage || detail.frozen != true),
                            modifier = Modifier.weight(1f),
                            shapes = ButtonDefaults.shapes(),
                        ) {
                            Icon(
                                Icons.Rounded.Bedtime,
                                contentDescription = null,
                                modifier = Modifier.size(ButtonDefaults.IconSize),
                            )
                            Spacer(Modifier.size(ButtonDefaults.IconSpacing))
                            Text("冻结", maxLines = 1)
                        }
                        FilledTonalButton(
                            onClick = { onFreeze(target, false) },
                            enabled = !busy && (scopeByPackage || detail.frozen != false),
                            modifier = Modifier.weight(1f),
                            shapes = ButtonDefaults.shapes(),
                        ) {
                            Icon(
                                Icons.Rounded.PlayArrow,
                                contentDescription = null,
                                modifier = Modifier.size(ButtonDefaults.IconSize),
                            )
                            Spacer(Modifier.size(ButtonDefaults.IconSpacing))
                            Text("解冻", maxLines = 1)
                        }
                    }
                }
            }
        }

        // Destructive and rarely needed: kept apart from the tuning list, at the very end.
        if ("stop" in features && packageTarget != null) {
            Button(
                onClick = { onStop(packageTarget, detail.user) },
                enabled = !busy,
                modifier = Modifier.fillMaxWidth(),
                shapes = ButtonDefaults.shapes(),
                colors = ButtonDefaults.buttonColors(
                    containerColor = scheme.errorContainer,
                    contentColor = scheme.onErrorContainer,
                ),
            ) {
                Icon(
                    Icons.Rounded.DeleteForever,
                    contentDescription = null,
                    modifier = Modifier.size(ButtonDefaults.IconSize),
                )
                Spacer(Modifier.size(ButtonDefaults.IconSpacing))
                Text("强制停止应用", maxLines = 1)
            }
        }

        if (features.isEmpty()) {
            NoteTile(text = "模块未报告任何动作能力，本机只能查看。")
        }
    }
}

/**
 * Preset segments + slider + custom field for one bounded integer. Shared by nice and
 * oom_score_adj, which used to carry two copies of this block.
 */
@Composable
private fun NumberAdjuster(
    presets: List<Int>,
    current: Int?,
    text: String,
    onTextChange: (String) -> Unit,
    range: IntRange,
    maxLen: Int,
    busy: Boolean,
    applyEvenIfSame: Boolean,
    onApply: (Int) -> Unit,
) {
    val parsed = text.toIntOrNull()
    val valid = parsed != null && parsed in range
    ConnectedChoiceRow(
        options = presets,
        selected = current,
        label = { it.toString() },
        onSelect = { value -> if (applyEvenIfSame || value != current) onApply(value) },
        enabled = !busy,
        compact = true,
    )
    Slider(
        value = (parsed ?: current ?: 0).coerceIn(range.first, range.last).toFloat(),
        onValueChange = { onTextChange(it.roundToInt().toString()) },
        valueRange = range.first.toFloat()..range.last.toFloat(),
        enabled = !busy,
    )
    Row(
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        FilledTonalIconButton(
            onClick = { onTextChange(toggleNumberSign(text)) },
            enabled = !busy,
        ) {
            Text("±", style = MaterialTheme.typography.titleMediumEmphasized)
        }
        OutlinedTextField(
            value = text,
            onValueChange = { onTextChange(filterSignedDigits(it, maxLen)) },
            label = { Text("自定义 (${range.first}..${range.last})") },
            isError = text.isNotEmpty() && !valid,
            singleLine = true,
            enabled = !busy,
            keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Ascii),
            modifier = Modifier.weight(1f),
        )
        FilledTonalButton(
            onClick = { parsed?.let(onApply) },
            enabled = !busy && valid && (applyEvenIfSame || parsed != current),
            shapes = ButtonDefaults.shapes(),
        ) {
            Text("应用", maxLines = 1)
        }
    }
}

/** The action in flight; every control is disabled until it finishes. */
@Composable
private fun BusyTile(label: String) {
    Surface(
        modifier = Modifier.fillMaxWidth(),
        shape = MaterialTheme.shapes.large,
        color = MaterialTheme.colorScheme.secondaryContainer,
        contentColor = MaterialTheme.colorScheme.onSecondaryContainer,
    ) {
        Row(
            modifier = Modifier.padding(horizontal = 16.dp, vertical = 10.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            LoadingIndicator(modifier = Modifier.size(32.dp))
            Text(
                "正在${label}…",
                style = MaterialTheme.typography.bodyMedium,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
        }
    }
}

private fun schedOptionLabel(policy: Int): String = when (policy) {
    0 -> "默认 OTHER"
    1 -> "实时 FIFO"
    2 -> "实时 RR"
    3 -> "批处理 BATCH"
    5 -> "空闲 IDLE"
    else -> policy.toString()
}

internal fun filterSignedDigits(raw: String, maxLen: Int): String {
    val builder = StringBuilder()
    raw.trim().forEachIndexed { index, ch ->
        if ((ch == '-' && index == 0) || ch.isDigit()) {
            builder.append(ch)
        }
    }
    return builder.toString().take(maxLen)
}

internal fun toggleNumberSign(raw: String): String {
    val trimmed = raw.trim()
    if (trimmed.isEmpty() || trimmed == "0") return "-"
    if (trimmed == "-") return ""
    return if (trimmed.startsWith("-")) trimmed.drop(1) else "-$trimmed"
}
