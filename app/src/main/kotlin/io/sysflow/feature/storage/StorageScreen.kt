@file:OptIn(
    ExperimentalMaterial3Api::class,
    ExperimentalMaterial3ExpressiveApi::class,
    ExperimentalLayoutApi::class,
)

package io.sysflow.feature.storage

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.asPaddingValues
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.navigationBars
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.Close
import androidx.compose.material.icons.rounded.DeleteForever
import androidx.compose.material.icons.rounded.ErrorOutline
import androidx.compose.material.icons.rounded.FolderOpen
import androidx.compose.material.icons.rounded.Info
import androidx.compose.material.icons.rounded.Refresh
import androidx.compose.material.icons.rounded.Speed
import androidx.compose.material.icons.rounded.Storage
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.ExperimentalMaterial3ExpressiveApi
import androidx.compose.material3.FilterChip
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.LoadingIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.unit.dp
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.compose.LocalLifecycleOwner
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.repeatOnLifecycle
import io.sysflow.core.ui.components.ExpressiveCard
import io.sysflow.core.ui.components.KeyValueRow
import io.sysflow.core.ui.components.LineChart
import io.sysflow.core.ui.components.SectionLabel
import io.sysflow.core.ui.components.StatusPill
import io.sysflow.core.ui.components.WavyMeter
import kotlinx.coroutines.delay
import kotlinx.coroutines.isActive
import org.koin.androidx.compose.koinViewModel

/**
 * Storage page: how full the partition is, how fast it is moving data, which queue policy is active,
 * and the two maintenance actions (TRIM and a bounded directory scan).
 *
 * `SysFlowApp` wraps this route in `FlexibleSubPage`, so no page heading is drawn here. The page
 * polls only while STARTED: a snapshot + one `storage io` reading every two seconds. Nothing runs in
 * the background.
 */
