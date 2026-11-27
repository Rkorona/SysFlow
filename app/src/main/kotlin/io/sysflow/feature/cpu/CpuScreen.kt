@file:OptIn(ExperimentalMaterial3ExpressiveApi::class, ExperimentalMaterial3Api::class, androidx.compose.foundation.layout.ExperimentalLayoutApi::class)

package io.sysflow.feature.cpu

import androidx.compose.material.icons.rounded.ChevronRight
import androidx.compose.material.icons.rounded.Bedtime
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.role
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.animation.core.animateDpAsState
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.material3.ButtonDefaults
import androidx.compose.foundation.background
import androidx.compose.foundation.selection.toggleable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.AutoGraph
import androidx.compose.material.icons.rounded.AccountTree
import androidx.compose.material.icons.rounded.MoreVert
import androidx.compose.material.icons.rounded.Refresh
import androidx.compose.material.icons.rounded.Speed
import androidx.compose.material.icons.rounded.Thermostat
import androidx.compose.material.icons.rounded.Warning
import androidx.compose.material3.Button
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.ExperimentalMaterial3ExpressiveApi
import androidx.compose.material3.FilledTonalButton
import androidx.compose.material3.FilterChip
import androidx.compose.material3.FilterChipDefaults
import androidx.compose.material3.RangeSlider
import androidx.compose.material3.SliderDefaults
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.LoadingIndicator
import androidx.compose.material3.MaterialShapes
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
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
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import kotlin.math.abs
import kotlin.math.roundToInt
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.compose.LocalLifecycleOwner
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.repeatOnLifecycle
import io.sysflow.core.model.CpuSnapshot
import io.sysflow.core.model.CpuLiveMetrics
import io.sysflow.core.model.ITEM_GOV
import io.sysflow.core.model.Schema
import io.sysflow.core.model.clusterLabel
import io.sysflow.core.model.formatKhz
import io.sysflow.core.model.formatTemp
import io.sysflow.core.model.lockedCores
import io.sysflow.core.model.stringOptions
import io.sysflow.core.ui.components.ExpressiveCard
import io.sysflow.core.ui.components.StatusPill
import io.sysflow.core.ui.components.WavyMeter
import io.sysflow.core.ui.components.cardPalette
import io.sysflow.core.ui.theme.sysFlowColors
import kotlinx.coroutines.delay
import kotlinx.coroutines.isActive
import org.koin.androidx.compose.koinViewModel

@Composable
fun CpuScreen(
    onOpenAdaptive: () -> Unit = {},
    onOpenCpuset: () -> Unit = {},
    onOpenScreenOff: () -> Unit = {},
    modifier: Modifier = Modifier,
    viewModel: CpuViewModel = koinViewModel(),
) {
    val state by viewModel.state.collectAsStateWithLifecycle()
    val snackbar = remember { SnackbarHostState() }
    val lifecycleOwner = LocalLifecycleOwner.current

    LaunchedEffect(Unit) {
        lifecycleOwner.repeatOnLifecycle(Lifecycle.State.STARTED) {
            while (isActive) {
                delay(1_500)
                viewModel.refreshLive()
            }
        }
    }
    LaunchedEffect(state.message) {
        val message = state.message ?: return@LaunchedEffect
        snackbar.showSnackbar(message)
        viewModel.dismissMessage()
    }

    Box(modifier.fillMaxSize()) {
        if (state.loading) {
            Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
                LoadingIndicator()
            }
        } else {
            CpuContent(
                state = state,
                onToggleCore = viewModel::toggleCore,
                onSetRange = viewModel::setRange,
                onGovernor = viewModel::setGovernor,
                onApplyCluster = viewModel::applyCluster,
                onRestoreCluster = viewModel::restoreCluster,
                onDiscardCluster = viewModel::discardCluster,
                onReset = viewModel::reset,
                onRebaseline = viewModel::rebaselineDefaults,
                onClearSafe = viewModel::clearSafeMode,
                onOpenAdaptive = onOpenAdaptive,
                onOpenCpuset = onOpenCpuset,
                onOpenScreenOff = onOpenScreenOff,
            )
        }
        SnackbarHost(
            hostState = snackbar,
            modifier = Modifier
                .align(Alignment.BottomCenter)
                .padding(start = 16.dp, end = 16.dp, bottom = if (state.dirty || state.pending) 104.dp else 16.dp),
        )
    }
}

