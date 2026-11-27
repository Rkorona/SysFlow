@file:OptIn(ExperimentalMaterial3ExpressiveApi::class)

package io.sysflow.feature.settings

import androidx.compose.animation.AnimatedVisibility
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.RowScope
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.rounded.KeyboardArrowRight
import androidx.compose.material.icons.rounded.AutoGraph
import androidx.compose.material.icons.rounded.BugReport
import androidx.compose.material.icons.rounded.Check
import androidx.compose.material.icons.rounded.Close
import androidx.compose.material.icons.rounded.DeleteForever
import androidx.compose.material.icons.rounded.GppGood
import androidx.compose.material.icons.rounded.Info
import androidx.compose.material.icons.rounded.Inventory2
import androidx.compose.material.icons.rounded.PowerSettingsNew
import androidx.compose.material.icons.rounded.Restore
import androidx.compose.material.icons.rounded.Save
import androidx.compose.material.icons.rounded.Shield
import androidx.compose.material.icons.rounded.Timelapse
import androidx.compose.material.icons.rounded.Warning
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.ExperimentalMaterial3ExpressiveApi
import androidx.compose.material3.Icon
import androidx.compose.material3.LoadingIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.Slider
import androidx.compose.material3.SliderDefaults
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
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.Shape
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import io.sysflow.core.ui.components.ExpressiveCard
import io.sysflow.core.ui.components.ExpressiveShapes
import io.sysflow.core.ui.components.IconBadge
import io.sysflow.core.ui.components.InfoPill
import io.sysflow.core.ui.components.SectionTitle
import io.sysflow.core.ui.components.WavyMeter
import io.sysflow.core.ui.components.segmentedShape
import org.koin.androidx.compose.koinViewModel

