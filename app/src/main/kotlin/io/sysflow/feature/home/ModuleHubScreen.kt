@file:OptIn(ExperimentalMaterial3ExpressiveApi::class)

package io.sysflow.feature.home

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.BatteryFull
import androidx.compose.material.icons.rounded.Dns
import androidx.compose.material.icons.rounded.FolderOpen
import androidx.compose.material.icons.rounded.Lock
import androidx.compose.material.icons.rounded.Memory
import androidx.compose.material.icons.rounded.Timeline
import androidx.compose.material3.ExperimentalMaterial3ExpressiveApi
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import io.sysflow.core.model.CapsData

@Composable
fun ModuleHubScreen(
    caps: CapsData,
    onOpenCpu: () -> Unit,
    modifier: Modifier = Modifier,
    batteryReady: Boolean = false,
    onOpenBattery: () -> Unit = {},
    wakelockReady: Boolean = false,
    onOpenWakelock: () -> Unit = {},
    memReady: Boolean = false,
    onOpenMem: () -> Unit = {},
) {
    val cpuReady = caps.modules["cpu"]?.supported == true
    val batterySupported = batteryReady || caps.modules["battery"]?.supported == true
    val wakelockSupported = wakelockReady || caps.modules["wakelock"]?.supported == true
    val memSupported = memReady || caps.modules["mem"]?.supported == true
    LazyColumn(
        modifier = modifier.fillMaxSize(),
        contentPadding = PaddingValues(start = 20.dp, end = 20.dp, top = 16.dp, bottom = 120.dp),
        verticalArrangement = Arrangement.spacedBy(18.dp),
    ) {
        item {
            Column(verticalArrangement = Arrangement.spacedBy(6.dp)) {
                Text(
                    "SYSFLOW / MODULES",
                    style = MaterialTheme.typography.labelMedium,
                    color = MaterialTheme.colorScheme.primary,
                )
                Text("管理模块", style = MaterialTheme.typography.headlineLargeEmphasized)
                Text(
                    "统一管理设备资源与系统行为",
                    style = MaterialTheme.typography.bodyMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
        }
        item {
            Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
                Row(horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                    ModuleFeatureCard(
                        modifier = Modifier.weight(1f),
                        title = "CPU 管理",
                        subtitle = "核心 · 频率 · 调度",
                        icon = Icons.Rounded.Memory,
                        ready = cpuReady,
                        onClick = if (cpuReady) onOpenCpu else null,
                    )
                    ModuleFeatureCard(
                        modifier = Modifier.weight(1f),
                        title = "电池管理",
                        subtitle = "电量 · 温度 · 健康度",
                        icon = Icons.Rounded.BatteryFull,
                        ready = batterySupported,
                        onClick = if (batterySupported) onOpenBattery else null,
                    )
                }
                Row(horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                    ModuleFeatureCard(
                        modifier = Modifier.weight(1f),
                        title = "内存管理",
                        subtitle = "占用 · 回收 · 压力",
                        icon = Icons.Rounded.Dns,
                        ready = memSupported,
                        onClick = if (memSupported) onOpenMem else null,
                    )
                    ModuleFeatureCard(
                        modifier = Modifier.weight(1f),
                        title = "储存管理",
                        subtitle = "空间 · 文件 · I/O",
                        icon = Icons.Rounded.FolderOpen,
                    )
                }
                Row(horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                    ModuleFeatureCard(
                        modifier = Modifier.weight(1f),
                        title = "进程管理",
                        subtitle = "进程 · 资源 · 优先级",
                        icon = Icons.Rounded.Timeline,
                    )
                    ModuleFeatureCard(
                        modifier = Modifier.weight(1f),
                        title = "唤醒锁",
                        subtitle = "唤醒源 · 持锁 · 拦截",
                        icon = Icons.Rounded.Lock,
                        ready = wakelockReady,
                        onClick = if (wakelockReady) onOpenWakelock else null,
                    )
                }
            }
        }
        item {
            Text(
                "模块仅在实现并通过设备能力探测后开放。当前未接入的模块不会展示模拟数据。",
                modifier = Modifier
                    .fillMaxWidth()
                    .padding(top = 2.dp),
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
    }
}