@Composable
private fun CpuContent(
    state: CpuUiState,
    onToggleCore: (Int, Boolean) -> Unit,
    onSetRange: (String, Long, Long) -> Unit,
    onGovernor: (String, String) -> Unit,
    onApplyCluster: (String, Boolean) -> Unit,
    onRestoreCluster: (String) -> Unit,
    onDiscardCluster: (String) -> Unit,
    onReset: () -> Unit,
    onRebaseline: () -> Unit,
    onClearSafe: () -> Unit,
    onOpenAdaptive: () -> Unit,
    onOpenCpuset: () -> Unit,
    onOpenScreenOff: () -> Unit,
) {
    val snapshot = state.snapshot ?: return
    val draft = state.draft ?: return
    val schema = state.schema
    val clusters = snapshot.clusters.keys.sortedBy { it.toIntOrNull() ?: 0 }
    val locks = schema?.lockedCores().orEmpty()
    val safeMode = state.safeMode
    val online = draft.onlineCount
    val total = draft.online.size

    Box(Modifier.fillMaxSize()) {
        LazyColumn(
            modifier = Modifier.fillMaxSize(),
            contentPadding = PaddingValues(
                start = 16.dp,
                end = 16.dp,
                top = 8.dp,
                // The floating apply dock that used to need 96dp of clearance is gone; only the
                // navigation bar inset remains.
                bottom = 24.dp,
            ),
            verticalArrangement = Arrangement.spacedBy(18.dp),
        ) {
            if (safeMode) {
                item { SafeBanner(onClearSafe) }
            }
            item {
                CpuHero(
                    online = online,
                    total = total,
                    temp = snapshot.tempDeciCelsius,
                    dirty = state.dirty,
                    clusters = clusters.size,
                )
            }
            item { AdaptiveSettingsEntry(onClick = onOpenAdaptive) }
            item { ScreenOffEntry(onClick = onOpenScreenOff) }
            item { ProcessGroupsEntry(onClick = onOpenCpuset) }
            item {
                SectionHeading(
                    index = "01",
                    title = "核心矩阵",
                    trailing = "${total.toString().padStart(2, '0')} CORES",
                    onReset = onReset,
                    onRebaseline = onRebaseline,
                    resetEnabled = !state.pending && !safeMode,
                )
            }
            item {
                // Core switches save themselves, so the hint is part of the control: without it a
                // flipped tile looks like an unapplied edit waiting for a button that does not exist.
                Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                    CoreGrid(
                        online = draft.online,
                        snapshot = snapshot,
                        live = state.live,
                        locked = locks,
                        safeMode = safeMode,
                        onToggle = onToggleCore,
                    )
                    Text(
                        "开关立即生效并保存，无需再点应用。",
                        style = MaterialTheme.typography.labelSmall,
                        color = MaterialTheme.colorScheme.outline,
                        modifier = Modifier.padding(horizontal = 2.dp),
                    )
                }
            }
            item {
                SectionHeading(
                    index = "02",
                    title = "调频工作区",
                    trailing = "CLUSTER TUNING",
                )
            }
            if (clusters.isNotEmpty()) {
                item {
                    ClusterCards(
                        clusters = clusters,
                        snapshot = snapshot,
                        state = state,
                        schema = schema,
                        safeMode = safeMode,
                        onSetRange = onSetRange,
                        onGovernor = onGovernor,
                        onApplyCluster = onApplyCluster,
                        onRestore = onRestoreCluster,
                        onDiscard = onDiscardCluster,
                    )
                }
            } else {
                item {
                    ExpressiveCard {
                        Text("设备未提供调频簇信息", style = MaterialTheme.typography.bodyMedium)
                    }
                }
            }
            if (state.mismatch.isNotEmpty()) {
                item {
                    ExpressiveCard(containerColor = MaterialTheme.colorScheme.errorContainer) {
                        Row(horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                            Icon(
                                Icons.Rounded.Warning,
                                contentDescription = null,
                                tint = MaterialTheme.colorScheme.onErrorContainer,
                            )
                            Column {
                                Text(
                                    "部分设置未完全生效",
                                    fontWeight = FontWeight.SemiBold,
                                    color = MaterialTheme.colorScheme.onErrorContainer,
                                )
                                Text(
                                    state.mismatch.joinToString("\n"),
                                    style = MaterialTheme.typography.bodySmall,
                                    color = MaterialTheme.colorScheme.onErrorContainer,
                                )
                            }
                        }
                    }
                }
            }
        }

    }
}

