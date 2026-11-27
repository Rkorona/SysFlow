@file:OptIn(ExperimentalMaterial3ExpressiveApi::class)

package io.sysflow.feature.adaptive

import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.animateContentSize
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.AutoGraph
import androidx.compose.material.icons.rounded.BatteryAlert
import androidx.compose.material.icons.rounded.Check
import androidx.compose.material.icons.rounded.Close
import androidx.compose.material.icons.rounded.ExpandMore
import androidx.compose.material.icons.rounded.Insights
import androidx.compose.material.icons.rounded.RestartAlt
import androidx.compose.material.icons.rounded.Save
import androidx.compose.material.icons.rounded.Thermostat
import androidx.compose.material.icons.rounded.Tune
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.ExperimentalMaterial3ExpressiveApi
import androidx.compose.material3.FilledTonalButton
import androidx.compose.material3.Icon
import androidx.compose.material3.LoadingIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
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
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.rotate
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.Shape
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.compose.LocalLifecycleOwner
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.repeatOnLifecycle
import io.sysflow.core.ui.components.ExpressiveCard
import io.sysflow.core.ui.components.ExpressiveShapes
import io.sysflow.core.ui.components.IconBadge
import io.sysflow.core.ui.components.InfoPill
import io.sysflow.core.ui.components.SectionTitle
import io.sysflow.core.ui.components.SegmentedValueList
import io.sysflow.core.ui.components.ShapeStat
import io.sysflow.core.ui.components.StatusPill
import io.sysflow.core.ui.components.StatusTone
import io.sysflow.core.ui.components.onPrimaryContainerSwitchColors
import io.sysflow.core.ui.components.segmentedShape
import io.sysflow.core.ui.theme.sysFlowColors
import kotlinx.coroutines.delay
import kotlinx.coroutines.isActive
import org.koin.androidx.compose.koinViewModel

private data class AdaptiveFieldUi(
    val key: String,
    val label: String,
    val unit: String,
)

private data class AdaptiveGroupUi(
    val id: String,
    val title: String,
    val description: String,
    val summary: String,
    val icon: ImageVector,
    val fields: List<AdaptiveFieldUi>,
)

private val adaptiveGroups = listOf(
    AdaptiveGroupUi(
        id = "smart",
        title = "智能负载分档",
        description = "根据实时负载，为每个 CPU 调频簇逐档调整频率上限",
        summary = "上限 45/65/85/100% · 阈值 12/35/70%",
        icon = Icons.Rounded.Tune,
        fields = listOf(
            AdaptiveFieldUi("cpu.adaptive.load_cap1", "第 1 档频率上限", "%"),
            AdaptiveFieldUi("cpu.adaptive.load_cap2", "第 2 档频率上限", "%"),
            AdaptiveFieldUi("cpu.adaptive.load_cap3", "第 3 档频率上限", "%"),
            AdaptiveFieldUi("cpu.adaptive.load_cap4", "第 4 档频率上限", "%"),
            AdaptiveFieldUi("cpu.adaptive.load_t1", "第 1 档负载阈值", "%"),
            AdaptiveFieldUi("cpu.adaptive.load_t2", "第 2 档负载阈值", "%"),
            AdaptiveFieldUi("cpu.adaptive.load_t3", "第 3 档负载阈值", "%"),
            AdaptiveFieldUi("cpu.adaptive.load_down_rounds", "降档确认次数", "次"),
        ),
    ),
    AdaptiveGroupUi(
        id = "thermal",
        title = "温度动态收紧",
        description = "CPU / GPU 超过启动温度后逐档收紧，降温后逐档恢复",
        summary = "启动 75°C · 恢复 68°C",
        icon = Icons.Rounded.Thermostat,
        fields = listOf(
            AdaptiveFieldUi("cpu.adaptive.temp_high", "启动收紧温度", "°C"),
            AdaptiveFieldUi("cpu.adaptive.temp_low", "开始恢复温度", "°C"),
            AdaptiveFieldUi("cpu.adaptive.temp_step", "每档收紧幅度", "%"),
            AdaptiveFieldUi("cpu.adaptive.temp_max", "最大收紧幅度", "%"),
            AdaptiveFieldUi("cpu.adaptive.temp_release_rounds", "恢复确认次数", "次"),
        ),
    ),
    AdaptiveGroupUi(
        id = "battery",
        title = "低电量节能",
        description = "仅电池放电时收紧 CPU / GPU 上限，分别设置低电量与临界电量阈值",
        summary = "低电量 20% · 临界电量 10%",
        icon = Icons.Rounded.BatteryAlert,
        fields = listOf(
            AdaptiveFieldUi("cpu.adaptive.batt_low", "低电量阈值", "%"),
            AdaptiveFieldUi("cpu.adaptive.batt_critical", "临界电量阈值", "%"),
            AdaptiveFieldUi("cpu.adaptive.batt_low_cut", "低电量收紧幅度", "%"),
            AdaptiveFieldUi("cpu.adaptive.batt_critical_cut", "临界额外收紧幅度", "%"),
            AdaptiveFieldUi("cpu.adaptive.batt_hyst", "恢复滞后范围", "%"),
        ),
    ),
    AdaptiveGroupUi(
        id = "psi",
        title = "PSI 性能护栏",
        description = "CPU PSI avg10 达到阈值时，将低负载档位向上抬高；需要同时开启智能负载分档",
        summary = "PSI 阈值 8 · 最低性能上限 85%",
        icon = Icons.Rounded.Insights,
        fields = listOf(
            AdaptiveFieldUi("cpu.adaptive.psi_threshold", "PSI avg10 阈值", "%"),
            AdaptiveFieldUi("cpu.adaptive.psi_cap", "最低性能上限", "%"),
        ),
    ),
)

