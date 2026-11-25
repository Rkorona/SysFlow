@file:OptIn(ExperimentalMaterial3ExpressiveApi::class, ExperimentalLayoutApi::class)

package io.sysflow.feature.cpuset

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.ExperimentalMaterial3ExpressiveApi
import androidx.compose.material3.FilledTonalButton
import androidx.compose.material3.LoadingIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.ToggleButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.draw.clip
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextDecoration
import androidx.compose.ui.unit.dp
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.compose.LocalLifecycleOwner
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.repeatOnLifecycle
import io.sysflow.core.ui.components.InfoPill
import io.sysflow.core.ui.components.cardPalette
import kotlinx.coroutines.delay
import kotlinx.coroutines.isActive
import org.koin.androidx.compose.koinViewModel

@Composable
fun CpusetScreen(
    modifier: Modifier = Modifier,
    viewModel: CpusetViewModel = koinViewModel(),
) {
    val state by viewModel.state.collectAsStateWithLifecycle()
    val lifecycleOwner = LocalLifecycleOwner.current
    var confirmResetAll by rememberSaveable { mutableStateOf(false) }

    LaunchedEffect(Unit) {
        lifecycleOwner.repeatOnLifecycle(Lifecycle.State.STARTED) {
            while (isActive) {
                delay(2_500)
                viewModel.refresh()
            }
        }
    }

    Box(modifier.fillMaxSize()) {
        if (state.loading && state.groups.isEmpty()) {
            Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
                LoadingIndicator()
            }
        } else {
            LazyColumn(
                modifier = Modifier.fillMaxSize(),
                contentPadding = PaddingValues(start = 16.dp, end = 16.dp, top = 8.dp, bottom = 24.dp),
                verticalArrangement = Arrangement.spacedBy(14.dp),
            ) {
                item(key = "status") {
                    Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                        if (state.error != null) {
                            Text(state.error.orEmpty(), color = MaterialTheme.colorScheme.error, style = MaterialTheme.typography.bodyMedium)
                        } else if (state.message != null) {
                            Text(state.message.orEmpty(), color = MaterialTheme.colorScheme.primary, style = MaterialTheme.typography.bodyMedium)
                        }
                        if (state.safeMode) {
                            InfoPill(
                                text = "安全模式下仅允许查看，修改已停用",
                                container = MaterialTheme.colorScheme.errorContainer,
                                content = MaterialTheme.colorScheme.onErrorContainer,
                            )
                        }
                    }
                }

                if (state.groups.isEmpty()) {
                    item(key = "empty") {
                        Surface(
                            color = MaterialTheme.colorScheme.surfaceContainerLow,
                            shape = MaterialTheme.shapes.extraLarge,
                        ) {
                            Text(
                                "没有发现可配置的 /dev/cpuset 进程组。该系统可能使用 cgroup v2 或不提供 cpuset 接口。",
                                modifier = Modifier.padding(20.dp),
                                style = MaterialTheme.typography.bodyMedium,
                                color = MaterialTheme.colorScheme.onSurfaceVariant,
                            )
                        }
                    }
                } else {
                    itemsIndexed(state.groups, key = { _, group -> group.id }) { index, group ->
                        ProcessGroupCard(
                            index = index,
                            group = group,
                            clusters = state.clusters,
                            busy = state.busy,
                            safeMode = state.safeMode,
                            onToggle = { viewModel.toggle(group.id, it) },
                            onSelect = { viewModel.select(group.id, it) },
                            onApply = { viewModel.apply(group.id) },
                            onReset = { viewModel.reset(group.id) },
                        )
                    }
                    item(key = "reset-all") {
                        FilledTonalButton(
                            onClick = { confirmResetAll = true },
                            enabled = !state.busy && !state.safeMode && state.groups.any { it.selected != it.default },
                            modifier = Modifier.fillMaxWidth(),
                            shapes = ButtonDefaults.shapes(),
                        ) {
                            Text("全部恢复默认")
                        }
                    }
                }
            }
        }
    }

    if (confirmResetAll) {
        AlertDialog(
            onDismissRequest = { confirmResetAll = false },
            title = { Text("恢复全部进程组？") },
            text = { Text("所有已修改的进程组将恢复为首次保存配置前记录的默认核心分配。") },
            confirmButton = {
                TextButton(onClick = {
                    confirmResetAll = false
                    viewModel.resetAll()
                }) { Text("恢复") }
            },
            dismissButton = { TextButton(onClick = { confirmResetAll = false }) { Text("取消") } },
        )
    }
}