@Composable
private fun AdaptiveSettingsEntry(onClick: () -> Unit) {
    val colors = MaterialTheme.colorScheme
    Surface(
        onClick = onClick,
        shape = MaterialTheme.shapes.extraLarge,
        color = colors.surfaceContainerLow,
        modifier = Modifier.fillMaxWidth(),
    ) {
        Row(
            modifier = Modifier.padding(horizontal = 18.dp, vertical = 15.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(14.dp),
        ) {
            Box(
                modifier = Modifier.size(42.dp).clip(CircleShape).background(colors.primaryContainer),
                contentAlignment = Alignment.Center,
            ) {
                Icon(Icons.Rounded.AutoGraph, contentDescription = null, tint = colors.onPrimaryContainer)
            }
            Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(2.dp)) {
                Text("自适应性能管理", style = MaterialTheme.typography.titleSmallEmphasized)
                Text(
                    "智能负载、温控、低电量与 PSI 性能护栏",
                    style = MaterialTheme.typography.bodySmall,
                    color = colors.onSurfaceVariant,
                )
            }
            Icon(Icons.Rounded.ChevronRight, contentDescription = null, tint = colors.onSurfaceVariant)
        }
    }
}

@Composable
private fun ScreenOffEntry(onClick: () -> Unit) {
    val colors = MaterialTheme.colorScheme
    Surface(
        onClick = onClick,
        shape = MaterialTheme.shapes.extraLarge,
        color = colors.surfaceContainerLow,
        modifier = Modifier.fillMaxWidth(),
    ) {
        Row(
            modifier = Modifier.padding(horizontal = 18.dp, vertical = 15.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(14.dp),
        ) {
            Box(
                modifier = Modifier.size(42.dp).clip(CircleShape).background(colors.tertiaryContainer),
                contentAlignment = Alignment.Center,
            ) {
                Icon(Icons.Rounded.Bedtime, contentDescription = null, tint = colors.onTertiaryContainer)
            }
            Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(2.dp)) {
                Text("熄屏管理", style = MaterialTheme.typography.titleSmallEmphasized)
                Text(
                    "熄屏频率上限与额外关闭的核心",
                    style = MaterialTheme.typography.bodySmall,
                    color = colors.onSurfaceVariant,
                )
            }
            Icon(Icons.Rounded.ChevronRight, contentDescription = null, tint = colors.onSurfaceVariant)
        }
    }
}

@Composable
private fun ProcessGroupsEntry(onClick: () -> Unit) {
    val colors = MaterialTheme.colorScheme
    Surface(
        onClick = onClick,
        shape = MaterialTheme.shapes.extraLarge,
        color = colors.surfaceContainerLow,
        modifier = Modifier.fillMaxWidth(),
    ) {
        Row(
            modifier = Modifier.padding(horizontal = 18.dp, vertical = 15.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(14.dp),
        ) {
            Box(
                modifier = Modifier.size(42.dp).clip(CircleShape).background(colors.secondaryContainer),
                contentAlignment = Alignment.Center,
            ) {
                Icon(Icons.Rounded.AccountTree, contentDescription = null, tint = colors.onSecondaryContainer)
            }
            Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(2.dp)) {
                Text("进程组", style = MaterialTheme.typography.titleSmallEmphasized)
                Text(
                    "限制顶层、前台与后台进程可使用的 CPU 核心",
                    style = MaterialTheme.typography.bodySmall,
                    color = colors.onSurfaceVariant,
                )
            }
            Icon(Icons.Rounded.ChevronRight, contentDescription = null, tint = colors.onSurfaceVariant)
        }
    }
}

