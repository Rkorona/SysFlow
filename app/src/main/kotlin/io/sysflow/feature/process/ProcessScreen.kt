@file:OptIn(ExperimentalMaterial3ExpressiveApi::class)

package io.sysflow.feature.process

import androidx.activity.compose.BackHandler
import androidx.compose.animation.AnimatedVisibility
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.asPaddingValues
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.navigationBars
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.AccountTree
import androidx.compose.material.icons.rounded.Bedtime
import androidx.compose.material.icons.rounded.Bolt
import androidx.compose.material.icons.rounded.Close
import androidx.compose.material.icons.rounded.DeleteForever
import androidx.compose.material.icons.rounded.Info
import androidx.compose.material.icons.rounded.Speed
import androidx.compose.material.icons.rounded.Timeline
import androidx.compose.material.icons.rounded.Tune
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.ExperimentalMaterial3ExpressiveApi
import androidx.compose.material3.FilledTonalButton
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.LoadingIndicator
import androidx.compose.material3.MaterialShapes
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.toShape
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
import androidx.compose.ui.unit.dp
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.compose.LocalLifecycleOwner
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.repeatOnLifecycle
import kotlinx.coroutines.delay
import kotlinx.coroutines.isActive
import org.koin.androidx.compose.koinViewModel

/**
 * Process page: who is running, how much they use, and the few one-shot actions SysFlow offers.
 *
 * This file only hosts the page — polling, the loading / empty states, the detail sheet and the
 * confirmation dialog. The list lives in `ProcessOverview.kt`, the sheet in `ProcessDetail.kt`.
 *
 * The page owns the polling loop (2 s while STARTED) and passes a 300 ms sampling window, so the
 * CPU number is fresh without the page feeling heavy. Every action is one-shot: the module never
 * persists OOM/nice/sched/cpuset state, and ActivityManager is free to reset it afterwards — the UI
 * says so where it matters instead of pretending otherwise.
 *
 * `SysFlowApp` wraps this route in `FlexibleSubPage`, so no heading is drawn here.
 */