@Composable
fun SettingsScreen(
    onOpenAdaptive: () -> Unit,
    onOpenDiagnostics: () -> Unit,
    modifier: Modifier = Modifier,
    viewModel: SettingsViewModel = koinViewModel(),
) {
    val state by viewModel.state.collectAsStateWithLifecycle()
    val snackbar = remember { SnackbarHostState() }
    var intervalDraft by rememberSaveable { mutableStateOf(10f) }
    var bootDelayDraft by rememberSaveable { mutableStateOf(30f) }
    var showResetConfirm by rememberSaveable { mutableStateOf(false) }
    var showRecoveryRestoreConfirm by rememberSaveable { mutableStateOf(false) }
    var showRecoveryClearConfirm by rememberSaveable { mutableStateOf(false) }

    LaunchedEffect(Unit) { viewModel.load() }
    LaunchedEffect(state.guardIntervalSeconds) { intervalDraft = state.guardIntervalSeconds.toFloat() }
    LaunchedEffect(state.bootDelaySeconds) { bootDelayDraft = state.bootDelaySeconds.toFloat() }
    LaunchedEffect(state.message) {
        val message = state.message ?: return@LaunchedEffect
        snackbar.showSnackbar(message)
        viewModel.dismissMessage()
    }

    val guardSubtitle = when {
        state.guardEnabled && state.guardRunning -> "后台服务运行中：定期修复被系统覆盖的配置并执行自适应采样"
        state.guardEnabled -> "已启用，等待模块服务启动"
        !state.bootApply -> "需要先开启开机应用配置"
        else -> "已关闭：不会自动修复配置漂移，也不会周期执行自适应策略"
    }

    Box(modifier.fillMaxSize()) {
        LazyColumn(
            modifier = Modifier.fillMaxSize(),
            contentPadding = PaddingValues(start = 16.dp, end = 16.dp, top = 14.dp, bottom = 132.dp),
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            item {
                Column(
                    modifier = Modifier.padding(horizontal = 4.dp),
                    verticalArrangement = Arrangement.spacedBy(5.dp),
                ) {
                    Text(
                        "SYSFLOW / PREFERENCES",
                        style = MaterialTheme.typography.labelMedium,
                        color = MaterialTheme.colorScheme.primary,
                    )
                    Text("设置", style = MaterialTheme.typography.headlineLargeEmphasized)
                    Text(
                        "模块行为、性能策略与维护工具",
                        style = MaterialTheme.typography.bodyMedium,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
            }

            // Device health leads the page: it is the one thing that can change what else is allowed.
            item {
                SafetyHero(
                    safeMode = state.safeMode,
                    rescueMarker = state.rescueMarker,
                    bootCount = state.bootCount,
                    bootThreshold = state.bootThreshold,
                    pending = state.pending,
                    onExitSafeMode = viewModel::clearSafeMode,
                )
            }
            if (state.hasRecoveryBackup) {
                item {
                    NoticeCard(
                        icon = Icons.Rounded.Inventory2,
                        title = "发现已隔离的配置备份",
                        body = "模块：${state.recoveryModules.joinToString().ifBlank { "未知" }}。" +
                            "恢复只会放回配置，不会自动执行写入；退出安全模式后再重启应用配置。",
                    ) {
                        Button(
                            onClick = { showRecoveryRestoreConfirm = true },
                            enabled = !state.pending,
                            shapes = ButtonDefaults.shapes(),
                            modifier = Modifier.weight(1f),
                        ) { Text("恢复配置") }
                        TextButton(
                            onClick = { showRecoveryClearConfirm = true },
                            enabled = !state.pending,
                            shapes = ButtonDefaults.shapes(),
                        ) { Text("丢弃") }
                    }
                }
            }
            if (state.hasCorruptConfig) {
                item {
                    NoticeCard(
                        icon = Icons.Rounded.Warning,
                        title = "发现损坏的 config.json 备份",
                        body = "文件已自动隔离为 config.json.corrupt。确认不再需要后可以删除。",
                    ) {
                        TextButton(
                            onClick = viewModel::clearCorruptConfig,
                            enabled = !state.pending,
                            shapes = ButtonDefaults.shapes(),
                        ) { Text("删除备份") }
                    }
                }
            }

            item { SectionTitle("模块行为") }
            item {
                ConnectedList(
                    listOf(
                        { shape ->
                            SwitchRow(
                                shape = shape,
                                icon = Icons.Rounded.PowerSettingsNew,
                                title = "开机应用配置",
                                description = "设备启动完成后，自动恢复已保存的 CPU / GPU 等模块设置",
                                checked = state.bootApply,
                                enabled = !state.pending,
                                onCheckedChange = viewModel::setBootApply,
                            )
                        },
                        { shape ->
                            SwitchRow(
                                shape = shape,
                                icon = Icons.Rounded.GppGood,
                                title = "持续守护",
                                description = guardSubtitle,
                                checked = state.guardEnabled,
                                enabled = !state.pending && state.bootApply && !state.safeMode && !state.rescueMarker,
                                onCheckedChange = viewModel::setGuardEnabled,
                            )
                        },
                        { shape ->
                            SliderRow(
                                shape = shape,
                                icon = Icons.Rounded.Timelapse,
                                title = "守护间隔",
                                valueLabel = "${intervalDraft.toInt()} 秒",
                                hint = "范围 2–120 秒。间隔越短响应越快，但后台采样更频繁。",
                                value = intervalDraft,
                                onValueChange = { intervalDraft = it },
                                valueRange = 2f..120f,
                                steps = 117,
                                enabled = !state.pending,
                                dirty = intervalDraft.toInt() != state.guardIntervalSeconds,
                                saveLabel = "保存守护间隔",
                                onSave = { viewModel.setGuardInterval(intervalDraft.toInt()) },
                            )
                        },
                        { shape ->
                            SliderRow(
                                shape = shape,
                                icon = Icons.Rounded.Restore,
                                title = "开机应用延迟",
                                valueLabel = "${bootDelayDraft.toInt()} 秒",
                                hint = "等待系统稳定后再写入配置，以 5 秒为步进。",
                                value = bootDelayDraft,
                                onValueChange = { bootDelayDraft = it },
                                valueRange = 0f..300f,
                                steps = 59,
                                enabled = !state.pending,
                                dirty = bootDelayDraft.toInt() != state.bootDelaySeconds,
                                saveLabel = "保存开机延迟",
                                onSave = { viewModel.setBootDelay(bootDelayDraft.toInt()) },
                            )
                        },
                    ),
                )
            }

            item { SectionTitle("性能策略") }
            item {
                ConnectedList(
                    listOf(
                        { shape ->
                            NavigationRow(
                                shape = shape,
                                icon = Icons.Rounded.AutoGraph,
                                title = "自适应性能管理",
                                description = "智能负载分档、温度动态收紧、低电量节能与 PSI 性能护栏",
                                onClick = onOpenAdaptive,
                            )
                        },
                    ),
                )
            }

            item { SectionTitle("维护") }
            item {
                ConnectedList(
                    listOf(
                        { shape ->
                            NavigationRow(
                                shape = shape,
                                icon = Icons.Rounded.BugReport,
                                title = "诊断与日志",
                                description = "查看设备能力、配置漂移、CPU 状态与模块运行日志",
                                onClick = onOpenDiagnostics,
                            )
                        },
                        { shape ->
                            NavigationRow(
                                shape = shape,
                                icon = Icons.Rounded.Restore,
                                title = "恢复所有模块默认值",
                                description = "恢复 SysFlow 修改过的设置；无法恢复的节点会保留诊断信息",
                                onClick = { showResetConfirm = true },
                                danger = true,
                            )
                        },
                    ),
                )
            }

            item { SectionTitle("关于") }
            item { AboutCard() }

            if (state.loading) {
                item {
                    Box(
                        Modifier
                            .fillMaxWidth()
                            .padding(12.dp),
                        contentAlignment = Alignment.Center,
                    ) { LoadingIndicator() }
                }
            }
        }
        SnackbarHost(
            snackbar,
            modifier = Modifier
                .align(Alignment.BottomCenter)
                .padding(bottom = 112.dp, start = 16.dp, end = 16.dp),
        )
    }

    if (showRecoveryRestoreConfirm) {
        ConfirmDialog(
            icon = Icons.Rounded.Inventory2,
            title = "恢复隔离配置？",
            text = "将配置备份恢复到活动配置，但不会解除安全模式，也不会立即写入 sysfs。请确认设备稳定后再退出安全模式。",
            confirmLabel = "恢复配置",
            onConfirm = { showRecoveryRestoreConfirm = false; viewModel.restoreRecovery() },
            onDismiss = { showRecoveryRestoreConfirm = false },
        )
    }
    if (showRecoveryClearConfirm) {
        ConfirmDialog(
            icon = Icons.Rounded.DeleteForever,
            title = "丢弃隔离配置备份？",
            text = "备份删除后无法通过 SysFlow 恢复。",
            confirmLabel = "丢弃备份",
            danger = true,
            onConfirm = { showRecoveryClearConfirm = false; viewModel.discardRecovery() },
            onDismiss = { showRecoveryClearConfirm = false },
        )
    }
    if (showResetConfirm) {
        ConfirmDialog(
            icon = Icons.Rounded.Restore,
            title = "恢复所有模块默认值？",
            text = "此操作会尝试恢复 SysFlow 曾修改并记录过原始值的所有设置。无法恢复的项目不会被伪称为成功。",
            confirmLabel = "恢复默认",
            danger = true,
            onConfirm = { showResetConfirm = false; viewModel.resetAll() },
            onDismiss = { showResetConfirm = false },
        )
    }
}

// ---------------------------------------------------------------------------------------------
// Hero / notices
// ---------------------------------------------------------------------------------------------

/** Large tonal status surface: Expressive badge, one headline, boot-failure meter. */
@Composable
private fun SafetyHero(
    safeMode: Boolean,
    rescueMarker: Boolean,
    bootCount: Int,
    bootThreshold: Int,
    pending: Boolean,
    onExitSafeMode: () -> Unit,
) {
    val scheme = MaterialTheme.colorScheme
    val alert = safeMode || rescueMarker
    val container = if (alert) scheme.errorContainer else scheme.primaryContainer
    val content = if (alert) scheme.onErrorContainer else scheme.onPrimaryContainer
    val badgeContainer = if (alert) scheme.error else scheme.primary
    val badgeContent = if (alert) scheme.onError else scheme.onPrimary
    val title = when {
        safeMode -> "安全模式已启用"
        rescueMarker -> "检测到救援标记"
        else -> "设备状态正常"
    }
    val progress = if (bootThreshold > 0) bootCount.toFloat() / bootThreshold else 0f

    ExpressiveCard(containerColor = container, contentColor = content) {
        Row(
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(16.dp),
        ) {
            Box(Modifier.size(64.dp), contentAlignment = Alignment.Center) {
                Surface(
                    modifier = Modifier.matchParentSize(),
                    shape = if (alert) ExpressiveShapes.Clover4 else ExpressiveShapes.Cookie9,
                    color = badgeContainer,
                ) {}
                Icon(
                    imageVector = if (alert) Icons.Rounded.Warning else Icons.Rounded.Shield,
                    contentDescription = null,
                    tint = badgeContent,
                    modifier = Modifier.size(30.dp),
                )
            }
            Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(2.dp)) {
                Text(
                    "恢复与安全",
                    style = MaterialTheme.typography.labelMedium,
                    color = content.copy(alpha = 0.72f),
                )
                Text(title, style = MaterialTheme.typography.titleLargeEmphasized)
            }
        }
        Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Text(
                    "启动失败计数",
                    modifier = Modifier.weight(1f),
                    style = MaterialTheme.typography.bodyMedium,
                    color = content.copy(alpha = 0.8f),
                )
                Text(
                    "$bootCount / $bootThreshold",
                    style = MaterialTheme.typography.titleSmallEmphasized,
                )
            }
            WavyMeter(
                progress = progress,
                color = badgeContainer,
                trackColor = content.copy(alpha = 0.14f),
            )
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                InfoPill(
                    text = if (rescueMarker) "救援标记：存在" else "救援标记：无",
                    container = content.copy(alpha = 0.12f),
                    content = content,
                )
            }
        }
        if (rescueMarker) {
            Text(
                "disable_tuning 救援标记由模块目录或数据目录中的文件控制。请先确认设备稳定，再通过恢复环境或 root shell 删除该标记。",
                style = MaterialTheme.typography.bodySmall,
                color = content.copy(alpha = 0.86f),
            )
        }
        if (safeMode && !rescueMarker) {
            Button(
                onClick = onExitSafeMode,
                enabled = !pending,
                shapes = ButtonDefaults.shapes(),
                modifier = Modifier.fillMaxWidth(),
            ) { Text("退出安全模式") }
        }
    }
}