@Composable
private fun CpuHero(
    online: Int,
    total: Int,
    temp: Long?,
    dirty: Boolean,
    clusters: Int,
) {
    val colors = MaterialTheme.colorScheme
    Surface(
        shape = MaterialTheme.shapes.extraLarge,
        color = colors.primaryContainer,
        contentColor = colors.onPrimaryContainer,
        tonalElevation = 2.dp,
        modifier = Modifier.fillMaxWidth(),
    ) {
        Box {
            Box(
                modifier = Modifier
                    .align(Alignment.TopEnd)
                    .padding(top = 12.dp, end = 8.dp)
                    .size(124.dp)
                    .clip(MaterialShapes.Cookie9Sided.toShape())
                    .background(colors.onPrimaryContainer.copy(alpha = 0.045f)),
            )
            Column(
                modifier = Modifier.padding(20.dp),
                verticalArrangement = Arrangement.spacedBy(16.dp),
            ) {
                Row(verticalAlignment = Alignment.Top) {
                    Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(5.dp)) {
                        Text(
                            "CPU CONTROL  /  LIVE",
                            style = MaterialTheme.typography.labelSmall,
                            color = colors.onPrimaryContainer.copy(alpha = 0.68f),
                        )
                        Text("核心与频率", style = MaterialTheme.typography.headlineMediumEmphasized)
                        Text(
                            "实时掌控处理器运行状态",
                            style = MaterialTheme.typography.bodySmall,
                            color = colors.onPrimaryContainer.copy(alpha = 0.76f),
                        )
                    }
                    Surface(
                        shape = MaterialShapes.Cookie9Sided.toShape(),
                        color = colors.surface.copy(alpha = 0.64f),
                        contentColor = colors.onSurface,
                    ) {
                        Column(
                            modifier = Modifier.padding(horizontal = 12.dp, vertical = 10.dp),
                            horizontalAlignment = Alignment.CenterHorizontally,
                            verticalArrangement = Arrangement.spacedBy(3.dp),
                        ) {
                            Icon(Icons.Rounded.Thermostat, contentDescription = null, modifier = Modifier.size(20.dp))
                            Text(formatTemp(temp), style = MaterialTheme.typography.labelMedium, fontWeight = FontWeight.Bold)
                        }
                    }
                }
                Row(
                    modifier = Modifier.fillMaxWidth(),
                    verticalAlignment = Alignment.Bottom,
                    horizontalArrangement = Arrangement.spacedBy(16.dp),
                ) {
                    Row(
                        modifier = Modifier.weight(1f),
                        verticalAlignment = Alignment.Bottom,
                        horizontalArrangement = Arrangement.spacedBy(10.dp),
                    ) {
                        Text(
                            online.toString().padStart(2, '0'),
                            style = MaterialTheme.typography.displayMediumEmphasized,
                        )
                        Column(Modifier.padding(bottom = 6.dp)) {
                            Text("/ ${total.toString().padStart(2, '0')}", style = MaterialTheme.typography.titleMedium)
                            Text("在线核心", style = MaterialTheme.typography.labelSmall, color = colors.onPrimaryContainer.copy(alpha = 0.72f))
                        }
                    }
                    StatusPill(
                        "$clusters 个调频簇",
                        container = colors.surface.copy(alpha = 0.58f),
                        content = colors.onSurface,
                        modifier = Modifier.padding(bottom = 8.dp),
                    )
                }
                WavyMeter(
                    progress = if (total > 0) online.toFloat() / total else 0f,
                    // `primary` is intentionally dark in the light scheme and nearly
                    // blends into the deep-teal primaryContainer. Use the tested accent
                    // role designed specifically for drawing on this container instead.
                    color = MaterialTheme.sysFlowColors.containerAccent,
                    trackColor = colors.onPrimaryContainer.copy(alpha = 0.22f),
                )
                Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    StatusPill(
                        "读回校验",
                        container = colors.surface.copy(alpha = 0.52f),
                        content = colors.onSurface,
                    )
                    StatusPill(
                        if (dirty) "待应用修改" else "配置已同步",
                        container = colors.surface.copy(alpha = 0.52f),
                        content = colors.onSurface,
                    )
                }
            }
        }
    }
}

