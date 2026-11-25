@file:OptIn(ExperimentalMaterial3Api::class, ExperimentalMaterial3ExpressiveApi::class)

package io.sysflow.feature.screenoff

import androidx.compose.animation.animateColorAsState
import androidx.compose.animation.core.animateDpAsState
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.Check
import androidx.compose.material.icons.rounded.Close
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.ExperimentalMaterial3ExpressiveApi
import androidx.compose.material3.FilledTonalButton
import androidx.compose.material3.Icon
import androidx.compose.material3.LoadingIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Slider
import androidx.compose.material3.SliderDefaults
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.Surface
import androidx.compose.material3.Switch
import androidx.compose.material3.SwitchDefaults
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.compose.LocalLifecycleOwner
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.repeatOnLifecycle
import io.sysflow.core.model.formatKhz
import io.sysflow.core.ui.components.InfoPill
import io.sysflow.core.ui.components.SectionTitle
import io.sysflow.core.ui.components.cardPalette
import io.sysflow.core.ui.components.onPrimaryContainerSwitchColors
import kotlinx.coroutines.delay
import kotlinx.coroutines.isActive
import org.koin.androidx.compose.koinViewModel
import kotlin.math.roundToInt

@Composable
fun ScreenOffScreen(
    modifier: Modifier = Modifier,
    viewModel: ScreenOffViewModel = koinViewModel(),
) {
    val state by viewModel.state.collectAsStateWithLifecycle()
    val lifecycleOwner = LocalLifecycleOwner.current
    val snackbar = remember { SnackbarHostState() }
    var confirmClear by rememberSaveable { mutableStateOf(false) }

    LaunchedEffect(Unit) {
        lifecycleOwner.repeatOnLifecycle(Lifecycle.State.STARTED) {
            viewModel.refresh()
            while (isActive) {
                delay(2_000)
                viewModel.refresh()
            }
        }
    }
    LaunchedEffect(state.message, state.error) {
        val message = state.error ?: state.message ?: return@LaunchedEffect
        snackbar.showSnackbar(message)
        viewModel.dismissMessage()
    }

    BoxScopeScreen(modifier = modifier) {
        when {
            state.loading && state.policies.isEmpty() -> {
                androidx.compose.foundation.layout.Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
                    LoadingIndicator()
                }
            }
            else -> {
                LazyColumn(
                    modifier = Modifier.fillMaxSize(),
                    contentPadding = PaddingValues(start = 16.dp, end = 16.dp, top = 8.dp, bottom = 24.dp),
                    verticalArrangement = Arrangement.spacedBy(12.dp),
                ) {
                    item(key = "master") {
                        ScreenOffMasterCard(
                            enabled = state.enabled,
                            active = state.active,
                            busy = state.pending,
                            safeMode = state.safeMode,
                            onChange = viewModel::setEnabled,
                        )
                    }
                    item(key = "frequency-title") {
                        SectionTitle("熄屏时的最高频率")
                    }
                    if (state.policies.isEmpty()) {
                        item(key = "no-policy") {
                            InfoCard("设备没有提供可用的 CPU 调频簇，无法设置熄屏频率上限。")
                        }
                    }
                    items(state.policies.size, key = { "policy-${state.policies[it].id}" }) { index ->
                        val policy = state.policies[index]
                        ScreenOffCapCard(
                            policy = policy,
                            index = state.capIndices[policy.id] ?: policy.steps.lastIndex.coerceAtLeast(0),
                            palette = cardPalette(index),
                            active = state.active && state.enabled,
                            busy = state.pending || state.safeMode,
                            onChange = { viewModel.setCap(policy.id, it) },
                        )
                    }
                    item(key = "cores-title") {
                        SectionTitle("熄屏时关闭的核心")
                    }
                    if (state.coreCount == 0) {
                        item(key = "no-cores") { InfoCard("设备没有提供 CPU 核心在线控制接口。") }
                    } else {
                        items((state.coreCount + 1) / 2, key = { "core-row-$it" }) { rowIndex ->
                            Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(10.dp)) {
                                repeat(2) { column ->
                                    val cpu = rowIndex * 2 + column
                                    if (cpu < state.coreCount) {
                                        val selected = cpu in state.offCores
                                        val locked = cpu in state.locked || (cpu !in state.baseOnline && !selected)
                                        val canClose = state.canClose(cpu)
                                        ScreenOffCoreCard(
                                            cpu = cpu,
                                            closed = selected,
                                            online = cpu in state.online,
                                            reason = when {
                                                cpu in state.locked -> "该核心不可下线"
                                                cpu !in state.baseOnline && selected -> "常规配置已关闭，可移除此项"
                                                cpu !in state.baseOnline -> "常规配置已关闭"
                                                !selected && !canClose -> "至少保留一个核心在线"
                                                else -> if (selected) "熄屏时关闭" else "熄屏时保持在线"
                                            },
                                            enabled = !state.pending && !state.safeMode && !locked && (selected || canClose),
                                            modifier = Modifier.weight(1f),
                                            onChange = { viewModel.setCoreClosed(cpu, it) },
                                        )
                                    } else {
                                        Spacer(Modifier.weight(1f))
                                    }
                                }
                            }
                        }
                    }
                    item(key = "save") {
                        Column(verticalArrangement = Arrangement.spacedBy(10.dp)) {
                            if (state.active) {
                                InfoPill(
                                    text = "熄屏配置当前正在生效",
                                    container = MaterialTheme.colorScheme.tertiaryContainer,
                                    content = MaterialTheme.colorScheme.onTertiaryContainer,
                                )
                            } else if (state.enabled) {
                                InfoPill(
                                    text = "已启用，等待下一次熄屏",
                                    container = MaterialTheme.colorScheme.secondaryContainer,
                                    content = MaterialTheme.colorScheme.onSecondaryContainer,
                                )
                            }
                            if (state.error != null) {
                                InfoCard(state.error!!, error = true)
                            }
                            Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(10.dp)) {
                                FilledTonalButton(
                                    onClick = { confirmClear = true },
                                    enabled = !state.pending && !state.safeMode,
                                    shapes = ButtonDefaults.shapes(),
                                    modifier = Modifier.weight(1f),
                                ) { Text("清除配置") }
                                Button(
                                    onClick = viewModel::save,
                                    enabled = state.canSave,
                                    shapes = ButtonDefaults.shapes(),
                                    modifier = Modifier.weight(1f),
                                ) { Text(if (state.pending) "正在应用…" else "保存配置") }
                            }
                            if (state.dirty) {
                                Text(
                                    "有尚未保存的修改",
                                    style = MaterialTheme.typography.labelMedium,
                                    color = MaterialTheme.colorScheme.primary,
                                )
                            }
                            if (state.safeMode) {
                                InfoCard("模块处于安全模式，当前只能查看熄屏配置，不能修改。", error = true)
                            }
                        }
                    }
                    item(key = "hints") {
                        Column(
                            modifier = Modifier
                                .fillMaxWidth()
                                .clip(MaterialTheme.shapes.large)
                                .background(MaterialTheme.colorScheme.surfaceContainerLow)
                                .padding(20.dp),
                            verticalArrangement = Arrangement.spacedBy(8.dp),
                        ) {
                            listOf(
                                "屏幕熄灭后由模块状态监测器应用限制；亮屏后自动恢复熄屏前的频率范围和核心在线状态。",
                                "滑块最右侧为“不限制”。熄屏最高频率只会收紧正常上限，不会突破静态配置。",
                                "这里关闭的核心只在熄屏期间下线，不会修改 CPU 页面中的常规核心开关配置。",
                                "至少保留一个正常配置为在线的核心。限频过低可能影响音乐、导航、通话等后台任务。",
                                "需要启用模块的开机应用，系统启动后才能持续检测屏幕状态并自动切换配置。",
                            ).forEach { hint ->
                                Text(
                                    "· $hint",
                                    style = MaterialTheme.typography.bodySmall,
                                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                                )
                            }
                        }
                    }
                }
            }
        }
        SnackbarHost(hostState = snackbar, modifier = Modifier.align(Alignment.BottomCenter).padding(bottom = 16.dp))
    }

    if (confirmClear) {
        AlertDialog(
            onDismissRequest = { confirmClear = false },
            title = { Text("清除熄屏限制配置？") },
            text = { Text("各调频簇恢复为“不限制”，并清空熄屏时关闭的核心列表。总开关状态保持不变。") },
            confirmButton = {
                TextButton(onClick = {
                    confirmClear = false
                    viewModel.clear()
                }) { Text("清除") }
            },
            dismissButton = { TextButton(onClick = { confirmClear = false }) { Text("取消") } },
        )
    }
}