@Composable
fun ProcessScreen(viewModel: ProcessViewModel = koinViewModel()) {
    val state by viewModel.state.collectAsStateWithLifecycle()
    val lifecycleOwner = LocalLifecycleOwner.current
    val bottomInset = WindowInsets.navigationBars.asPaddingValues().calculateBottomPadding()
    var confirm by remember { mutableStateOf<ProcessConfirm?>(null) }
    var showTuning by remember { mutableStateOf(false) }

    val sheetOpen = state.detailLoading || state.detail != null
    BackHandler(enabled = state.selectionMode && !sheetOpen) { viewModel.clearSelection() }

    // Only while the page is visible; a running action or multi-select pauses the poll so the list
    // does not jump under the user's finger. Keyed on the interval so a change from the tune dialog
    // restarts the loop with the new cadence instead of waiting out the rest of the old one.
    LaunchedEffect(state.pollMs) {
        lifecycleOwner.repeatOnLifecycle(Lifecycle.State.STARTED) {
            while (isActive) {
                delay(state.pollMs.toLong())
                val current = viewModel.state.value
                if (current.actionRunning == null && !current.selectionMode) {
                    viewModel.refresh(silent = true)
                }
            }
        }
    }

    val list = state.list

    Column(Modifier.fillMaxSize()) {
        // The sheet shows its own feedback; the page banner would sit behind the scrim.
        val feedback = state.error ?: state.message
        AnimatedVisibility(visible = feedback != null && !sheetOpen && list != null) {
            val danger = state.error != null
            NoteTile(
                text = feedback.orEmpty(),
                icon = Icons.Rounded.Info,
                container = if (danger) {
                    MaterialTheme.colorScheme.errorContainer
                } else {
                    MaterialTheme.colorScheme.secondaryContainer
                },
                content = if (danger) {
                    MaterialTheme.colorScheme.onErrorContainer
                } else {
                    MaterialTheme.colorScheme.onSecondaryContainer
                },
                modifier = Modifier.padding(horizontal = 16.dp, vertical = 8.dp),
                onDismiss = viewModel::dismissMessage,
            )
        }
        Box(Modifier.weight(1f)) {
            when {
                list == null && state.loading -> Box(Modifier.fillMaxSize(), Alignment.Center) {
                    LoadingIndicator()
                }
                list == null -> Box(Modifier.fillMaxSize(), Alignment.Center) {
                    EmptyActionTile(
                        icon = Icons.Rounded.Timeline,
                        text = state.error ?: "设备没有提供 /proc，或模块尚未加载。",
                        actionLabel = "重试",
                        onAction = { viewModel.refresh() },
                        modifier = Modifier.padding(16.dp),
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
                    onOpenTuning = { showTuning = true },
                    onLoadMore = viewModel::loadMore,
                    onToggleSelectionMode = viewModel::toggleSelectionMode,
                    onEnterSelection = viewModel::enterSelectionWith,
                    onToggleSelected = viewModel::toggleSelected,
                    onOpen = viewModel::loadDetail,
                )
            }
            if (state.selectionMode && list != null) {
                val selectedEntries = list.entries.filter { it.pid in state.selectedPids }
                val packageCount = selectedEntries.mapNotNull { it.packageName }.distinct().size
                val canBatchStop = "stop" in state.features && packageCount > 0
                val canBatchFreeze = state.experimental && "freeze" in state.features && selectedEntries.isNotEmpty()
                // A plain pill instead of HorizontalFloatingToolbar: with text + button content
                // the toolbar mis-measures and renders as a huge oval over the list (the nav bar's
                // IconButton-only rows are its one working pattern). This container is fixed-size
                // and fully under our control; the colors match the toolbar's standard look.
                Surface(
                    modifier = Modifier
                        .align(Alignment.BottomCenter)
                        .padding(bottom = 16.dp + bottomInset),
                    shape = MaterialTheme.shapes.extraLarge,
                    color = MaterialTheme.colorScheme.surfaceContainerHighest,
                    contentColor = MaterialTheme.colorScheme.onSurface,
                    tonalElevation = 3.dp,
                    shadowElevation = 6.dp,
                ) {
                    Row(
                        modifier = Modifier.padding(start = 12.dp, end = 4.dp, top = 4.dp, bottom = 4.dp),
                        verticalAlignment = Alignment.CenterVertically,
                        horizontalArrangement = Arrangement.spacedBy(8.dp),
                    ) {
                        Text(
                            "已选 ${state.selectedPids.size}",
                            style = MaterialTheme.typography.labelLarge,
                            modifier = Modifier.padding(start = 4.dp),
                            maxLines = 1,
                        )
                        if (canBatchStop) {
                            FilledTonalButton(
                                onClick = {
                                    confirm = batchStopConfirm(packageCount) { viewModel.batchStop() }
                                },
                                enabled = state.actionRunning == null,
                                shapes = ButtonDefaults.shapes(),
                                contentPadding = ButtonDefaults.ExtraSmallContentPadding,
                            ) { Text("强停", maxLines = 1) }
                        }
                        if (canBatchFreeze) {
                            FilledTonalButton(
                                onClick = {
                                    confirm = batchFreezeConfirm(selectedEntries.size, true) {
                                        viewModel.batchFreeze(true)
                                    }
                                },
                                enabled = state.actionRunning == null,
                                shapes = ButtonDefaults.shapes(),
                                contentPadding = ButtonDefaults.ExtraSmallContentPadding,
                            ) { Text("冻结", maxLines = 1) }
                            FilledTonalButton(
                                onClick = {
                                    confirm = batchFreezeConfirm(selectedEntries.size, false) {
                                        viewModel.batchFreeze(false)
                                    }
                                },
                                enabled = state.actionRunning == null,
                                shapes = ButtonDefaults.shapes(),
                                contentPadding = ButtonDefaults.ExtraSmallContentPadding,
                            ) { Text("解冻", maxLines = 1) }
                        }
                        IconButton(onClick = viewModel::clearSelection) {
                            Icon(Icons.Rounded.Close, contentDescription = "退出多选")
                        }
                    }
                }
            }
        }
    }

    if (sheetOpen) {
        ProcessDetailSheet(
            state = state,
            onDismiss = viewModel::dismissDetail,
            onDismissMessage = viewModel::dismissMessage,
            onStop = { target, user -> confirm = stopConfirm(target) { viewModel.stop(target, user) } },
            onOom = { target, value -> confirm = oomConfirm(target, value) { viewModel.oom(target, value) } },
            onRenice = { target, value ->
                confirm = reniceConfirm(target, value) { viewModel.renice(target, value) }
            },
            onSched = { target, policy, priority ->
                confirm = schedConfirm(target, policy, priority) {
                    viewModel.sched(target, policy, priority)
                }
            },
            onCpuset = { target, group ->
                confirm = cpusetConfirm(target, group) { viewModel.cpuset(target, group) }
            },
            onFreeze = { target, frozen ->
                confirm = freezeConfirm(target, frozen) { viewModel.freeze(target, frozen) }
            },
        )
    }

    confirm?.let { request -> ConfirmDialog(request = request, onDismiss = { confirm = null }) }
    if (showTuning) {
        TuningDialog(
            pollMs = state.pollMs,
            sampleMs = state.sampleMs,
            onPoll = viewModel::setPollMs,
            onSample = viewModel::setSampleMs,
            onDismiss = { showTuning = false },
        )
    }
}

// ---------------------------------------------------------------------------------------------
// Poll tuning
// ---------------------------------------------------------------------------------------------

/** Poll interval and sampling window of the list, both saved on the device (not module state). */
@Composable
private fun TuningDialog(
    pollMs: Int,
    sampleMs: Int,
    onPoll: (Int) -> Unit,
    onSample: (Int) -> Unit,
    onDismiss: () -> Unit,
) {
    val scheme = MaterialTheme.colorScheme
    AlertDialog(
        onDismissRequest = onDismiss,
        shape = MaterialTheme.shapes.extraLarge,
        containerColor = scheme.surfaceContainerHigh,
        title = { Text("列表刷新", style = MaterialTheme.typography.headlineSmallEmphasized) },
        text = {
            Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
                Column(verticalArrangement = Arrangement.spacedBy(6.dp)) {
                    Text("轮询间隔", style = MaterialTheme.typography.titleSmallEmphasized)
                    ConnectedChoiceRow(
                        options = ProcessTuning.POLL_CHOICES,
                        selected = pollMs,
                        label = { pollLabel(it) },
                        onSelect = onPoll,
                    )
                }
                Column(verticalArrangement = Arrangement.spacedBy(6.dp)) {
                    Text("采样窗口", style = MaterialTheme.typography.titleSmallEmphasized)
                    ConnectedChoiceRow(
                        options = ProcessTuning.SAMPLE_CHOICES,
                        selected = sampleMs,
                        label = { "$it ms" },
                        onSelect = onSample,
                    )
                }
                Text(
                    "间隔越短列表越实时，窗口越长 CPU% 越平稳；设置保存在本机。" +
                        "模块会把采样窗口钳制在 200–2000 ms。",
                    style = MaterialTheme.typography.bodySmall,
                    color = scheme.onSurfaceVariant,
                )
            }
        },
        confirmButton = {
            Button(onClick = onDismiss, shapes = ButtonDefaults.shapes()) { Text("完成") }
        },
    )
}