@Composable
private fun SectionHeading(
    index: String,
    title: String,
    trailing: String,
    onReset: (() -> Unit)? = null,
    onRebaseline: (() -> Unit)? = null,
    resetEnabled: Boolean = true,
) {
    var expanded by remember { mutableStateOf(false) }
    Row(verticalAlignment = Alignment.CenterVertically) {
        Surface(
            shape = MaterialShapes.Cookie9Sided.toShape(),
            color = MaterialTheme.colorScheme.secondaryContainer,
            contentColor = MaterialTheme.colorScheme.onSecondaryContainer,
        ) {
            Text(
                index,
                modifier = Modifier.padding(horizontal = 9.dp, vertical = 6.dp),
                style = MaterialTheme.typography.labelMedium,
                fontWeight = FontWeight.Bold,
            )
        }
        Text(
            title,
            modifier = Modifier.padding(start = 9.dp).weight(1f),
            style = MaterialTheme.typography.titleMediumEmphasized,
        )
        Text(
            trailing,
            style = MaterialTheme.typography.labelSmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        if (onReset != null) {
            Box {
                IconButton(onClick = { expanded = true }, enabled = resetEnabled) {
                    Icon(Icons.Rounded.MoreVert, contentDescription = "CPU 更多操作")
                }
                DropdownMenu(expanded = expanded, onDismissRequest = { expanded = false }) {
                    DropdownMenuItem(
                        text = { Text("恢复 CPU 默认设置") },
                        leadingIcon = { Icon(Icons.Rounded.Refresh, contentDescription = null) },
                        enabled = resetEnabled,
                        onClick = {
                            expanded = false
                            onReset()
                        },
                    )
                    if (onRebaseline != null) {
                        // Separated from 恢复默认 on purpose: this does not touch the running
                        // frequencies, it changes what 恢复默认 will restore from now on.
                        DropdownMenuItem(
                            text = { Text("写回硬件下限为默认") },
                            leadingIcon = { Icon(Icons.Rounded.Speed, contentDescription = null) },
                            enabled = resetEnabled,
                            onClick = {
                                expanded = false
                                onRebaseline()
                            },
                        )
                    }
                }
            }
        }
    }
}

@Composable
private fun CoreGrid(
    online: List<Int>,
    snapshot: CpuSnapshot,
    live: CpuLiveMetrics,
    locked: Set<Int>,
    safeMode: Boolean,
    onToggle: (Int, Boolean) -> Unit,
) {
    Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
        online.indices.toList().chunked(4).forEach { rowIndices ->
            Row(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                rowIndices.forEach { index ->
                    CoreTile(
                        index = index,
                        isOnline = online[index] == 1,
                        usage = live.coreUsage[index],
                        frequency = live.coreFreqKhz[index] ?: snapshot.curFreq[index.toString()],
                        locked = index == 0 || index in locked,
                        safeMode = safeMode,
                        onToggle = { onToggle(index, it) },
                        modifier = Modifier.weight(1f),
                    )
                }
                repeat(4 - rowIndices.size) {
                    Spacer(Modifier.weight(1f))
                }
            }
        }
    }
}