@Composable
private fun BoxScopeScreen(modifier: Modifier, content: @Composable androidx.compose.foundation.layout.BoxScope.() -> Unit) {
    androidx.compose.foundation.layout.Box(modifier.fillMaxSize(), content = content)
}

@Composable
private fun ScreenOffMasterCard(
    enabled: Boolean,
    active: Boolean,
    busy: Boolean,
    safeMode: Boolean,
    onChange: (Boolean) -> Unit,
) {
    val colors = MaterialTheme.colorScheme
    val container by animateColorAsState(
        targetValue = if (enabled) colors.primaryContainer else colors.surfaceContainerHigh,
        animationSpec = MaterialTheme.motionScheme.defaultEffectsSpec(),
        label = "screenOffMasterContainer",
    )
    val content = if (enabled) colors.onPrimaryContainer else colors.onSurface
    Surface(
        shape = MaterialTheme.shapes.extraLarge,
        color = container,
        contentColor = content,
        modifier = Modifier.fillMaxWidth(),
    ) {
        Row(
            modifier = Modifier.padding(start = 22.dp, end = 16.dp, top = 18.dp, bottom = 18.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(4.dp)) {
                Text("启用熄屏限制", style = MaterialTheme.typography.titleMediumEmphasized)
                Text(
                    when {
                        !enabled -> "已关闭"
                        active -> "正在应用熄屏频率与核心限制"
                        else -> "已开启，等待屏幕关闭"
                    },
                    style = MaterialTheme.typography.bodySmall,
                    color = content.copy(alpha = 0.82f),
                )
                Text(
                    "亮屏后自动恢复正常频率与核心状态",
                    style = MaterialTheme.typography.bodySmall,
                    color = content.copy(alpha = 0.72f),
                )
            }
            Switch(
                checked = enabled,
                onCheckedChange = onChange,
                enabled = !busy && !safeMode,
                colors = if (enabled) onPrimaryContainerSwitchColors() else SwitchDefaults.colors(),
                thumbContent = {
                    Icon(
                        imageVector = if (enabled) Icons.Rounded.Check else Icons.Rounded.Close,
                        contentDescription = null,
                        modifier = Modifier.size(SwitchDefaults.IconSize),
                    )
                },
            )
        }
    }
}

