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
import androidx.compose.foundation.layout.navigationBars
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.AccountTree
import androidx.compose.material.icons.rounded.Bedtime
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
import androidx.compose.material3.Icon
import androidx.compose.material3.LoadingIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import io.sysflow.core.ui.components.AppIcon
import io.sysflow.core.ui.components.SegmentedValueList
import io.sysflow.core.ui.components.StatusPill
import io.sysflow.core.ui.components.StatusTone
import io.sysflow.core.ui.components.rememberAppIdentities

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
    onStop: (String) -> Unit,
    onOom: (String, Int) -> Unit,
    onRenice: (String, Int) -> Unit,
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

        Column(
            modifier = Modifier
                .fillMaxWidth()
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
                    add("UID" to detail.uid.toString())
                    add("线程" to detail.threads.toString())
                    add("状态" to detail.state.ifBlank { "—" })
                    add("SwapPss" to formatProcessKb(detail.swapPssKb))
                    add("oom_score_adj" to (detail.oomScoreAdj?.toString() ?: "—"))
                    add("nice / policy" to "${detail.nice} / ${detail.policy}")
                    add("cgroup" to (detail.cgroup ?: "—"))
                    add("cpuset" to (detail.cpuset ?: "—"))
                    if (detail.packages.size > 1) add("共享包名" to detail.packages.joinToString("\n"))
                },
            )

            DetailActions(
                detail = detail,
                target = detail.packageName ?: detail.pid.toString(),
                features = state.features,
                experimental = state.experimental,
                groups = state.list?.cpusetGroups.orEmpty(),
                runningLabel = state.actionRunning,
                onStop = onStop,
                onOom = onOom,
                onRenice = onRenice,
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
        if (detail.foreground || detail.system || detail.protected) {
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                if (detail.foreground) StatusPill("前台", StatusTone.Brand)
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
    target: String,
    features: Set<String>,
    experimental: Boolean,
    groups: List<String>,
    runningLabel: String?,
    onStop: (String) -> Unit,
    onOom: (String, Int) -> Unit,
    onRenice: (String, Int) -> Unit,
    onCpuset: (String, String) -> Unit,
    onFreeze: (String, Boolean) -> Unit,
) {
    val scheme = MaterialTheme.colorScheme
    val busy = runningLabel != null
    val packageTarget = detail.packageName

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

        if (busy) BusyTile(label = runningLabel.orEmpty())

        if ("stop" in features && packageTarget != null) {
            Button(
                onClick = { onStop(packageTarget) },
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
                Text("强制停止", maxLines = 1)
            }
        }

        if ("renice" in features) {
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
                        onSelect = { value -> if (value != detail.nice) onRenice(target, value) },
                        enabled = !busy,
                        compact = true,
                    )
                }
            }
        }

        if (experimental && "oom_score_adj" in features) {
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
                        onSelect = { value -> if (value != detail.oomScoreAdj) onOom(target, value) },
                        enabled = !busy,
                        compact = true,
                    )
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
                                    if (group != detail.cpuset) onCpuset(target, group)
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
            SectionSurface {
                Column(
                    modifier = Modifier.padding(16.dp),
                    verticalArrangement = Arrangement.spacedBy(14.dp),
                ) {
                    ActionGroupHeader(
                        icon = Icons.Rounded.Bedtime,
                        title = "冻结",
                        supporting = "冻结后应用不运行，系统可能随时解冻",
                        container = scheme.errorContainer,
                        content = scheme.onErrorContainer,
                    )
                    Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                        Button(
                            onClick = { onFreeze(target, true) },
                            enabled = !busy,
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
                            enabled = !busy,
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