@Composable
private fun CoreTile(
    index: Int,
    isOnline: Boolean,
    usage: Float?,
    frequency: Long?,
    locked: Boolean,
    safeMode: Boolean,
    onToggle: (Boolean) -> Unit,
    modifier: Modifier = Modifier,
) {
    val colors = MaterialTheme.colorScheme
    val motion = MaterialTheme.motionScheme
    val displayedUsage = (usage ?: 0f).coerceIn(0f, 1f)
    val fill by animateFloatAsState(
        targetValue = if (isOnline) displayedUsage else 0f,
        animationSpec = motion.defaultEffectsSpec(),
        label = "cpuCoreUsageFill",
    )
    val corner by animateDpAsState(
        targetValue = if (isOnline) 20.dp else 38.dp,
        animationSpec = motion.defaultSpatialSpec(),
        label = "cpuCoreCorner",
    )
    val tileShape = RoundedCornerShape(corner)
    val fillColor = MaterialTheme.sysFlowColors.containerTint
    val usageLabel = "${(displayedUsage * 100f).roundToInt()}%"

    Box(
        modifier = modifier
            .height(76.dp)
            .clip(tileShape)
            .background(colors.surfaceContainerHigh)
            .toggleable(
                value = isOnline,
                enabled = !safeMode && !locked,
                role = Role.Switch,
                onValueChange = onToggle,
            ),
        contentAlignment = Alignment.Center,
    ) {
        Box(
            modifier = Modifier
                .align(Alignment.BottomCenter)
                .fillMaxWidth()
                .fillMaxHeight(fill)
                .background(fillColor),
        )
        Column(
            horizontalAlignment = Alignment.CenterHorizontally,
            verticalArrangement = Arrangement.spacedBy(1.dp),
        ) {
            Text(
                text = if (isOnline) {
                    "CPU $index · $usageLabel"
                } else {
                    "CPU $index"
                },
                style = MaterialTheme.typography.labelSmall,
                color = if (isOnline) colors.onSurfaceVariant else colors.outline,
                maxLines = 1,
            )
            Text(
                text = if (isOnline) formatKhz(frequency) else "OFF",
                style = MaterialTheme.typography.labelLargeEmphasized,
                color = if (isOnline) colors.onSurface else colors.outline,
                maxLines = 1,
                softWrap = false,
            )
        }
    }
}

