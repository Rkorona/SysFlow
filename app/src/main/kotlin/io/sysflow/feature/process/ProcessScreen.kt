@file:OptIn(
    ExperimentalMaterial3Api::class,
    ExperimentalMaterial3ExpressiveApi::class,
    ExperimentalLayoutApi::class,
)

package io.sysflow.feature.process

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.asPaddingValues
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.navigationBars
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.AccountTree
import androidx.compose.material.icons.rounded.Bedtime
import androidx.compose.material.icons.rounded.Close
import androidx.compose.material.icons.rounded.DeleteForever
import androidx.compose.material.icons.rounded.Info
import androidx.compose.material.icons.rounded.Lock
import androidx.compose.material.icons.rounded.PlayArrow
import androidx.compose.material.icons.rounded.Refresh
import androidx.compose.material.icons.rounded.Search
import androidx.compose.material.icons.rounded.Speed
import androidx.compose.material.icons.rounded.Timeline
import androidx.compose.material.icons.rounded.Tune
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.ExperimentalMaterial3ExpressiveApi
import androidx.compose.material3.FilterChip
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.LoadingIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.compose.LocalLifecycleOwner
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.repeatOnLifecycle
import io.sysflow.core.ui.components.AppIcon
import io.sysflow.core.ui.components.ExpressiveCard
import io.sysflow.core.ui.components.KeyValueRow
import io.sysflow.core.ui.components.SectionLabel
import io.sysflow.core.ui.components.StatusPill
import io.sysflow.core.ui.components.rememberAppIdentities
import kotlinx.coroutines.delay
import kotlinx.coroutines.isActive
import org.koin.androidx.compose.koinViewModel

/**
 * Process page: who is running, how much they use, and the few one-shot actions SysFlow offers.
 *
 * The page owns the polling loop (2 s while STARTED) and passes a 300 ms sampling window, so the
 * CPU number is fresh without the page feeling heavy. Every action is one-shot: the module never
 * persists OOM/nice/cpuset state, and ActivityManager is free to reset it afterwards — the UI says
 * so where it matters instead of pretending otherwise.
 *
 * `SysFlowApp` wraps this route in `FlexibleSubPage`, so no heading is drawn here.
 */
@Composable
fun ProcessScreen(viewModel: ProcessViewModel = koinViewModel()) {
    val state by viewModel.state.collectAsStateWithLifecycle()
    val lifecycleOwner = LocalLifecycleOwner.current
    val bottomInset = WindowInsets.navigationBars.asPaddingValues().calculateBottomPadding()
    var confirm by remember { mutableStateOf<ProcessConfirm?>(null) }

    // Only while the page is visible; a running action pauses the poll so the list does not jump
    // under the user's finger.
    LaunchedEffect(Unit) {
        lifecycleOwner.repeatOnLifecycle(Lifecycle.State.STARTED) {
            while (isActive) {
                delay(2_000)
                if (viewModel.state.value.actionRunning == null) {
                    viewModel.refresh(silent = true)
                }
            }
        }
    }

    Column(Modifier.fillMaxSize()) {
        val message = state.message
        val error = state.error
        if (message != null || error != null) {
            MessageBanner(
                text = error ?: message.orEmpty(),
                danger = error != null,
                onDismiss = viewModel::dismissMessage,
            )
        }
        val list = state.list
        Box(Modifier.weight(1f)) {
            when {
                list == null && state.loading -> Box(Modifier.fillMaxSize(), Alignment.Center) {
                    LoadingIndicator()
                }
                list == null -> Box(Modifier.fillMaxSize(), Alignment.Center) {
                    EmptyState(
                        text = state.error ?: "设备没有提供 /proc，或模块尚未加载。",
                        onRetry = { viewModel.refresh() },
                    )
                }
                else -> ProcessListContent(
                    state = state,
                    list = list,
                    bottomInset = bottomInset,
                    onRefresh = { viewModel.refresh() },
                    onQuery = viewModel::setQuery,
                    onSubmitQuery = viewModel::submitQuery,
                    onFilter = viewModel::setFilter,
                    onSort = viewModel::setSort,
                    onToggleExperimental = viewModel::toggleExperimental,
                    onOpen = viewModel::loadDetail,
                )
            }
        }
    }

    if (state.detailLoading || state.detail != null) {
        ProcessDetailSheet(
            state = state,
            onDismiss = viewModel::dismissDetail,
            onStop = { target -> confirm = stopConfirm(target) { viewModel.stop(target) } },
            onOom = { target, value -> confirm = oomConfirm(target, value) { viewModel.oom(target, value) } },
            onRenice = { target, value ->
                confirm = reniceConfirm(target, value) { viewModel.renice(target, value) }
            },
            onCpuset = { target, group ->
                confirm = cpusetConfirm(target, group) { viewModel.cpuset(target, group) }
            },
            onFreeze = { target, frozen ->
                confirm = freezeConfirm(target, frozen) { viewModel.freeze(target, frozen) }
            },
        )
    }

    confirm?.let { request ->
        AlertDialog(
            onDismissRequest = { confirm = null },
            icon = {
                Icon(
                    request.icon,
                    contentDescription = null,
                    tint = if (request.danger) {
                        MaterialTheme.colorScheme.error
                    } else {
                        MaterialTheme.colorScheme.primary
                    },
                )
            },
            title = { Text(request.title) },
            text = { Text(request.text) },
            confirmButton = {
                Button(
                    onClick = {
                        confirm = null
                        request.onConfirm()
                    },
                    colors = if (request.danger) {
                        ButtonDefaults.buttonColors(
                            containerColor = MaterialTheme.colorScheme.error,
                            contentColor = MaterialTheme.colorScheme.onError,
                        )
                    } else {
                        ButtonDefaults.buttonColors()
                    },
                ) { Text(request.confirmLabel) }
            },
            dismissButton = {
                TextButton(onClick = { confirm = null }) { Text("取消") }
            },
        )
    }
}

