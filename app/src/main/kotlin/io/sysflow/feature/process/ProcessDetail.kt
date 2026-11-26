@file:OptIn(ExperimentalMaterial3Api::class, ExperimentalMaterial3ExpressiveApi::class)

package io.sysflow.feature.process

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
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.AccountTree
import androidx.compose.material.icons.rounded.Bedtime
import androidx.compose.material.icons.rounded.Bolt
import androidx.compose.material.icons.rounded.DeleteForever
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
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import io.sysflow.core.ui.components.AppIcon
import io.sysflow.core.ui.components.SegmentedValueList
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
            SegmentedValueList(
                rows = buildList {
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
                    add("调度优先级 nice" to detail.nice.toString())
                    add(
                        "调度策略" to "${schedPolicyLabel(detail.policy, detail.policyName)} (${detail.policy}) · 实时优先级 ${detail.rtPriority}",
                    )
                    add("cgroup" to (detail.cgroup ?: "—"))
                    add("cpuset" to (detail.cpuset ?: "—"))
                    if (detail.cmdline.isNotEmpty()) add("命令行" to detail.cmdline.joinToString(" "))
                    if (detail.packages.size > 1) add("共享包名" to detail.packages.joinToString("\n"))
                },
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

// ---- actions ---------------------------------------------------------------------------------

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

        if ("renice" in features) {
            var niceText by rememberSaveable(detail.pid, detail.nice) {
                mutableStateOf(detail.nice.toString())
            }
            val parsedNice = niceText.toIntOrNull()
            val niceValid = parsedNice != null && parsedNice in -20..19

            SectionSurface {
                Column(
                    modifier = Modifier.padding(16.dp),
                    verticalArrangement = Arrangement.spacedBy(14.dp),
                ) {
                    ActionGroupHeader(
                        icon = Icons.Rounded.Speed,
                        title = "调度优先级 nice",
                        supporting = "越小越优先，当前 ${detail.nice}",
                    )
                    ConnectedChoiceRow(
                        options = listOf(-10, -5, 0, 5, 10),
                        selected = detail.nice,
                        label = { it.toString() },
                        onSelect = { value -> if (scopeByPackage || value != detail.nice) onRenice(target, value) },
                        enabled = !busy,
                        compact = true,
                    )
                    Slider(
                        value = (parsedNice ?: detail.nice).coerceIn(-20, 19).toFloat(),
                        onValueChange = { niceText = it.roundToInt().toString() },
                        valueRange = -20f..19f,
                        enabled = !busy,
                    )
                    Row(
                        verticalAlignment = Alignment.CenterVertically,
                        horizontalArrangement = Arrangement.spacedBy(8.dp),
                    ) {
                        FilledTonalIconButton(
                            onClick = { niceText = toggleNumberSign(niceText) },
                            enabled = !busy,
                        ) {
                            Text("±", style = MaterialTheme.typography.titleMediumEmphasized)
                        }
                        OutlinedTextField(
                            value = niceText,
                            onValueChange = { niceText = filterSignedDigits(it, 3) },
                            label = { Text("自定义 (-20..19)") },
                            isError = niceText.isNotEmpty() && !niceValid,
                            singleLine = true,
                            enabled = !busy,
                            keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Ascii),
                            modifier = Modifier.weight(1f),
                        )
                        FilledTonalButton(
                            onClick = { parsedNice?.let { onRenice(target, it) } },
                            enabled = !busy && niceValid && (scopeByPackage || parsedNice != detail.nice),
                            shapes = ButtonDefaults.shapes(),
                        ) {
                            Text("应用", maxLines = 1)
                        }
                    }
                }
            }
        }

        if (experimental && "oom_score_adj" in features) {
            var oomText by rememberSaveable(detail.pid, detail.oomScoreAdj) {
                mutableStateOf((detail.oomScoreAdj ?: 0).toString())
            }
            val parsedOom = oomText.toIntOrNull()
            val oomValid = parsedOom != null && parsedOom in -1000..1000

            SectionSurface {
                Column(
                    modifier = Modifier.padding(16.dp),
                    verticalArrangement = Arrangement.spacedBy(14.dp),
                ) {
                    ActionGroupHeader(
                        icon = Icons.Rounded.Tune,
                        title = "oom_score_adj",
                        supporting = "高风险 · 当前 ${detail.oomScoreAdj ?: "—"}",
                        container = scheme.errorContainer,
                        content = scheme.onErrorContainer,
                    )
                    ConnectedChoiceRow(
                        options = listOf(-1000, 0, 100, 500),
                        selected = detail.oomScoreAdj,
                        label = { it.toString() },
                        onSelect = { value -> if (scopeByPackage || value != detail.oomScoreAdj) onOom(target, value) },
                        enabled = !busy,
                        compact = true,
                    )
                    Slider(
                        value = (parsedOom ?: detail.oomScoreAdj ?: 0).coerceIn(-1000, 1000).toFloat(),
                        onValueChange = { oomText = it.roundToInt().toString() },
                        valueRange = -1000f..1000f,
                        enabled = !busy,
                    )
                    Row(
                        verticalAlignment = Alignment.CenterVertically,
                        horizontalArrangement = Arrangement.spacedBy(8.dp),
                    ) {
                        FilledTonalIconButton(
                            onClick = { oomText = toggleNumberSign(oomText) },
                            enabled = !busy,
                        ) {
                            Text("±", style = MaterialTheme.typography.titleMediumEmphasized)
                        }
                        OutlinedTextField(
                            value = oomText,
                            onValueChange = { oomText = filterSignedDigits(it, 5) },
                            label = { Text("自定义 (-1000..1000)") },
                            isError = oomText.isNotEmpty() && !oomValid,
                            singleLine = true,
                            enabled = !busy,
                            keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Ascii),
                            modifier = Modifier.weight(1f),
                        )
                        FilledTonalButton(
                            onClick = { parsedOom?.let { onOom(target, it) } },
                            enabled = !busy && oomValid && (scopeByPackage || parsedOom != detail.oomScoreAdj),
                            shapes = ButtonDefaults.shapes(),
                        ) {
                            Text("应用", maxLines = 1)
                        }
                    }
                }
            }
        }

        if (experimental && "sched" in features) {
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

            SectionSurface {
                Column(
                    modifier = Modifier.padding(16.dp),
                    verticalArrangement = Arrangement.spacedBy(14.dp),
                ) {
                    ActionGroupHeader(
                        icon = Icons.Rounded.Bolt,
                        title = "实时调度策略",
                        supporting = "高风险 · 当前 ${schedPolicyLabel(detail.policy, detail.policyName)} (${detail.rtPriority})",
                        container = scheme.errorContainer,
                        content = scheme.onErrorContainer,
                    )
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
                    if (isRt) {
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
        }

        if (experimental && "cpuset" in features && groups.isNotEmpty()) {
            SectionSurface {
                Column(
                    modifier = Modifier.padding(16.dp),
                    verticalArrangement = Arrangement.spacedBy(14.dp),
                ) {
                    ActionGroupHeader(
                        icon = Icons.Rounded.AccountTree,
                        title = "cpuset 绑核",
                        supporting = "当前 ${detail.cpuset ?: "—"}",
                        container = scheme.tertiaryContainer,
                        content = scheme.onTertiaryContainer,
                    )
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
        }

        if (experimental && "freeze" in features) {
            val freezeStatus = when (detail.frozen) {
                true -> "当前：已冻结 · 系统可能随时解冻"
                false -> "当前：运行中 · 冻结后应用停止运行"
                null -> "状态未知 · 冻结后应用不运行，系统可能随时解冻"
            }
            SectionSurface {
                Column(
                    modifier = Modifier.padding(16.dp),
                    verticalArrangement = Arrangement.spacedBy(14.dp),
                ) {
                    ActionGroupHeader(
                        icon = Icons.Rounded.Bedtime,
                        title = "冻结",
                        supporting = freezeStatus,
                        container = scheme.errorContainer,
                        content = scheme.onErrorContainer,
                    )
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

        if (features.isEmpty()) {
            NoteTile(text = "模块未报告任何动作能力，本机只能查看。")
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