@Composable
private fun ClusterCards(
    clusters: List<String>,
    snapshot: CpuSnapshot,
    state: CpuUiState,
    schema: Schema?,
    safeMode: Boolean,
    onSetRange: (String, Long, Long) -> Unit,
    onGovernor: (String, String) -> Unit,
    onApplyCluster: (String, Boolean) -> Unit,
    onRestore: (String) -> Unit,
    onDiscard: (String) -> Unit,
) {
    val draft = state.draft ?: return
    Column(verticalArrangement = Arrangement.spacedBy(14.dp)) {
        clusters.forEachIndexed { index, id ->
            val cpus = snapshot.clusters[id].orEmpty()
            val palette = cardPalette(index)
            val chipContainer = palette.content.copy(alpha = 0.12f)
            val current = cpus.firstNotNullOfOrNull { cpu ->
                state.live.coreFreqKhz[cpu]?.takeIf { it > 0L }
            } ?: snapshot.clusterCurrent(id)
            val draftMin = draft.minFreq[id] ?: snapshot.staticMin(id)
            val draftMax = draft.maxFreq[id] ?: snapshot.staticMax(id)
            val effectiveMin = snapshot.minFreq[id]
            val effectiveMax = snapshot.maxFreq[id]
            val staticMin = snapshot.staticMin(id)
            val staticMax = snapshot.staticMax(id)
            val adaptiveCapped = snapshot.adaptiveEnabled && !snapshot.screenOffActive &&
                effectiveMax != null && staticMax > 0L && effectiveMax < staticMax
            val screenCapped = snapshot.screenOffActive && effectiveMax != null &&
                staticMax > 0L && effectiveMax < staticMax

            // cpu.max_freq options in the schema are a union across every policy. Never use them
            // as a fallback here: a prime-cluster frequency offered for a little cluster is
            // rejected by the module. The snapshot's per-policy table is authoritative.
            val frequencySteps = remember(id, snapshot.frequencyTable) {
                snapshot.frequencyTable[id].orEmpty().filter { it > 0L }.distinct().sorted()
            }
            val loIndex = frequencySteps.nearestIndex(draftMin)
            val hiIndex = frequencySteps.nearestIndex(draftMax).coerceAtLeast(loIndex)
            val currentGovernor = draft.governor[id] ?: snapshot.governor[id].orEmpty()
            val governors = snapshot.governorTable[id].orEmpty().ifEmpty {
                schema?.stringOptions(ITEM_GOV).orEmpty()
            }.let { options ->
                (options + currentGovernor).filter(String::isNotBlank).distinct()
            }
            val clusterDirty = draft.isClusterDirty(snapshot, id)

            Column(
                modifier = Modifier
                    .fillMaxWidth()
                    .clip(
                        RoundedCornerShape(
                            topStart = 28.dp,
                            topEnd = 44.dp,
                            bottomEnd = 28.dp,
                            bottomStart = 44.dp,
                        ),
                    )
                    .background(palette.container)
                    .padding(horizontal = 22.dp, vertical = 20.dp),
                verticalArrangement = Arrangement.spacedBy(12.dp),
            ) {
                Row(
                    modifier = Modifier.fillMaxWidth(),
                    horizontalArrangement = Arrangement.SpaceBetween,
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(2.dp)) {
                        Text(
                            text = clusterLabel(id, cpus),
                            style = MaterialTheme.typography.titleMediumEmphasized,
                            color = palette.content,
                        )
                        Text(
                            "${cpus.size} 个逻辑核心 · ${currentGovernor.ifBlank { "未读取 Governor" }}",
                            style = MaterialTheme.typography.bodySmall,
                            color = palette.content.copy(alpha = 0.78f),
                        )
                    }
                    Text(
                        formatKhz(current),
                        style = MaterialTheme.typography.headlineSmallEmphasized,
                        color = palette.content,
                    )
                }

                when {
                    adaptiveCapped -> StatusPill(
                        text = "动态封顶中",
                        container = chipContainer,
                        content = palette.content,
                    )
                    screenCapped -> StatusPill(
                        text = "熄屏限频中",
                        container = chipContainer,
                        content = palette.content,
                    )
                }

                Column(verticalArrangement = Arrangement.spacedBy(3.dp)) {
                    ClusterInfoLine(
                        label = "静态配置",
                        value = if (staticMin > 0L && staticMax > 0L) {
                            "${formatKhz(staticMin)} – ${formatKhz(staticMax)}"
                        } else "—",
                        color = palette.content,
                    )
                    ClusterInfoLine(
                        label = "当前生效",
                        value = if (effectiveMin != null && effectiveMax != null) {
                            "${formatKhz(effectiveMin)} – ${formatKhz(effectiveMax)}"
                        } else "—",
                        color = palette.content,
                    )
                }

                if (frequencySteps.size >= 2) {
                    Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.SpaceBetween) {
                        ClusterValueColumn("最低频率", formatKhz(draftMin), palette.content, Alignment.Start)
                        ClusterValueColumn("最高频率", formatKhz(draftMax), palette.content, Alignment.End)
                    }
                    RangeSlider(
                        value = loIndex.toFloat()..hiIndex.toFloat(),
                        onValueChange = { range ->
                            val low = frequencySteps[range.start.roundToInt().coerceIn(frequencySteps.indices)]
                            val high = frequencySteps[range.endInclusive.roundToInt().coerceIn(frequencySteps.indices)]
                            onSetRange(id, low, high)
                        },
                        valueRange = 0f..frequencySteps.lastIndex.toFloat(),
                        steps = (frequencySteps.size - 2).coerceAtLeast(0),
                        enabled = !safeMode && !state.pending,
                        colors = SliderDefaults.colors(
                            thumbColor = palette.accent,
                            activeTrackColor = palette.accent,
                            inactiveTrackColor = palette.content.copy(alpha = 0.18f),
                            activeTickColor = palette.container,
                            inactiveTickColor = palette.content.copy(alpha = 0.28f),
                        ),
                    )
                    Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.SpaceBetween) {
                        Text(
                            formatKhz(frequencySteps.first()),
                            style = MaterialTheme.typography.labelSmall,
                            color = palette.content.copy(alpha = 0.74f),
                        )
                        Text(
                            "硬件频率范围",
                            style = MaterialTheme.typography.labelSmall,
                            color = palette.content.copy(alpha = 0.74f),
                        )
                        Text(
                            formatKhz(frequencySteps.last()),
                            style = MaterialTheme.typography.labelSmall,
                            color = palette.content.copy(alpha = 0.74f),
                        )
                    }
                } else {
                    Text(
                        "模块未提供该调频簇频率表",
                        style = MaterialTheme.typography.bodySmall,
                        color = palette.content.copy(alpha = 0.8f),
                    )
                }

                Text("调速器", style = MaterialTheme.typography.labelLarge, color = palette.content)
                FlowRow(
                    horizontalArrangement = Arrangement.spacedBy(8.dp),
                    verticalArrangement = Arrangement.spacedBy(8.dp),
                ) {
                    governors.forEach { governor ->
                        FilterChip(
                            selected = currentGovernor == governor,
                            onClick = { onGovernor(id, governor) },
                            enabled = frequencySteps.size >= 2 && !safeMode && !state.pending,
                            label = { Text(governor) },
                            colors = FilterChipDefaults.filterChipColors(
                                containerColor = chipContainer,
                                labelColor = palette.content,
                                selectedContainerColor = palette.accent,
                                selectedLabelColor = palette.container,
                            ),
                        )
                    }
                }

                Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    // Dual state, one button: while edits are pending it discards them locally, and
                    // once clean it writes the factory defaults. Writing on a dirty cluster would
                    // silently do the opposite of what the label promised.
                    FilledTonalButton(
                        onClick = { if (clusterDirty) onDiscard(id) else onRestore(id) },
                        enabled = frequencySteps.size >= 2 && !safeMode && !state.pending,
                        shapes = ButtonDefaults.shapes(),
                        colors = ButtonDefaults.filledTonalButtonColors(
                            containerColor = chipContainer,
                            contentColor = palette.content,
                        ),
                        modifier = Modifier.weight(1f),
                    ) {
                        Text(if (clusterDirty) "放弃修改" else "恢复默认")
                    }
                    Button(
                        onClick = { onApplyCluster(id, true) },
                        enabled = frequencySteps.size >= 2 && clusterDirty && !safeMode && !state.pending,
                        shapes = ButtonDefaults.shapes(),
                        colors = ButtonDefaults.buttonColors(
                            containerColor = palette.accent,
                            contentColor = palette.container,
                        ),
                        modifier = Modifier.weight(1f),
                    ) {
                        // The only apply path left: single cluster, persisted. Naming the scope is
                        // what distinguishes it from the removed "apply everything" button.
                        Text("应用本簇")
                    }
                }
                if (clusterDirty) {
                    StatusPill(
                        text = "本簇存在未应用更改",
                        container = palette.accent.copy(alpha = 0.16f),
                        content = palette.content,
                    )
                }
            }
        }
    }
}