/** Warning-toned card for recovery leftovers; actions sit in a single row at the bottom. */
@Composable
private fun NoticeCard(
    icon: ImageVector,
    title: String,
    body: String,
    actions: @Composable RowScope.() -> Unit,
) {
    val scheme = MaterialTheme.colorScheme
    ExpressiveCard(
        containerColor = scheme.tertiaryContainer,
        contentColor = scheme.onTertiaryContainer,
    ) {
        Row(
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            IconBadge(
                icon = icon,
                container = scheme.tertiary,
                content = scheme.onTertiary,
            )
            Text(
                title,
                modifier = Modifier.weight(1f),
                style = MaterialTheme.typography.titleMediumEmphasized,
            )
        }
        Text(body, style = MaterialTheme.typography.bodySmall)
        Row(
            modifier = Modifier.fillMaxWidth(),
            horizontalArrangement = Arrangement.spacedBy(8.dp),
            verticalAlignment = Alignment.CenterVertically,
            content = actions,
        )
    }
}

// ---------------------------------------------------------------------------------------------
// Connected list + rows
// ---------------------------------------------------------------------------------------------

/** Rows share one column; [segmentedShape] gives the first/last rows large outer corners. */
@Composable
private fun ConnectedList(rows: List<@Composable (Shape) -> Unit>) {
    Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
        rows.forEachIndexed { index, row -> row(segmentedShape(index, rows.size)) }
    }
}