private const val COLLAPSED_SENSORS = 4

@Composable
fun AdaptiveScreen(
    modifier: Modifier = Modifier,
    viewModel: AdaptiveViewModel = koinViewModel(),
) {
    val state by viewModel.state.collectAsStateWithLifecycle()
    val snackbar = remember { SnackbarHostState() }
    val lifecycleOwner = LocalLifecycleOwner.current
    var expanded by rememberSaveable { mutableStateOf(setOf<String>()) }
    var confirmReset by rememberSaveable { mutableStateOf(false) }

    LaunchedEffect(Unit) {
        viewModel.load()
        lifecycleOwner.repeatOnLifecycle(Lifecycle.State.STARTED) {
            while (isActive) {
                delay(2_000)
                viewModel.refresh()
            }
        }
    }
    LaunchedEffect(state.message) {
        val message = state.message ?: return@LaunchedEffect
        snackbar.showSnackbar(message)
        viewModel.dismissMessage()
    }

    val runtime = state.runtime
    val liveRows = buildList {
        if (runtime.enabled) {
            add("动态上限" to "CPU ${runtime.cpuAdjustment}% · GPU ${runtime.gpuAdjustment}%")
            if (runtime.temperatureCut > 0 || runtime.batteryCut > 0 || runtime.psiFloor > 0) {
                add("收紧来源" to "温度 ${runtime.temperatureCut}% · 电量 ${runtime.batteryCut}% · PSI ${runtime.psiFloor}%")
            }
            runtime.policies.forEach { policy ->
                add("调频簇 ${policy.id}" to "第 ${policy.tier + 1} 档 · ${policy.load?.let { value -> "$value%" } ?: "采样中"}")
            }
        } else {
            add("动态限频" to "开启引擎后显示")
        }
        runtime.temperatureSensor?.let { add("最热传感器" to it) }
        state.batteryTempDeciC?.let { add("电池温度" to "${it / 10}.${it % 10}°C") }
    }

    Box(modifier.fillMaxSize()) {
        LazyColumn(
            modifier = Modifier.fillMaxSize(),
            contentPadding = PaddingValues(start = 16.dp, end = 16.dp, top = 8.dp, bottom = 24.dp),
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            item {
                EngineCard(
                    enabled = state.config["cpu.adaptive"] == 1,
                    pending = state.pending,
                    onToggle = { viewModel.toggle("cpu.adaptive", it) },
                    guardEnabled = runtime.guardEnabled,
                    guardRunning = runtime.guardRunning,
                    guardIntervalSeconds = runtime.guardIntervalSeconds,
                )
            }

            // Live data sits right under the engine switch: it is what you check after changing anything.
            item { SectionTitle("实时状态") }
            item {
                LiveStatsCard(
                    cpuPercent = state.live.totalUsage?.times(100f),
                    gpuPercent = state.gpuBusy,
                    maxTemperature = runtime.maxTemperature,
                )
            }
            item { SegmentedValueList(rows = liveRows) }
            item { SensorsCard(sensors = state.sensors) }

            item { SectionTitle("调节策略") }
            items(adaptiveGroups, key = { it.id }) { group ->
                StrategyCard(
                    group = group,
                    enabled = state.config["cpu.adaptive.${group.id}"] == 1,
                    expanded = group.id in expanded,
                    pending = state.pending,
                    drafts = state.drafts,
                    dirty = group.id in state.dirtyGroups,
                    onToggle = { viewModel.toggle("cpu.adaptive.${group.id}", it) },
                    onExpand = {
                        expanded = if (group.id in expanded) expanded - group.id else expanded + group.id
                    },
                    onEdit = viewModel::edit,
                    onSave = { viewModel.saveGroup(group.id) },
                )
            }
            item {
                FilledTonalButton(
                    onClick = { confirmReset = true },
                    enabled = !state.pending && !state.loading,
                    modifier = Modifier.fillMaxWidth(),
                    shapes = ButtonDefaults.shapes(),
                ) {
                    Icon(Icons.Rounded.RestartAlt, contentDescription = null, modifier = Modifier.size(ButtonDefaults.IconSize))
                    Spacer(Modifier.width(ButtonDefaults.IconSpacing))
                    Text("恢复自适应默认配置")
                }
            }
            if (state.loading && state.config.isEmpty()) {
                item {
                    Box(Modifier.fillMaxWidth().padding(20.dp), contentAlignment = Alignment.Center) {
                        LoadingIndicator()
                    }
                }
            }
        }
        SnackbarHost(
            hostState = snackbar,
            modifier = Modifier.align(Alignment.BottomCenter).padding(bottom = 16.dp, start = 16.dp, end = 16.dp),
        )
    }

    if (confirmReset) {
        AlertDialog(
            onDismissRequest = { confirmReset = false },
            icon = { Icon(Icons.Rounded.RestartAlt, contentDescription = null, tint = MaterialTheme.colorScheme.error) },
            title = { Text("恢复默认配置？") },
            text = { Text("将关闭自适应引擎和各项策略，并恢复所有参数默认值。") },
            confirmButton = {
                Button(
                    onClick = { confirmReset = false; viewModel.resetDefaults() },
                    shapes = ButtonDefaults.shapes(),
                    colors = ButtonDefaults.buttonColors(
                        containerColor = MaterialTheme.colorScheme.error,
                        contentColor = MaterialTheme.colorScheme.onError,
                    ),
                ) { Text("恢复") }
            },
            dismissButton = {
                TextButton(onClick = { confirmReset = false }, shapes = ButtonDefaults.shapes()) { Text("取消") }
            },
        )
    }
}