private fun pollLabel(ms: Int): String = "${ms / 1000} 秒"

// ---------------------------------------------------------------------------------------------
// Confirmation
// ---------------------------------------------------------------------------------------------

/** One confirmation: the icon sits in a cookie badge, and a risky action gets the error colors. */
@Composable
private fun ConfirmDialog(request: ProcessConfirm, onDismiss: () -> Unit) {
    val scheme = MaterialTheme.colorScheme
    AlertDialog(
        onDismissRequest = onDismiss,
        shape = MaterialTheme.shapes.extraLarge,
        containerColor = scheme.surfaceContainerHigh,
        icon = {
            Box(
                modifier = Modifier
                    .size(64.dp)
                    .clip(MaterialShapes.Cookie9Sided.toShape())
                    .background(if (request.danger) scheme.errorContainer else scheme.primaryContainer),
                contentAlignment = Alignment.Center,
            ) {
                Icon(
                    request.icon,
                    contentDescription = null,
                    tint = if (request.danger) scheme.onErrorContainer else scheme.onPrimaryContainer,
                    modifier = Modifier.size(30.dp),
                )
            }
        },
        title = { Text(request.title, style = MaterialTheme.typography.headlineSmallEmphasized) },
        text = { Text(request.text, style = MaterialTheme.typography.bodyMedium) },
        confirmButton = {
            Button(
                onClick = {
                    onDismiss()
                    request.onConfirm()
                },
                shapes = ButtonDefaults.shapes(),
                colors = if (request.danger) {
                    ButtonDefaults.buttonColors(
                        containerColor = scheme.error,
                        contentColor = scheme.onError,
                    )
                } else {
                    ButtonDefaults.buttonColors()
                },
            ) { Text(request.confirmLabel) }
        },
        dismissButton = {
            TextButton(onClick = onDismiss, shapes = ButtonDefaults.shapes()) { Text("取消") }
        },
    )
}

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

private fun schedConfirm(
    target: String,
    policy: String,
    priority: Int?,
    onConfirm: () -> Unit,
) = ProcessConfirm(
    title = "修改调度策略为 $policy？",
    text = "$target 的调度策略将设为 $policy${priority?.takeIf { it > 0 }?.let { "（优先级 $it）" } ?: ""}；实时调度可能影响系统响应，ActivityManager 也可能随后重置。",
    confirmLabel = "应用",
    icon = Icons.Rounded.Bolt,
    danger = policy == "fifo" || policy == "rr",
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

private fun batchStopConfirm(packageCount: Int, onConfirm: () -> Unit) = ProcessConfirm(
    title = "批量强制停止 $packageCount 个应用？",
    text = "所选应用未保存的数据可能丢失；下次打开时会重新启动。",
    confirmLabel = "强停",
    icon = Icons.Rounded.DeleteForever,
    danger = true,
    onConfirm = onConfirm,
)

private fun batchFreezeConfirm(count: Int, frozen: Boolean, onConfirm: () -> Unit) = ProcessConfirm(
    title = if (frozen) "批量冻结 $count 项？" else "批量解冻 $count 项？",
    text = if (frozen) "被冻结的进程将停止运行；系统可能随时解冻。" else "所选进程将恢复运行。",
    confirmLabel = if (frozen) "冻结" else "解冻",
    icon = Icons.Rounded.Bedtime,
    danger = frozen,
    onConfirm = onConfirm,
)