@Composable
private fun ProcessListContent(
    state: ProcessViewModel.UiState,
    list: ProcessList,
    bottomInset: androidx.compose.ui.unit.Dp,
    onRefresh: () -> Unit,
    onQuery: (String) -> Unit,
    onSubmitQuery: () -> Unit,
    onFilter: (String) -> Unit,
    onSort: (String) -> Unit,
    onToggleExperimental: () -> Unit,
    onOpen: (Int) -> Unit,
) {
    val packages = remember(list.entries) {
        list.entries.mapNotNull { it.packageName }.distinct()
    }
    val identities = rememberAppIdentities(packages)

    LazyColumn(
        modifier = Modifier.fillMaxSize(),
        contentPadding = PaddingValues(
            start = 16.dp,
            end = 16.dp,
            top = 8.dp,
            bottom = 24.dp + bottomInset,
        ),
        verticalArrangement = Arrangement.spacedBy(10.dp),
    ) {
        item(key = "search") {
            OutlinedTextField(
                value = state.query,
                onValueChange = onQuery,
                modifier = Modifier.fillMaxWidth(),
                singleLine = true,
                shape = MaterialTheme.shapes.extraLarge,
                leadingIcon = { Icon(Icons.Rounded.Search, contentDescription = null) },
                trailingIcon = {
                    Row {
                        if (state.refreshing) {
                            LoadingIndicator(modifier = Modifier.size(20.dp))
                        } else {
                            IconButton(onClick = onRefresh) {
                                Icon(Icons.Rounded.Refresh, contentDescription = "刷新")
                            }
                        }
                        if (state.query.isNotEmpty()) {
                            IconButton(onClick = { onQuery("") }) {
                                Icon(Icons.Rounded.Close, contentDescription = "清除")
                            }
                        }
                    }
                },
                placeholder = { Text("搜索进程、包名或 PID") },
                keyboardOptions = KeyboardOptions(imeAction = ImeAction.Search),
                keyboardActions = KeyboardActions(onSearch = { onSubmitQuery() }),
            )
        }

        item(key = "filters") {
            FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                listOf("all" to "全部", "user" to "用户", "system" to "系统").forEach { (value, label) ->
                    FilterChip(
                        selected = state.filter == value,
                        onClick = { onFilter(value) },
                        label = { Text(label) },
                    )
                }
            }
        }
        item(key = "sort") {
            FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                listOf("cpu" to "CPU", "rss" to "内存", "name" to "名称", "pid" to "PID")
                    .forEach { (value, label) ->
                        FilterChip(
                            selected = state.sort == value,
                            onClick = { onSort(value) },
                            label = { Text(label) },
                        )
                    }
            }
        }

        list.foreground.packageName?.let { packageName ->
            item(key = "foreground") {
                ForegroundCard(packageName = packageName, pid = list.foreground.pid)
            }
        }

        if (state.hasExperimental) {
            item(key = "experimental") {
                ExperimentalCard(
                    enabled = state.experimental,
                    features = state.features,
                    onToggle = onToggleExperimental,
                )
            }
        }

        if (list.entries.isEmpty()) {
            item(key = "empty") { EmptyState(text = "没有匹配的进程。", onRetry = onRefresh) }
        }
        items(list.entries, key = { "${it.pid}-${it.name}" }) { entry ->
            ProcessRow(
                entry = entry,
                icon = entry.packageName?.let { identities[it]?.icon },
                onClick = { onOpen(entry.pid) },
            )
        }

        item(key = "footer") {
            Text(
                buildString {
                    append("共 ${list.total} 个进程")
                    if (list.truncated) append("，仅显示前 ${list.returned} 个")
                    if (list.skipped > 0) append("，${list.skipped} 个进程读取失败")
                    append(" · 采样 ${list.sampleMs} ms")
                },
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                modifier = Modifier.padding(top = 4.dp, bottom = 4.dp),
            )
        }
    }
}