// ---------------------------------------------------------------------------------------------
// Engine
// ---------------------------------------------------------------------------------------------

@Composable
private fun EngineCard(
    enabled: Boolean,
    pending: Boolean,
    guardEnabled: Boolean,
    guardRunning: Boolean,
    guardIntervalSeconds: Int,
    onToggle: (Boolean) -> Unit,
) {
    val scheme = MaterialTheme.colorScheme
    val muted = if (enabled) scheme.onPrimaryContainer.copy(alpha = 0.82f) else scheme.onSurfaceVariant
    ExpressiveCard(
        containerColor = if (enabled) scheme.primaryContainer else scheme.surfaceContainerLow,
        contentColor = if (enabled) scheme.onPrimaryContainer else scheme.onSurface,
    ) {
        Row(
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(16.dp),
        ) {
            Box(Modifier.size(60.dp), contentAlignment = Alignment.Center) {
                Surface(
                    modifier = Modifier.matchParentSize(),
                    shape = ExpressiveShapes.Cookie9,
                    color = if (enabled) scheme.primary else scheme.surfaceContainerHighest,
                ) {}
                Icon(
                    Icons.Rounded.AutoGraph,
                    contentDescription = null,
                    tint = if (enabled) scheme.onPrimary else scheme.onSurfaceVariant,
                    modifier = Modifier.size(28.dp),
                )
            }
            Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(2.dp)) {
                Text("自适应引擎", style = MaterialTheme.typography.titleLargeEmphasized)
                Text(
                    "在已保存的频率上限之上动态调整",
                    style = MaterialTheme.typography.bodySmall,
                    color = muted,
                )
            }
            Switch(
                checked = enabled,
                onCheckedChange = onToggle,
                enabled = !pending,
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
        when {
            !enabled -> InfoPill("引擎已关闭", scheme.surfaceContainerHighest, scheme.onSurfaceVariant)
            guardRunning -> StatusPill("守护运行中 · 每 $guardIntervalSeconds 秒", StatusTone.Positive)
            guardEnabled -> StatusPill("守护进程未运行", StatusTone.Warning)
            else -> StatusPill("持续守护未开启", StatusTone.Warning)
        }
        Text(
            when {
                !enabled -> "只调整已保存的 CPU / GPU 频率限制，不会突破静态上限；关闭引擎后恢复已保存的频率限制。"
                guardRunning -> "后台守护会定期采样并修复配置漂移，不会突破已保存的静态上限。"
                guardEnabled -> "守护已启用，但后台进程未运行；请检查模块服务和日志。"
                else -> "策略已启用，但持续守护未开启；请前往设置启用。"
            },
            style = MaterialTheme.typography.bodySmall,
            color = muted,
        )
    }
}

// ---------------------------------------------------------------------------------------------
// Live status
// ---------------------------------------------------------------------------------------------

/** Three Expressive shapes: CPU load, GPU load and the hottest thermal zone. */
@Composable
private fun LiveStatsCard(
    cpuPercent: Float?,
    gpuPercent: Int?,
    maxTemperature: Int?,
) {
    val scheme = MaterialTheme.colorScheme
    val temp = maxTemperature?.takeIf { it >= 0 }
    val (tempContainer, tempContent) = temperatureColors(temp?.toFloat())
    ExpressiveCard {
        Row(
            modifier = Modifier.fillMaxWidth(),
            horizontalArrangement = Arrangement.SpaceEvenly,
        ) {
            ShapeStat(
                value = cpuPercent?.let { "${it.toInt().coerceIn(0, 100)}%" } ?: "—",
                label = "CPU 负载",
                shape = ExpressiveShapes.Cookie9,
                container = scheme.primaryContainer,
                content = scheme.onPrimaryContainer,
            )
            ShapeStat(
                value = gpuPercent?.let { "$it%" } ?: "—",
                label = "GPU 负载",
                shape = ExpressiveShapes.Clover4,
                container = scheme.tertiaryContainer,
                content = scheme.onTertiaryContainer,
            )
            ShapeStat(
                value = temp?.let { "$it°C" } ?: "—",
                label = "最高温度",
                shape = ExpressiveShapes.Sunny,
                container = tempContainer,
                content = tempContent,
            )
        }
    }
}

/** Container / content pair for a temperature in °C: calm below 60, warning to 75, error above. */
@Composable
private fun temperatureColors(celsius: Float?): Pair<Color, Color> {
    val scheme = MaterialTheme.colorScheme
    val extended = MaterialTheme.sysFlowColors
    return when {
        celsius == null -> scheme.secondaryContainer to scheme.onSecondaryContainer
        celsius >= 75f -> scheme.errorContainer to scheme.onErrorContainer
        celsius >= 60f -> extended.warningContainer to extended.onWarningContainer
        else -> scheme.secondaryContainer to scheme.onSecondaryContainer
    }
}

/**
 * Sensor list kept short on purpose: the four hottest zones as a two-column tile grid, the rest
 * behind a single expand button. Sorting by temperature means the collapsed view is the useful one.
 */
@Composable
private fun SensorsCard(sensors: List<AdaptiveSensor>) {
    val scheme = MaterialTheme.colorScheme
    var showAll by rememberSaveable { mutableStateOf(false) }
    val sorted = remember(sensors) { sensors.sortedByDescending { it.temperatureDeciC } }
    val visible = if (showAll) sorted else sorted.take(COLLAPSED_SENSORS)

    ExpressiveCard(modifier = Modifier.animateContentSize()) {
        Row(
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            IconBadge(Icons.Rounded.Thermostat)
            Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(2.dp)) {
                Text("温控传感器", style = MaterialTheme.typography.titleMediumEmphasized)
                Text(
                    "CPU / GPU / SoC 温度，从高到低，每 2 秒刷新",
                    style = MaterialTheme.typography.bodySmall,
                    color = scheme.onSurfaceVariant,
                )
            }
            if (sorted.isNotEmpty()) {
                InfoPill("${sorted.size} 个", scheme.secondaryContainer, scheme.onSecondaryContainer)
            }
        }
        if (sorted.isEmpty()) {
            Text(
                "暂未读取到可用的 CPU / GPU / SoC 温度传感器",
                style = MaterialTheme.typography.bodySmall,
                color = scheme.onSurfaceVariant,
            )
        } else {
            Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                visible.chunked(2).forEach { pair ->
                    Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                        pair.forEach { sensor ->
                            SensorTile(sensor, Modifier.weight(1f))
                        }
                        if (pair.size == 1) Spacer(Modifier.weight(1f))
                    }
                }
            }
            if (sorted.size > COLLAPSED_SENSORS) {
                FilledTonalButton(
                    onClick = { showAll = !showAll },
                    shapes = ButtonDefaults.shapes(),
                    modifier = Modifier.fillMaxWidth(),
                ) {
                    Text(if (showAll) "收起" else "展开全部 ${sorted.size} 个")
                    Spacer(Modifier.width(ButtonDefaults.IconSpacing))
                    Icon(
                        Icons.Rounded.ExpandMore,
                        contentDescription = null,
                        modifier = Modifier
                            .size(ButtonDefaults.IconSize)
                            .rotate(if (showAll) 180f else 0f),
                    )
                }
            }
        }
    }
}