@Composable
private fun ScreenOffCapCard(
    policy: ScreenOffPolicy,
    index: Int,
    palette: io.sysflow.core.ui.components.CardPalette,
    active: Boolean,
    busy: Boolean,
    onChange: (Int) -> Unit,
) {
    Column(
        modifier = Modifier
            .fillMaxWidth()
            .clip(RoundedCornerShape(topStart = 28.dp, topEnd = 44.dp, bottomEnd = 28.dp, bottomStart = 44.dp))
            .background(palette.container)
            .padding(horizontal = 22.dp, vertical = 18.dp),
        verticalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(2.dp)) {
                Text("Cluster ${policy.id} · CPU ${formatCpuRange(policy.cpus)}", style = MaterialTheme.typography.titleMediumEmphasized, color = palette.content)
                Text(
                    "硬件频率范围 ${formatKhz(policy.hwMin)} – ${formatKhz(policy.hwMax)}",
                    style = MaterialTheme.typography.bodySmall,
                    color = palette.content.copy(alpha = 0.78f),
                )
            }
            Column(horizontalAlignment = Alignment.End) {
                Text(
                    if (!policy.usable || index >= policy.steps.lastIndex) "不限制" else formatKhz(policy.steps[index]),
                    style = MaterialTheme.typography.headlineSmallEmphasized,
                    color = palette.content,
                    fontWeight = FontWeight.SemiBold,
                )
                if (active && policy.liveMax > 0L) {
                    Text("当前上限 ${formatKhz(policy.liveMax)}", style = MaterialTheme.typography.labelSmall, color = palette.content.copy(alpha = 0.72f))
                }
            }
        }
        if (!policy.usable) {
            Text("无法读取该 Cluster 的频率范围；保存其它项目时会保留当前设置。", style = MaterialTheme.typography.bodySmall, color = palette.content.copy(alpha = 0.8f))
        } else if (policy.steps.size < 2) {
            Text("该 Cluster 只有一个可用频率点，当前只能选择不限制。", style = MaterialTheme.typography.bodySmall, color = palette.content.copy(alpha = 0.8f))
        } else {
            Text("熄屏最高频率", style = MaterialTheme.typography.labelLarge, color = palette.content.copy(alpha = 0.82f))
            Slider(
                value = index.coerceIn(0, policy.steps.lastIndex).toFloat(),
                onValueChange = { onChange(it.roundToInt()) },
                valueRange = 0f..policy.steps.lastIndex.toFloat(),
                steps = (policy.steps.size - 2).coerceAtLeast(0),
                enabled = !busy,
                colors = SliderDefaults.colors(
                    thumbColor = palette.accent,
                    activeTrackColor = palette.accent,
                    inactiveTrackColor = palette.content.copy(alpha = 0.18f),
                    activeTickColor = palette.container,
                    inactiveTickColor = palette.content.copy(alpha = 0.34f),
                ),
            )
            Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.SpaceBetween) {
                Text(formatKhz(policy.steps.first()), style = MaterialTheme.typography.labelMedium, color = palette.content.copy(alpha = 0.76f))
                Text("不限制", style = MaterialTheme.typography.labelMedium, color = palette.content.copy(alpha = 0.76f))
            }
        }
    }
}