@Composable
private fun ForegroundCard(packageName: String, pid: Int?) {
    ExpressiveCard(containerColor = MaterialTheme.colorScheme.primaryContainer) {
        Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
            Icon(Icons.Rounded.Timeline, contentDescription = null, tint = MaterialTheme.colorScheme.onPrimaryContainer)
            Column(Modifier.weight(1f)) {
                Text(
                    "前台应用",
                    style = MaterialTheme.typography.labelMedium,
                    color = MaterialTheme.colorScheme.onPrimaryContainer,
                )
                Text(
                    packageName,
                    style = MaterialTheme.typography.titleMedium,
                    color = MaterialTheme.colorScheme.onPrimaryContainer,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
            }
            pid?.let {
                StatusPill(
                    "PID $it",
                    container = MaterialTheme.colorScheme.onPrimaryContainer,
                    content = MaterialTheme.colorScheme.primaryContainer,
                )
            }
        }
    }
}

@Composable
private fun ExperimentalCard(
    enabled: Boolean,
    features: Set<String>,
    onToggle: () -> Unit,
) {
    ExpressiveCard {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(2.dp)) {
                Text("实验功能", style = MaterialTheme.typography.titleMedium)
                Text(
                    buildString {
                        append("OOM 调节、cpuset 绑核、冻结")
                        if (features.none { it == "oom_score_adj" || it == "cpuset" || it == "freeze" }) {
                            append("（本机不可用）")
                        }
                    },
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
            Switch(checked = enabled, onCheckedChange = { onToggle() })
        }
    }
}

@Composable
private fun ProcessRow(
    entry: ProcessEntry,
    icon: androidx.compose.ui.graphics.ImageBitmap?,
    onClick: () -> Unit,
) {
    val scheme = MaterialTheme.colorScheme
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .clip(MaterialTheme.shapes.large)
            .background(if (entry.foreground) scheme.primaryContainer else scheme.surfaceContainerHigh)
            .clickable(onClick = onClick)
            .padding(horizontal = 14.dp, vertical = 12.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        AppIcon(icon = icon, size = 40.dp, highlight = entry.foreground)
        Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(2.dp)) {
            Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                Text(
                    entry.displayName,
                    style = MaterialTheme.typography.bodyLarge,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
                if (entry.foreground) StatusPill("前台")
                if (entry.protected) {
                    Icon(
                        Icons.Rounded.Lock,
                        contentDescription = entry.protectedReason ?: "受保护",
                        modifier = Modifier.size(16.dp),
                        tint = scheme.onSurfaceVariant,
                    )
                }
            }
            Text(
                processSubtitle(entry),
                style = MaterialTheme.typography.bodySmall,
                color = scheme.onSurfaceVariant,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
        }
        Column(horizontalAlignment = Alignment.End, verticalArrangement = Arrangement.spacedBy(2.dp)) {
            Text(
                formatProcessCpu(entry.cpuPct),
                style = MaterialTheme.typography.titleSmall,
                fontWeight = FontWeight.SemiBold,
            )
            Text(
                formatProcessKb(entry.rssKb),
                style = MaterialTheme.typography.bodySmall,
                color = scheme.onSurfaceVariant,
            )
        }
    }
}