@Composable
private fun RowSurface(
    shape: Shape,
    modifier: Modifier = Modifier,
    onClick: (() -> Unit)? = null,
    enabled: Boolean = true,
    content: @Composable () -> Unit,
) {
    val color = MaterialTheme.colorScheme.surfaceContainerLow
    if (onClick != null) {
        Surface(
            onClick = onClick,
            enabled = enabled,
            modifier = modifier.fillMaxWidth(),
            shape = shape,
            color = color,
            content = content,
        )
    } else {
        Surface(
            modifier = modifier.fillMaxWidth(),
            shape = shape,
            color = color,
            content = content,
        )
    }
}

@Composable
private fun RowText(
    title: String,
    description: String,
    modifier: Modifier = Modifier,
    titleColor: Color = MaterialTheme.colorScheme.onSurface,
) {
    Column(modifier, verticalArrangement = Arrangement.spacedBy(3.dp)) {
        Text(title, style = MaterialTheme.typography.titleMediumEmphasized, color = titleColor)
        Text(
            description,
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
    }
}

@Composable
private fun SwitchRow(
    shape: Shape,
    icon: ImageVector,
    title: String,
    description: String,
    checked: Boolean,
    enabled: Boolean,
    onCheckedChange: (Boolean) -> Unit,
) {
    RowSurface(shape = shape, onClick = { onCheckedChange(!checked) }, enabled = enabled) {
        Row(
            modifier = Modifier.padding(horizontal = 16.dp, vertical = 14.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(14.dp),
        ) {
            IconBadge(
                icon = icon,
                container = if (checked) MaterialTheme.colorScheme.primaryContainer else MaterialTheme.colorScheme.surfaceContainerHighest,
                content = if (checked) MaterialTheme.colorScheme.onPrimaryContainer else MaterialTheme.colorScheme.onSurfaceVariant,
            )
            RowText(title, description, Modifier.weight(1f))
            Switch(
                checked = checked,
                onCheckedChange = null,
                enabled = enabled,
                thumbContent = {
                    Icon(
                        imageVector = if (checked) Icons.Rounded.Check else Icons.Rounded.Close,
                        contentDescription = null,
                        modifier = Modifier.size(SwitchDefaults.IconSize),
                    )
                },
            )
        }
    }
}

@Composable
private fun NavigationRow(
    shape: Shape,
    icon: ImageVector,
    title: String,
    description: String,
    onClick: () -> Unit,
    danger: Boolean = false,
) {
    val scheme = MaterialTheme.colorScheme
    RowSurface(shape = shape, onClick = onClick) {
        Row(
            modifier = Modifier.padding(horizontal = 16.dp, vertical = 14.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(14.dp),
        ) {
            IconBadge(
                icon = icon,
                container = if (danger) scheme.errorContainer else scheme.secondaryContainer,
                content = if (danger) scheme.onErrorContainer else scheme.onSecondaryContainer,
            )
            RowText(
                title = title,
                description = description,
                modifier = Modifier.weight(1f),
                titleColor = if (danger) scheme.error else scheme.onSurface,
            )
            Icon(
                imageVector = Icons.AutoMirrored.Rounded.KeyboardArrowRight,
                contentDescription = null,
                tint = scheme.onSurfaceVariant,
            )
        }
    }
}

/** Slider row: value shown as a pill, Save only appears once the draft differs from the stored value. */
@Composable
private fun SliderRow(
    shape: Shape,
    icon: ImageVector,
    title: String,
    valueLabel: String,
    hint: String,
    value: Float,
    onValueChange: (Float) -> Unit,
    valueRange: ClosedFloatingPointRange<Float>,
    steps: Int,
    enabled: Boolean,
    dirty: Boolean,
    saveLabel: String,
    onSave: () -> Unit,
) {
    val scheme = MaterialTheme.colorScheme
    RowSurface(shape = shape) {
        Column(
            modifier = Modifier.padding(horizontal = 16.dp, vertical = 14.dp),
            verticalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            Row(
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(14.dp),
            ) {
                IconBadge(icon = icon)
                Text(
                    title,
                    modifier = Modifier.weight(1f),
                    style = MaterialTheme.typography.titleMediumEmphasized,
                )
                InfoPill(
                    text = valueLabel,
                    container = scheme.primaryContainer,
                    content = scheme.onPrimaryContainer,
                )
            }
            Slider(
                value = value,
                onValueChange = onValueChange,
                valueRange = valueRange,
                steps = steps,
                enabled = enabled,
                colors = SliderDefaults.colors(
                    activeTickColor = Color.Transparent,
                    inactiveTickColor = Color.Transparent,
                ),
            )
            Text(
                hint,
                style = MaterialTheme.typography.bodySmall,
                color = scheme.onSurfaceVariant,
            )
            AnimatedVisibility(visible = dirty) {
                Button(
                    onClick = onSave,
                    enabled = enabled,
                    shapes = ButtonDefaults.shapes(),
                    modifier = Modifier.fillMaxWidth(),
                ) {
                    Icon(Icons.Rounded.Save, contentDescription = null, modifier = Modifier.size(ButtonDefaults.IconSize))
                    Spacer(Modifier.width(ButtonDefaults.IconSpacing))
                    Text(saveLabel)
                }
            }
        }
    }
}

@Composable
private fun AboutCard() {
    val scheme = MaterialTheme.colorScheme
    ExpressiveCard {
        Row(
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(16.dp),
        ) {
            Box(Modifier.size(56.dp), contentAlignment = Alignment.Center) {
                Surface(
                    modifier = Modifier.matchParentSize(),
                    shape = ExpressiveShapes.Sunny,
                    color = scheme.tertiaryContainer,
                ) {}
                Icon(
                    Icons.Rounded.Info,
                    contentDescription = null,
                    tint = scheme.onTertiaryContainer,
                    modifier = Modifier.size(26.dp),
                )
            }
            Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(2.dp)) {
                Text("SysFlow", style = MaterialTheme.typography.titleLargeEmphasized)
                Text(
                    "Device manager · Android CPU / GPU 控制与实时监控",
                    style = MaterialTheme.typography.bodySmall,
                    color = scheme.onSurfaceVariant,
                )
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Dialog
// ---------------------------------------------------------------------------------------------

@Composable
private fun ConfirmDialog(
    icon: ImageVector,
    title: String,
    text: String,
    confirmLabel: String,
    onConfirm: () -> Unit,
    onDismiss: () -> Unit,
    danger: Boolean = false,
) {
    AlertDialog(
        onDismissRequest = onDismiss,
        icon = {
            Icon(
                icon,
                contentDescription = null,
                tint = if (danger) MaterialTheme.colorScheme.error else MaterialTheme.colorScheme.primary,
            )
        },
        title = { Text(title) },
        text = { Text(text) },
        confirmButton = {
            Button(
                onClick = onConfirm,
                shapes = ButtonDefaults.shapes(),
                colors = if (danger) {
                    ButtonDefaults.buttonColors(
                        containerColor = MaterialTheme.colorScheme.error,
                        contentColor = MaterialTheme.colorScheme.onError,
                    )
                } else {
                    ButtonDefaults.buttonColors()
                },
            ) { Text(confirmLabel) }
        },
        dismissButton = {
            TextButton(onClick = onDismiss, shapes = ButtonDefaults.shapes()) { Text("取消") }
        },
    )
}