private fun List<Long>.nearestIndex(target: Long): Int =
    indices.minByOrNull { index -> abs(this[index] - target) } ?: 0

@Composable
private fun ClusterInfoLine(label: String, value: String, color: androidx.compose.ui.graphics.Color) {
    Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.SpaceBetween) {
        Text(label, style = MaterialTheme.typography.bodyMedium, color = color.copy(alpha = 0.8f))
        Text(value, style = MaterialTheme.typography.titleSmallEmphasized, color = color)
    }
}

@Composable
private fun ClusterValueColumn(
    label: String,
    value: String,
    color: androidx.compose.ui.graphics.Color,
    alignment: Alignment.Horizontal,
) {
    Column(horizontalAlignment = alignment, verticalArrangement = Arrangement.spacedBy(2.dp)) {
        Text(label, style = MaterialTheme.typography.labelMedium, color = color.copy(alpha = 0.76f))
        Text(value, style = MaterialTheme.typography.headlineSmallEmphasized, color = color)
    }
}

@Composable
private fun SafeBanner(onClearSafe: () -> Unit) {
    val colors = MaterialTheme.colorScheme
    ExpressiveCard(containerColor = colors.errorContainer, contentColor = colors.onErrorContainer) {
        Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
            Icon(Icons.Rounded.Warning, contentDescription = null, tint = colors.onErrorContainer)
            Column(Modifier.weight(1f)) {
                Text("安全模式中，调节已禁用", style = MaterialTheme.typography.titleSmall, color = colors.onErrorContainer)
                Text(
                    "确认设备稳定后退出安全模式再调节。",
                    style = MaterialTheme.typography.bodySmall,
                    color = colors.onErrorContainer.copy(alpha = 0.8f),
                )
            }
            FilledTonalButton(onClick = onClearSafe) { Text("退出") }
        }
    }
}