@Composable
fun StorageScreen(viewModel: StorageViewModel = koinViewModel()) {
    val state by viewModel.state.collectAsStateWithLifecycle()
    val lifecycleOwner = LocalLifecycleOwner.current
    val bottomInset = WindowInsets.navigationBars.asPaddingValues().calculateBottomPadding()
    var confirm by remember { mutableStateOf<StorageConfirm?>(null) }

    LaunchedEffect(Unit) {
        lifecycleOwner.repeatOnLifecycle(Lifecycle.State.STARTED) {
            while (isActive) {
                viewModel.refresh()
                viewModel.pollIo()
                delay(2_000)
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
        Box(Modifier.weight(1f)) {
            val snapshot = state.snapshot
            when {
                snapshot == null && state.loading -> Box(Modifier.fillMaxSize(), Alignment.Center) {
                    LoadingIndicator()
                }
                snapshot == null -> Box(Modifier.fillMaxSize(), Alignment.Center) {
                    EmptyState(
                        text = state.error ?: "设备没有提供 /data 容量信息，或模块尚未加载。",
                        onRetry = viewModel::refresh,
                    )
                }
                else -> LazyColumn(
                    modifier = Modifier.fillMaxSize(),
                    contentPadding = PaddingValues(
                        start = 16.dp,
                        end = 16.dp,
                        top = 8.dp,
                        bottom = 24.dp + bottomInset,
                    ),
                    verticalArrangement = Arrangement.spacedBy(12.dp),
                ) {
                    item(key = "hero") { CapacityHero(snapshot) }
                    snapshot.sdcard?.let { sdcard ->
                        item(key = "sdcard") {
                            CapacityCard(title = "外置 / 模拟存储", capacity = sdcard)
                        }
                    }
                    item(key = "io") {
                        IoCard(
                            io = state.io,
                            loading = state.ioLoading,
                            onRefresh = viewModel::pollIo,
                        )
                    }
                    item(key = "history") {
                        HistoryCard(
                            history = state.history,
                            loading = state.historyLoading,
                            onRefresh = viewModel::loadHistory,
                        )
                    }
                    item(key = "device") {
                        DeviceCard(
                            state = state,
                            onScheduler = viewModel::setScheduler,
                            onReadAhead = viewModel::setReadAhead,
                        )
                    }
                    item(key = "maintenance") {
                        MaintenanceCard(
                            state = state,
                            onTrim = { confirm = trimConfirm { viewModel.trim() } },
                            onAnalyze = viewModel::analyzeUsage,
                        )
                    }
                }
            }
        }
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
private fun CapacityHero(snapshot: StorageSnapshot) {
    val scheme = MaterialTheme.colorScheme
    Column(
        modifier = Modifier
            .fillMaxWidth()
            .clip(MaterialTheme.shapes.extraLarge)
            .background(scheme.primaryContainer)
            .padding(20.dp),
        verticalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Icon(Icons.Rounded.Storage, contentDescription = null, tint = scheme.onPrimaryContainer)
            Spacer(Modifier.width(10.dp))
            Column(Modifier.weight(1f)) {
                Text(
                    "内部存储 /data",
                    style = MaterialTheme.typography.titleMedium,
                    color = scheme.onPrimaryContainer,
                )
                Text(
                    "${formatStorageKb(snapshot.capacity.usedKb)} / ${formatStorageKb(snapshot.capacity.totalKb)}",
                    style = MaterialTheme.typography.headlineSmall,
                    color = scheme.onPrimaryContainer,
                )
            }
            StatusPill(
                formatStoragePct(snapshot.capacity.usedPct),
                container = scheme.onPrimaryContainer,
                content = scheme.primaryContainer,
            )
        }
        WavyMeter(
            progress = snapshot.capacity.usedFraction,
            modifier = Modifier.fillMaxWidth(),
            color = scheme.onPrimaryContainer,
        )
        Row(horizontalArrangement = Arrangement.spacedBy(18.dp)) {
            CapacityLabel("可用", formatStorageKb(snapshot.capacity.availKb), scheme.onPrimaryContainer)
            CapacityLabel("空闲", formatStorageKb(snapshot.capacity.freeKb), scheme.onPrimaryContainer)
            snapshot.device.name?.let {
                CapacityLabel("设备", it, scheme.onPrimaryContainer)
            }
        }
    }
}

@Composable
private fun CapacityLabel(label: String, value: String, color: Color) {
    Column {
        Text(label, style = MaterialTheme.typography.labelSmall, color = color.copy(alpha = 0.75f))
        Text(value, style = MaterialTheme.typography.bodyMedium, color = color)
    }
}

@Composable
private fun CapacityCard(title: String, capacity: StorageCapacity) {
    ExpressiveCard {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Column(Modifier.weight(1f)) {
                Text(title, style = MaterialTheme.typography.titleMedium)
                Text(
                    "${formatStorageKb(capacity.usedKb)} / ${formatStorageKb(capacity.totalKb)}",
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
            StatusPill(formatStoragePct(capacity.usedPct))
        }
        WavyMeter(progress = capacity.usedFraction, modifier = Modifier.fillMaxWidth())
    }
}

@Composable
private fun IoCard(io: StorageIo?, loading: Boolean, onRefresh: () -> Unit) {
    ExpressiveCard {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Icon(Icons.Rounded.Speed, contentDescription = null, tint = MaterialTheme.colorScheme.primary)
            Spacer(Modifier.width(8.dp))
            Text("实时 I/O", style = MaterialTheme.typography.titleMedium, modifier = Modifier.weight(1f))
            if (loading) {
                LoadingIndicator(modifier = Modifier.size(20.dp))
            } else {
                IconButton(onClick = onRefresh) {
                    Icon(Icons.Rounded.Refresh, contentDescription = "刷新 I/O")
                }
            }
        }
        if (io == null) {
            Text(
                "正在采样…（间隔 1 秒）",
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
            return@ExpressiveCard
        }
        Row(horizontalArrangement = Arrangement.spacedBy(24.dp)) {
            IoMetric("读取", formatStorageRate(io.readKb), "${formatIops(io.readIops)} IOPS")
            IoMetric("写入", formatStorageRate(io.writeKb), "${formatIops(io.writeIops)} IOPS")
            IoMetric("忙碌", String.format(java.util.Locale.ROOT, "%.1f%%", io.utilPct), "${io.intervalMs} ms")
        }
        if (io.totalReadKb != null && io.totalWriteKb != null) {
            Text(
                "自开机累计：读 ${formatStorageKb(io.totalReadKb)} · 写 ${formatStorageKb(io.totalWriteKb)}",
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
    }
}

@Composable
private fun IoMetric(label: String, value: String, detail: String) {
    Column {
        Text(label, style = MaterialTheme.typography.labelSmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
        Text(value, style = MaterialTheme.typography.titleMedium)
        Text(detail, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
    }
}

@Composable
private fun HistoryCard(history: StorageHistory, loading: Boolean, onRefresh: () -> Unit) {
    val scheme = MaterialTheme.colorScheme
    ExpressiveCard {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Text("24 小时吞吐", style = MaterialTheme.typography.titleMedium, modifier = Modifier.weight(1f))
            if (loading) {
                LoadingIndicator(modifier = Modifier.size(20.dp))
            } else {
                IconButton(onClick = onRefresh) {
                    Icon(Icons.Rounded.Refresh, contentDescription = "刷新历史")
                }
            }
        }
        if (history.isEmpty) {
            Text(
                "还没有历史采样点。守护进程每 5 分钟记录一次；刚升级或 daemon 未运行时曲线为空。",
                style = MaterialTheme.typography.bodySmall,
                color = scheme.onSurfaceVariant,
            )
        } else {
            Text("读取", style = MaterialTheme.typography.labelMedium, color = scheme.onSurfaceVariant)
            LineChart(points = history.read, color = scheme.primary, height = 84.dp)
            Spacer(Modifier.height(4.dp))
            Text("写入", style = MaterialTheme.typography.labelMedium, color = scheme.onSurfaceVariant)
            LineChart(points = history.write, color = scheme.tertiary, height = 84.dp)
        }
    }
}

@Composable
private fun DeviceCard(
    state: StorageViewModel.UiState,
    onScheduler: (String) -> Unit,
    onReadAhead: (Int) -> Unit,
) {
    val snapshot = state.snapshot ?: return
    val scheme = MaterialTheme.colorScheme
    ExpressiveCard {
        SectionLabel("设备与队列")
        KeyValueRow("设备", snapshot.device.name ?: "—")
        KeyValueRow("型号", snapshot.device.model ?: "—")
        snapshot.device.sizeKb?.let { KeyValueRow("设备容量", formatStorageKb(it)) }
        KeyValueRow("介质", if (snapshot.device.rotational) "机械盘" else "闪存")
        snapshot.device.healthPct?.let { KeyValueRow("剩余寿命（JEDEC 估算）", "$it%") }

        if (state.options.scheduler.isNotEmpty()) {
            Text("I/O 调度器", style = MaterialTheme.typography.labelMedium, color = scheme.onSurfaceVariant)
            FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                state.options.scheduler.forEach { option ->
                    FilterChip(
                        selected = snapshot.scheduler == option,
                        onClick = { if (!state.saving) onScheduler(option) },
                        enabled = !state.saving,
                        label = { Text(option) },
                    )
                }
            }
        }
        if (state.options.readAhead.isNotEmpty()) {
            Text("预读（kB）", style = MaterialTheme.typography.labelMedium, color = scheme.onSurfaceVariant)
            FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                state.options.readAhead.forEach { option ->
                    FilterChip(
                        selected = snapshot.readAheadKb == option,
                        onClick = { if (!state.saving) onReadAhead(option) },
                        enabled = !state.saving,
                        label = { Text(option.toString()) },
                    )
                }
            }
        }
        if (state.saving) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                LoadingIndicator(modifier = Modifier.size(16.dp))
                Spacer(Modifier.width(8.dp))
                Text("正在写入并读回…", style = MaterialTheme.typography.bodySmall)
            }
        }
        Text(
            "调度器与预读会持久化，开机由模块重新应用；ROM 改回时守护进程会修复。",
            style = MaterialTheme.typography.bodySmall,
            color = scheme.onSurfaceVariant,
        )
    }
}

@Composable
private fun MaintenanceCard(
    state: StorageViewModel.UiState,
    onTrim: () -> Unit,
    onAnalyze: () -> Unit,
) {
    val scheme = MaterialTheme.colorScheme
    ExpressiveCard {
        SectionLabel("维护")
        Row(horizontalArrangement = Arrangement.spacedBy(12.dp), verticalAlignment = Alignment.CenterVertically) {
            Button(
                onClick = onTrim,
                enabled = state.actionRunning == null,
                colors = ButtonDefaults.buttonColors(
                    containerColor = scheme.errorContainer,
                    contentColor = scheme.onErrorContainer,
                ),
            ) {
                Icon(Icons.Rounded.DeleteForever, contentDescription = null, modifier = Modifier.size(18.dp))
                Spacer(Modifier.width(6.dp))
                Text("TRIM /data")
            }
            Text(
                "一次性 fstrim，不会开机重放",
                style = MaterialTheme.typography.bodySmall,
                color = scheme.onSurfaceVariant,
            )
        }

        Row(horizontalArrangement = Arrangement.spacedBy(12.dp), verticalAlignment = Alignment.CenterVertically) {
            Button(onClick = onAnalyze, enabled = !state.usageLoading) {
                Icon(Icons.Rounded.FolderOpen, contentDescription = null, modifier = Modifier.size(18.dp))
                Spacer(Modifier.width(6.dp))
                Text("分析目录占用")
            }
            if (state.usageLoading) {
                LoadingIndicator(modifier = Modifier.size(20.dp))
                Text("扫描中（最多约 3 秒）…", style = MaterialTheme.typography.bodySmall)
            }
        }
        state.usage?.let { UsageResult(it) }
    }
}

@Composable
private fun UsageResult(usage: StorageUsage) {
    val scheme = MaterialTheme.colorScheme
    Column(verticalArrangement = Arrangement.spacedBy(6.dp)) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Text(
                "/data 共 ${formatStorageKb(usage.totalKb)}",
                style = MaterialTheme.typography.titleSmall,
                modifier = Modifier.weight(1f),
            )
            if (usage.partial) {
                StatusPill(
                    "部分结果",
                    container = scheme.tertiaryContainer,
                    content = scheme.onTertiaryContainer,
                )
            }
        }
        Text(
            "耗时 ${usage.elapsedMs} ms · 访问 ${usage.visited} 项" +
                if (usage.denied > 0) " · ${usage.denied} 项无权限" else "",
            style = MaterialTheme.typography.bodySmall,
            color = scheme.onSurfaceVariant,
        )
        if (usage.entries.isEmpty()) {
            Text("没有可统计的条目", style = MaterialTheme.typography.bodySmall)
            return
        }
        val heaviest = usage.entries.maxOf { it.kb }.coerceAtLeast(1)
        usage.entries.forEachIndexed { index, entry ->
            Column(verticalArrangement = Arrangement.spacedBy(3.dp)) {
                Row {
                    Text(entry.name, style = MaterialTheme.typography.bodyMedium, modifier = Modifier.weight(1f))
                    Text(formatStorageKb(entry.kb), style = MaterialTheme.typography.bodySmall)
                }
                LinearProgressIndicator(
                    progress = { (entry.kb.toFloat() / heaviest).coerceIn(0f, 1f) },
                    modifier = Modifier
                        .fillMaxWidth()
                        .height(4.dp)
                        .clip(CircleShape),
                    color = if (index == 0) scheme.primary else scheme.secondary,
                )
                Text(
                    "${entry.dirs} 目录 · ${entry.files} 文件",
                    style = MaterialTheme.typography.labelSmall,
                    color = scheme.onSurfaceVariant,
                )
            }
        }
        if (usage.partial) {
            Text(
                "预算内只扫描了部分条目；再次点击会重新扫描（结果可能略有不同）。",
                style = MaterialTheme.typography.bodySmall,
                color = scheme.onSurfaceVariant,
            )
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
            if (danger) Icons.Rounded.ErrorOutline else Icons.Rounded.Info,
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

private data class StorageConfirm(
    val title: String,
    val text: String,
    val confirmLabel: String,
    val icon: androidx.compose.ui.graphics.vector.ImageVector,
    val danger: Boolean = false,
    val onConfirm: () -> Unit,
)

private fun trimConfirm(onConfirm: () -> Unit) = StorageConfirm(
    title = "执行 TRIM？",
    text = "向 /data 的存储控制器发送一次丢弃指令。不会删除文件，但可能让设备短暂卡顿；不会在开机时重放。",
    confirmLabel = "执行",
    icon = Icons.Rounded.DeleteForever,
    danger = true,
    onConfirm = onConfirm,
)