@Composable
private fun ProcessGroupCard(
    index: Int,
    group: ProcessGroupUi,
    clusters: List<Pair<String, List<Int>>>,
    busy: Boolean,
    safeMode: Boolean,
    onToggle: (Int) -> Unit,
    onSelect: (List<Int>) -> Unit,
    onApply: () -> Unit,
    onReset: () -> Unit,
) {
    val palette = cardPalette(index)
    val chipContainer = palette.content.copy(alpha = 0.12f)
    val description = processGroupDescription(group.id)
    val problem = when {
        group.selected.isEmpty() -> "请至少选择一个核心"
        !group.hasOnlineSelection -> "至少选择一个当前在线的核心"
        else -> null
    }

    Column(
        modifier = Modifier
            .fillMaxWidth()
            // Match the original SoCFlow process-group card silhouette and spacing.
            .clip(
                RoundedCornerShape(
                    topStart = 28.dp,
                    topEnd = 44.dp,
                    bottomEnd = 28.dp,
                    bottomStart = 44.dp,
                ),
            )
            .background(palette.container)
            .padding(horizontal = 24.dp, vertical = 20.dp),
        verticalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        Row(
            modifier = Modifier.fillMaxWidth(),
            horizontalArrangement = Arrangement.SpaceBetween,
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Column(Modifier.weight(1f)) {
                Text(
                    processGroupTitle(group.id),
                    style = MaterialTheme.typography.titleMediumEmphasized,
                    color = palette.content,
                )
                Text(
                    group.id,
                    style = MaterialTheme.typography.bodySmall,
                    color = palette.content.copy(alpha = 0.8f),
                )
            }
            Column(horizontalAlignment = Alignment.End) {
                Text(
                    "当前生效",
                    style = MaterialTheme.typography.labelMedium,
                    color = palette.content.copy(alpha = 0.8f),
                )
                Text(
                    formatCpuSet(group.current),
                    style = MaterialTheme.typography.titleLargeEmphasized,
                    color = palette.content,
                )
            }
        }

        Text(
            description,
            style = MaterialTheme.typography.bodySmall,
            color = palette.content.copy(alpha = 0.8f),
        )
        Text(
            "允许使用的核心",
            style = MaterialTheme.typography.labelLarge,
            color = palette.content,
        )
        FlowRow(
            horizontalArrangement = Arrangement.spacedBy(8.dp),
            verticalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            repeat(group.count) { cpu ->
                val online = cpu in group.online
                ToggleButton(
                    checked = cpu in group.selected,
                    onCheckedChange = { onToggle(cpu) },
                    enabled = !busy && !safeMode && cpu !in group.locked,
                    modifier = Modifier
                        .alpha(if (online) 1f else 0.55f)
                        .semantics {
                            contentDescription = if (online) "CPU $cpu" else "CPU $cpu，当前离线"
                        },
                ) {
                    Text(
                        text = cpu.toString(),
                        textDecoration = if (online) null else TextDecoration.LineThrough,
                    )
                }
            }
        }

        // Filled tonal presets, matching the original SoCFlow visual hierarchy.
        FlowRow(
            horizontalArrangement = Arrangement.spacedBy(8.dp),
            verticalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            val allCpus = (0 until group.count).toList()
            PresetButton(
                text = "全部",
                enabled = !busy && !safeMode,
                container = chipContainer,
                content = palette.content,
                onClick = { onSelect(allCpus) },
            )
            clusters.forEach { (_, cpus) ->
                PresetButton(
                    text = "仅 CPU ${formatCpuSet(cpus)}",
                    enabled = !busy && !safeMode,
                    container = chipContainer,
                    content = palette.content,
                    onClick = { onSelect(cpus) },
                )
            }
        }

        if (problem != null) {
            Text(
                problem,
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.error,
            )
        }
        if (group.dirty) {
            InfoPill(
                text = "有未应用的修改",
                container = palette.accent,
                content = palette.container,
            )
        }

        Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            // Match SoCFlow: keep the reset action visually available even when the
            // current draft already equals the default. Only real runtime restrictions
            // (busy / safe mode) should disable it.
            FilledTonalButton(
                onClick = onReset,
                enabled = !busy && !safeMode,
                shapes = ButtonDefaults.shapes(),
                colors = ButtonDefaults.filledTonalButtonColors(
                    containerColor = chipContainer,
                    contentColor = palette.content,
                ),
                modifier = Modifier.weight(1f),
            ) {
                Text("恢复默认")
            }
            // Match SoCFlow: applying an unchanged draft is still a valid action.
            // Do not let the clean state dim the primary button; only disable it when
            // the page is busy, in safe mode, or the CPU selection is invalid.
            Button(
                onClick = onApply,
                enabled = !busy && !safeMode && problem == null,
                shapes = ButtonDefaults.shapes(),
                colors = ButtonDefaults.buttonColors(
                    containerColor = palette.accent,
                    contentColor = palette.container,
                ),
                modifier = Modifier.weight(1f),
            ) {
                Text("应用")
            }
        }
    }
}