@Composable
private fun ScreenOffCoreCard(
    cpu: Int,
    closed: Boolean,
    online: Boolean,
    reason: String,
    enabled: Boolean,
    modifier: Modifier = Modifier,
    onChange: (Boolean) -> Unit,
) {
    val colors = MaterialTheme.colorScheme
    val corner by animateDpAsState(
        targetValue = if (closed) 42.dp else 26.dp,
        animationSpec = MaterialTheme.motionScheme.defaultSpatialSpec(),
        label = "screenOffCoreCorner",
    )
    val container by animateColorAsState(
        targetValue = when {
            closed -> colors.tertiaryContainer
            !online -> colors.surfaceContainerLow
            else -> colors.surfaceContainerHigh
        },
        animationSpec = MaterialTheme.motionScheme.defaultEffectsSpec(),
        label = "screenOffCoreContainer",
    )
    val content = if (closed) colors.onTertiaryContainer else colors.onSurface
    Row(
        modifier = modifier
            .clip(RoundedCornerShape(corner))
            .background(container)
            .padding(start = 14.dp, end = 8.dp, top = 12.dp, bottom = 12.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(2.dp)) {
            Text("CPU $cpu", style = MaterialTheme.typography.titleMediumEmphasized, color = content)
            Text(reason, style = MaterialTheme.typography.bodySmall, color = content.copy(alpha = 0.78f))
        }
        Switch(
            checked = closed,
            onCheckedChange = onChange,
            enabled = enabled,
            thumbContent = {
                Icon(
                    if (closed) Icons.Rounded.Check else Icons.Rounded.Close,
                    contentDescription = null,
                    modifier = Modifier.size(SwitchDefaults.IconSize),
                )
            },
        )
    }
}

@Composable
private fun InfoCard(text: String, error: Boolean = false) {
    val colors = MaterialTheme.colorScheme
    val container = if (error) colors.errorContainer else colors.surfaceContainerLow
    val content = if (error) colors.onErrorContainer else colors.onSurfaceVariant
    Text(
        text = text,
        modifier = Modifier
            .fillMaxWidth()
            .clip(MaterialTheme.shapes.large)
            .background(container)
            .padding(18.dp),
        style = MaterialTheme.typography.bodySmall,
        color = content,
    )
}

private fun formatCpuRange(cpus: List<Int>): String {
    val sorted = cpus.sorted()
    if (sorted.isEmpty()) return "—"
    if (sorted.size == 1) return sorted.first().toString()
    if (sorted.zipWithNext().all { (a, b) -> b == a + 1 }) return "${sorted.first()}–${sorted.last()}"
    return sorted.joinToString(", ")
}