@Composable
private fun ProcessDetailSheet(
    state: ProcessViewModel.UiState,
    onDismiss: () -> Unit,
    onStop: (String) -> Unit,
    onOom: (String, Int) -> Unit,
    onRenice: (String, Int) -> Unit,
    onCpuset: (String, String) -> Unit,
    onFreeze: (String, Boolean) -> Unit,
) {
    val sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true)
    ModalBottomSheet(onDismissRequest = onDismiss, sheetState = sheetState) {
        val detail = state.detail
        if (detail == null) {
            Box(
                Modifier
                    .fillMaxWidth()
                    .padding(32.dp),
                contentAlignment = Alignment.Center,
            ) { LoadingIndicator() }
            return@ModalBottomSheet
        }
        val groups = state.list?.cpusetGroups.orEmpty()

        Column(
            modifier = Modifier
                .verticalScroll(rememberScrollState())
                .padding(horizontal = 20.dp)
                .padding(bottom = 28.dp),
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(2.dp)) {
                    Text(
                        detail.packageName ?: detail.name,
                        style = MaterialTheme.typography.headlineSmall,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                    )
                    Text(
                        "PID ${detail.pid} · UID ${detail.uid} · ${detail.threads} 线程",
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
                if (detail.foreground) StatusPill("前台")
            }

            ExpressiveCard {
                KeyValueRow("CPU", formatProcessCpu(detail.cpuPct))
                KeyValueRow("RSS", formatProcessKb(detail.rssKb))
                KeyValueRow("PSS", formatProcessKb(detail.pssKb))
                KeyValueRow("SwapPss", formatProcessKb(detail.swapPssKb))
                KeyValueRow("oom_score_adj", detail.oomScoreAdj?.toString() ?: "—")
                KeyValueRow("nice / policy", "${detail.nice} / ${detail.policy}")
                KeyValueRow("cgroup", detail.cgroup ?: "—")
                KeyValueRow("cpuset", detail.cpuset ?: "—")
                KeyValueRow("状态", detail.state.ifBlank { "—" })
            }

            if (detail.protected) {
                StatusPill(
                    "受保护：${detail.protectedReason ?: "系统进程"}",
                    container = MaterialTheme.colorScheme.errorContainer,
                    content = MaterialTheme.colorScheme.onErrorContainer,
                )
            }

            val target = detail.packageName ?: detail.pid.toString()
            DetailActions(
                detail = detail,
                target = target,
                features = state.features,
                experimental = state.experimental,
                groups = groups,
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
private fun DetailActions(
    detail: ProcessDetail,
    target: String,
    features: Set<String>,
    experimental: Boolean,
    groups: List<String>,
    onStop: (String) -> Unit,
    onOom: (String, Int) -> Unit,
    onRenice: (String, Int) -> Unit,
    onCpuset: (String, String) -> Unit,
    onFreeze: (String, Boolean) -> Unit,
) {
    val disabled = detail.protected
    val packageTarget = detail.packageName

    SectionLabel("操作")
    if (disabled) {
        Text(
            "系统进程受保护，所有操作已禁用。",
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        return
    }

    if ("stop" in features && packageTarget != null) {
        Button(
            onClick = { onStop(packageTarget) },
            enabled = true,
            colors = ButtonDefaults.buttonColors(
                containerColor = MaterialTheme.colorScheme.errorContainer,
                contentColor = MaterialTheme.colorScheme.onErrorContainer,
            ),
        ) {
            Icon(Icons.Rounded.DeleteForever, contentDescription = null, modifier = Modifier.size(18.dp))
            Text("  强制停止 $packageTarget")
        }
    }

    if ("renice" in features) {
        Text("nice（一次性，AMS 可能重置）", style = MaterialTheme.typography.labelMedium)
        ChipValues(listOf(-10, -5, 0, 5, 10)) { value ->
            onRenice(target, value)
        }
    }

    if (experimental && "oom_score_adj" in features) {
        Text("oom_score_adj（高风险，一次性）", style = MaterialTheme.typography.labelMedium)
        ChipValues(listOf(-1000, 0, 100, 500)) { value ->
            onOom(target, value)
        }
    }

    if (experimental && "cpuset" in features && groups.isNotEmpty()) {
        Text("cpuset 绑核（一次性）", style = MaterialTheme.typography.labelMedium)
        FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            groups.forEach { group ->
                FilterChip(
                    selected = detail.cpuset == group,
                    onClick = { onCpuset(target, group) },
                    label = { Text(group) },
                )
            }
        }
    }

    if (experimental && "freeze" in features) {
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            Button(onClick = { onFreeze(target, true) }) {
                Icon(Icons.Rounded.Bedtime, contentDescription = null, modifier = Modifier.size(18.dp))
                Text("  冻结")
            }
            TextButton(onClick = { onFreeze(target, false) }) {
                Icon(Icons.Rounded.PlayArrow, contentDescription = null, modifier = Modifier.size(18.dp))
                Text("  解冻")
            }
        }
    }

    if (features.isEmpty()) {
        Text(
            "模块未报告任何动作能力，本机只能查看。",
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
    }
}

@Composable
private fun ChipValues(values: List<Int>, onPick: (Int) -> Unit) {
    FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
        values.forEach { value ->
            FilterChip(selected = false, onClick = { onPick(value) }, label = { Text(value.toString()) })
        }
    }
}

@Composable
private fun MessageBanner(text: String, danger: Boolean, onDismiss: () -> Unit) {
    val scheme = MaterialTheme.colorScheme
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .padding(horizontal = 16.dp, vertical = 8.dp)
            .clip(MaterialTheme.shapes.large)
            .background(if (danger) scheme.errorContainer else scheme.secondaryContainer)
            .padding(start = 14.dp, end = 4.dp, top = 8.dp, bottom = 8.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        Icon(
            if (danger) Icons.Rounded.Info else Icons.Rounded.Speed,
            contentDescription = null,
            tint = if (danger) scheme.onErrorContainer else scheme.onSecondaryContainer,
        )
        Text(
            text,
            modifier = Modifier.weight(1f),
            style = MaterialTheme.typography.bodySmall,
            color = if (danger) scheme.onErrorContainer else scheme.onSecondaryContainer,
        )
        IconButton(onClick = onDismiss) {
            Icon(Icons.Rounded.Close, contentDescription = "关闭")
        }
    }
}

@Composable
private fun EmptyState(text: String, onRetry: () -> Unit) {
    ExpressiveCard(modifier = Modifier.padding(16.dp)) {
        Text(text, style = MaterialTheme.typography.bodyMedium)
        TextButton(onClick = onRetry) { Text("重试") }
    }
}

// ---------------------------------------------------------------------------------------------
// Confirmation requests
// ---------------------------------------------------------------------------------------------

private data class ProcessConfirm(
    val title: String,
    val text: String,
    val confirmLabel: String,
    val icon: ImageVector,
    val danger: Boolean = false,
    val onConfirm: () -> Unit,
)

private fun stopConfirm(target: String, onConfirm: () -> Unit) = ProcessConfirm(
    title = "强制停止？",
    text = "$target 的未保存数据可能丢失；下次打开时应用会重新启动。",
    confirmLabel = "停止",
    icon = Icons.Rounded.DeleteForever,
    danger = true,
    onConfirm = onConfirm,
)

private fun oomConfirm(target: String, value: Int, onConfirm: () -> Unit) = ProcessConfirm(
    title = "设置 oom_score_adj = $value？",
    text = "$target 的优先级会立即改变；ActivityManager 可能在状态变化后重置。",
    confirmLabel = "应用",
    icon = Icons.Rounded.Tune,
    danger = true,
    onConfirm = onConfirm,
)

private fun reniceConfirm(target: String, value: Int, onConfirm: () -> Unit) = ProcessConfirm(
    title = "设置 nice = $value？",
    text = "$target 的调度优先级会立即改变；这是高风险实验设置。",
    confirmLabel = "应用",
    icon = Icons.Rounded.Speed,
    onConfirm = onConfirm,
)

private fun cpusetConfirm(target: String, group: String, onConfirm: () -> Unit) = ProcessConfirm(
    title = "移入 $group？",
    text = "$target 的进程将只使用该 cpuset 允许的核心；ActivityManager 可能随后改回。",
    confirmLabel = "移动",
    icon = Icons.Rounded.AccountTree,
    onConfirm = onConfirm,
)

private fun freezeConfirm(target: String, frozen: Boolean, onConfirm: () -> Unit) = ProcessConfirm(
    title = if (frozen) "冻结 $target？" else "解冻 $target？",
    text = if (frozen) {
        "被冻结的应用不会运行；系统可能随时解冻或重置。"
    } else {
        "应用恢复运行。"
    },
    confirmLabel = if (frozen) "冻结" else "解冻",
    icon = Icons.Rounded.Bedtime,
    danger = frozen,
    onConfirm = onConfirm,
)