/** The same compact filled-tonal preset button used by the original SoCFlow page. */
@Composable
private fun PresetButton(
    text: String,
    enabled: Boolean,
    container: androidx.compose.ui.graphics.Color,
    content: androidx.compose.ui.graphics.Color,
    onClick: () -> Unit,
) {
    FilledTonalButton(
        onClick = onClick,
        enabled = enabled,
        shapes = ButtonDefaults.shapes(),
        colors = ButtonDefaults.filledTonalButtonColors(
            containerColor = container,
            contentColor = content,
        ),
        contentPadding = ButtonDefaults.ExtraSmallContentPadding,
        modifier = Modifier.heightIn(min = ButtonDefaults.ExtraSmallContainerHeight),
    ) {
        Text(text, style = MaterialTheme.typography.labelLarge)
    }
}

private fun processGroupTitle(id: String): String = when (id) {
    "top-app" -> "顶层进程"
    "foreground" -> "前台进程"
    "system-background" -> "系统后台进程"
    "background" -> "后台进程"
    "foreground_window" -> "前台窗口进程"
    "foreground-boost" -> "前台增强进程"
    "audio-app" -> "音频进程"
    "camera-daemon" -> "相机守护进程"
    "abnormal" -> "异常进程"
    "midground" -> "中后台进程"
    "moderate" -> "中优先级进程"
    "restricted" -> "受限进程"
    "sf" -> "SurfaceFlinger"
    else -> id
}

private fun processGroupDescription(id: String): String = when (id) {
    "top-app" -> "当前正在使用的 App。"
    "foreground" -> "可见但不是顶层的进程。"
    "system-background" -> "系统的后台进程。"
    "background" -> "不可见的后台进程。"
    "foreground_window" -> "拥有前台窗口的进程。"
    "foreground-boost" -> "被临时提升优先级的前台进程。"
    "audio-app" -> "音频类应用进程，影响播放稳定性。"
    "camera-daemon" -> "相机守护进程，拍照和预览时使用。"
    "abnormal" -> "ROM 定义的异常进程分组。"
    "midground" -> "介于前台与后台之间的进程。"
    "moderate" -> "中等优先级、性能需求较低的进程。"
    "restricted" -> "被系统限制运行的进程。"
    "sf" -> "负责画面合成的 SurfaceFlinger，建议谨慎调整。"
    else -> "由当前 ROM 提供的自定义进程组；请先确认用途再调整。"
}