@Composable
private fun SensorTile(sensor: AdaptiveSensor, modifier: Modifier = Modifier) {
    val scheme = MaterialTheme.colorScheme
    val celsius = sensor.temperatureDeciC / 10f
    val (container, content) = temperatureColors(celsius)
    // Only warm/hot tiles get a tint; cool ones stay neutral so the eye lands on what matters.
    val neutral = celsius < 60f
    Surface(
        modifier = modifier,
        shape = MaterialTheme.shapes.large,
        color = if (neutral) scheme.surfaceContainerHigh else container,
        contentColor = if (neutral) scheme.onSurface else content,
    ) {
        Column(
            modifier = Modifier.padding(horizontal = 14.dp, vertical = 12.dp),
            verticalArrangement = Arrangement.spacedBy(2.dp),
        ) {
            Text(
                sensor.name,
                style = MaterialTheme.typography.labelMedium,
                color = if (neutral) scheme.onSurfaceVariant else content.copy(alpha = 0.85f),
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
            Text(
                "${sensor.temperatureDeciC / 10}.${sensor.temperatureDeciC % 10}°C",
                style = MaterialTheme.typography.titleMediumEmphasized,
            )
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Strategies
// ---------------------------------------------------------------------------------------------

@Composable
private fun StrategyCard(
    group: AdaptiveGroupUi,
    enabled: Boolean,
    expanded: Boolean,
    pending: Boolean,
    drafts: Map<String, String>,
    dirty: Boolean,
    onToggle: (Boolean) -> Unit,
    onExpand: () -> Unit,
    onEdit: (String, String) -> Unit,
    onSave: () -> Unit,
) {
    val scheme = MaterialTheme.colorScheme
    val chevron by animateFloatAsState(
        targetValue = if (expanded) 180f else 0f,
        animationSpec = MaterialTheme.motionScheme.defaultSpatialSpec(),
        label = "strategyChevron",
    )
    ExpressiveCard(modifier = Modifier.animateContentSize()) {
        Row(
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            IconBadge(
                icon = group.icon,
                container = if (enabled) scheme.primaryContainer else scheme.secondaryContainer,
                content = if (enabled) scheme.onPrimaryContainer else scheme.onSecondaryContainer,
            )
            Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(2.dp)) {
                Text(group.title, style = MaterialTheme.typography.titleMediumEmphasized)
                Text(
                    if (enabled) summaryForGroup(group, drafts) else "已关闭",
                    style = MaterialTheme.typography.bodySmall,
                    color = scheme.onSurfaceVariant,
                    maxLines = 2,
                    overflow = TextOverflow.Ellipsis,
                )
            }
            Switch(
                checked = enabled,
                onCheckedChange = onToggle,
                enabled = !pending,
                thumbContent = {
                    Icon(
                        imageVector = if (enabled) Icons.Rounded.Check else Icons.Rounded.Close,
                        contentDescription = null,
                        modifier = Modifier.size(SwitchDefaults.IconSize),
                    )
                },
            )
        }
        Text(group.description, style = MaterialTheme.typography.bodySmall, color = scheme.onSurfaceVariant)
        FilledTonalButton(
            onClick = onExpand,
            shapes = ButtonDefaults.shapes(),
            modifier = Modifier.fillMaxWidth(),
        ) {
            Icon(Icons.Rounded.Tune, contentDescription = null, modifier = Modifier.size(ButtonDefaults.IconSize))
            Spacer(Modifier.width(ButtonDefaults.IconSpacing))
            Text("参数设置 · ${group.fields.size} 项", modifier = Modifier.weight(1f))
            if (dirty) {
                InfoPill("未保存", scheme.tertiaryContainer, scheme.onTertiaryContainer)
                Spacer(Modifier.width(8.dp))
            }
            Icon(
                Icons.Rounded.ExpandMore,
                contentDescription = if (expanded) "收起参数" else "展开参数",
                modifier = Modifier.size(ButtonDefaults.IconSize).rotate(chevron),
            )
        }
        AnimatedVisibility(visible = expanded) {
            Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
                group.fields.forEachIndexed { index, field ->
                    FieldRow(
                        field = field,
                        value = drafts[field.key].orEmpty(),
                        enabled = !pending,
                        shape = segmentedShape(index, group.fields.size),
                        onValueChange = { onEdit(field.key, it) },
                    )
                }
                AnimatedVisibility(visible = dirty) {
                    Button(
                        onClick = onSave,
                        enabled = !pending,
                        shapes = ButtonDefaults.shapes(),
                        modifier = Modifier.fillMaxWidth().padding(top = 8.dp),
                    ) {
                        Icon(Icons.Rounded.Save, contentDescription = null, modifier = Modifier.size(ButtonDefaults.IconSize))
                        Spacer(Modifier.width(ButtonDefaults.IconSpacing))
                        Text("保存${group.title}参数")
                    }
                }
            }
        }
    }
}

/** One parameter per row: full label on the left, compact numeric field on the right. */
@Composable
private fun FieldRow(
    field: AdaptiveFieldUi,
    value: String,
    enabled: Boolean,
    shape: Shape,
    onValueChange: (String) -> Unit,
) {
    Surface(
        modifier = Modifier.fillMaxWidth(),
        shape = shape,
        color = MaterialTheme.colorScheme.surfaceContainerHigh,
    ) {
        Row(
            modifier = Modifier.padding(start = 16.dp, end = 10.dp, top = 8.dp, bottom = 8.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            Text(
                field.label,
                modifier = Modifier.weight(1f),
                style = MaterialTheme.typography.bodyMedium,
            )
            OutlinedTextField(
                value = value,
                onValueChange = onValueChange,
                suffix = { Text(field.unit) },
                singleLine = true,
                enabled = enabled,
                keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Number),
                textStyle = MaterialTheme.typography.titleSmall.copy(textAlign = TextAlign.End),
                shape = MaterialTheme.shapes.large,
                modifier = Modifier.width(116.dp),
            )
        }
    }
}

private fun summaryForGroup(group: AdaptiveGroupUi, values: Map<String, String>): String = when (group.id) {
    "smart" -> "上限 ${values["cpu.adaptive.load_cap1"] ?: 45}/${values["cpu.adaptive.load_cap2"] ?: 65}/${values["cpu.adaptive.load_cap3"] ?: 85}/${values["cpu.adaptive.load_cap4"] ?: 100}% · 阈值 ${values["cpu.adaptive.load_t1"] ?: 12}/${values["cpu.adaptive.load_t2"] ?: 35}/${values["cpu.adaptive.load_t3"] ?: 70}%"
    "thermal" -> "启动 ${values["cpu.adaptive.temp_high"] ?: 75}°C · 恢复 ${values["cpu.adaptive.temp_low"] ?: 68}°C"
    "battery" -> "低电量 ${values["cpu.adaptive.batt_low"] ?: 20}% · 临界 ${values["cpu.adaptive.batt_critical"] ?: 10}%"
    else -> "PSI 阈值 ${values["cpu.adaptive.psi_threshold"] ?: 8} · 最低性能上限 ${values["cpu.adaptive.psi_cap"] ?: 85}%"
}
