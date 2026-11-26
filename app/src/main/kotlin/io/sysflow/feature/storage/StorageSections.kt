@file:OptIn(ExperimentalMaterial3ExpressiveApi::class, ExperimentalLayoutApi::class)

package io.sysflow.feature.storage

import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.DeleteForever
import androidx.compose.material.icons.rounded.FolderOpen
import androidx.compose.material.icons.rounded.Refresh
import androidx.compose.material.icons.rounded.SdStorage
import androidx.compose.material.icons.rounded.Speed
import androidx.compose.material.icons.rounded.Tune
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.CircularWavyProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3ExpressiveApi
import androidx.compose.material3.FilledTonalButton
import androidx.compose.material3.FilledTonalIconButton
import androidx.compose.material3.Icon
import androidx.compose.material3.LoadingIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import io.sysflow.core.ui.components.InfoPill
import io.sysflow.core.ui.components.LineChart
import io.sysflow.core.ui.components.SegmentedValueList
import io.sysflow.core.ui.components.WavyMeter
import io.sysflow.core.ui.components.segmentedShape
import io.sysflow.core.ui.theme.sysFlowColors
import java.util.Locale

/**
 * Sections of the storage page, read top to bottom as a question chain: how full is it → how fast
 * is it moving data → how did the last day look → what is it and how is it queued → what can I do
 * about it.
 *
 * Each section is one list item and spaces its own parts, so the host only decides the order.
 */

// ---- capacity --------------------------------------------------------------------------------

/** How full /data is: a wavy ring for the share, the free space as the number people come for. */
@Composable
internal fun CapacityHero(snapshot: StorageSnapshot, modifier: Modifier = Modifier) {
    val scheme = MaterialTheme.colorScheme
    val accent = MaterialTheme.sysFlowColors.containerAccent
    val capacity = snapshot.capacity
    val progress by animateFloatAsState(
        targetValue = capacity.usedFraction,
        animationSpec = MaterialTheme.motionScheme.defaultEffectsSpec(),
        label = "storageUsed",
    )

    Surface(
        modifier = modifier.fillMaxWidth(),
        shape = MaterialTheme.shapes.extraExtraLarge,
        color = scheme.primaryContainer,
        contentColor = scheme.onPrimaryContainer,
    ) {
        Column(
            modifier = Modifier.padding(20.dp),
            verticalArrangement = Arrangement.spacedBy(20.dp),
        ) {
            Row(
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(16.dp),
            ) {
                Box(
                    modifier = Modifier
                        .size(120.dp)
                        .semantics { contentDescription = "已用 ${formatStoragePct(capacity.usedPct)}" },
                    contentAlignment = Alignment.Center,
                ) {
                    CircularWavyProgressIndicator(
                        progress = { progress },
                        modifier = Modifier.size(120.dp),
                        color = accent,
                        trackColor = scheme.onPrimaryContainer.copy(alpha = 0.16f),
                    )
                    Column(horizontalAlignment = Alignment.CenterHorizontally) {
                        Text(
                            formatStoragePct(capacity.usedPct),
                            style = MaterialTheme.typography.titleLargeEmphasized,
                        )
                        Text(
                            "已用",
                            style = MaterialTheme.typography.labelMedium,
                            color = scheme.onPrimaryContainer.copy(alpha = 0.75f),
                        )
                    }
                }
                Column(
                    modifier = Modifier.weight(1f),
                    verticalArrangement = Arrangement.spacedBy(4.dp),
                ) {
                    Text(
                        "内部存储 /data · 可用",
                        style = MaterialTheme.typography.labelLarge,
                        color = scheme.onPrimaryContainer.copy(alpha = 0.75f),
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                    )
                    Text(
                        formatStorageKb(capacity.availKb),
                        style = MaterialTheme.typography.headlineLargeEmphasized,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                    )
                }
            }
            FlowRow(
                horizontalArrangement = Arrangement.spacedBy(8.dp),
                verticalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                HeroPill("共 ${formatStorageKb(capacity.totalKb)}")
                HeroPill("已用 ${formatStorageKb(capacity.usedKb)}")
                HeroPill("空闲 ${formatStorageKb(capacity.freeKb)}")
            }
        }
    }
}

@Composable
private fun HeroPill(text: String) {
    InfoPill(
        text = text,
        container = MaterialTheme.colorScheme.onPrimaryContainer.copy(alpha = 0.12f),
        content = MaterialTheme.colorScheme.onPrimaryContainer,
    )
}

/** The second filesystem (external / emulated storage), when the device has one. */
@Composable
internal fun SdcardSection(capacity: StorageCapacity, modifier: Modifier = Modifier) {
    SectionSurface(modifier) {
        Column(
            modifier = Modifier.padding(16.dp),
            verticalArrangement = Arrangement.spacedBy(14.dp),
        ) {
            Row(
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(12.dp),
            ) {
                ActionGroupHeader(
                    icon = Icons.Rounded.SdStorage,
                    title = "外置 / 模拟存储",
                    supporting = "${formatStorageKb(capacity.usedKb)} / ${formatStorageKb(capacity.totalKb)}",
                    modifier = Modifier.weight(1f),
                )
                InfoPill(
                    text = formatStoragePct(capacity.usedPct),
                    container = MaterialTheme.colorScheme.secondaryContainer,
                    content = MaterialTheme.colorScheme.onSecondaryContainer,
                )
            }
            WavyMeter(progress = capacity.usedFraction, modifier = Modifier.fillMaxWidth())
        }
    }
}

// ---- live I/O --------------------------------------------------------------------------------

/** Two throughput tiles and one busy meter; the cumulative totals sit under them as a note. */
@Composable
internal fun IoSection(
    io: StorageIo?,
    loading: Boolean,
    onRefresh: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val scheme = MaterialTheme.colorScheme
    Column(modifier, verticalArrangement = Arrangement.spacedBy(8.dp)) {
        SectionHeader(
            title = "实时 I/O",
            supporting = if (io == null) "正在采样…（间隔 1 秒）" else "每 2 秒刷新 · 采样 ${io.intervalMs} ms",
            action = { RefreshAction(loading, "刷新 I/O", onRefresh) },
            modifier = Modifier.padding(bottom = 4.dp),
        )
        if (io == null) {
            NoteTile(text = "第一次读数需要约 1 秒的采样间隔。")
            return@Column
        }
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            StatTile(
                label = "读取",
                value = formatStorageRate(io.readKb),
                supporting = "${formatIops(io.readIops)} IOPS",
                container = scheme.primaryContainer,
                content = scheme.onPrimaryContainer,
                modifier = Modifier.weight(1f),
            )
            StatTile(
                label = "写入",
                value = formatStorageRate(io.writeKb),
                supporting = "${formatIops(io.writeIops)} IOPS",
                container = scheme.tertiaryContainer,
                content = scheme.onTertiaryContainer,
                modifier = Modifier.weight(1f),
            )
        }
        Surface(
            modifier = Modifier.fillMaxWidth(),
            shape = MaterialTheme.shapes.large,
            color = scheme.secondaryContainer,
            contentColor = scheme.onSecondaryContainer,
        ) {
            Column(
                modifier = Modifier.padding(horizontal = 16.dp, vertical = 14.dp),
                verticalArrangement = Arrangement.spacedBy(10.dp),
            ) {
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Text(
                        "设备忙碌",
                        style = MaterialTheme.typography.labelMedium,
                        color = scheme.onSecondaryContainer.copy(alpha = 0.75f),
                        modifier = Modifier.weight(1f),
                    )
                    Text(
                        String.format(Locale.ROOT, "%.1f%%", io.utilPct),
                        style = MaterialTheme.typography.titleMediumEmphasized,
                    )
                }
                WavyMeter(
                    progress = (io.utilPct / 100.0).toFloat(),
                    color = scheme.secondary,
                    trackColor = scheme.onSecondaryContainer.copy(alpha = 0.16f),
                )
            }
        }
        if (io.totalReadKb != null && io.totalWriteKb != null) {
            NoteTile(
                text = "自开机累计：读 ${formatStorageKb(io.totalReadKb)} · 写 ${formatStorageKb(io.totalWriteKb)}",
            )
        }
    }
}

/** Tonal icon button that turns into a spinner while its request is in flight. */
@Composable
private fun RefreshAction(loading: Boolean, description: String, onClick: () -> Unit) {
    FilledTonalIconButton(onClick = onClick, enabled = !loading) {
        if (loading) {
            LoadingIndicator(modifier = Modifier.size(24.dp))
        } else {
            Icon(Icons.Rounded.Refresh, contentDescription = description)
        }
    }
}

// ---- history ---------------------------------------------------------------------------------

@Composable
internal fun HistorySection(
    history: StorageHistory,
    loading: Boolean,
    onRefresh: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val scheme = MaterialTheme.colorScheme
    Column(modifier, verticalArrangement = Arrangement.spacedBy(8.dp)) {
        SectionHeader(
            title = "24 小时吞吐",
            supporting = "守护进程每 5 分钟记录一次",
            action = { RefreshAction(loading, "刷新历史", onRefresh) },
            modifier = Modifier.padding(bottom = 4.dp),
        )
        if (history.isEmpty) {
            NoteTile(text = "还没有历史采样点。刚升级或 daemon 未运行时曲线为空。")
            return@Column
        }
        SectionSurface {
            Column(
                modifier = Modifier.padding(16.dp),
                verticalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                ChartLegend("读取", scheme.primary)
                LineChart(points = history.read, color = scheme.primary, height = 84.dp)
                ChartLegend("写入", scheme.tertiary, modifier = Modifier.padding(top = 8.dp))
                LineChart(points = history.write, color = scheme.tertiary, height = 84.dp)
            }
        }
    }
}

@Composable
private fun ChartLegend(label: String, color: Color, modifier: Modifier = Modifier) {
    Row(
        modifier = modifier,
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        Box(
            Modifier
                .size(10.dp)
                .clip(CircleShape)
                .background(color),
        )
        Text(
            label,
            style = MaterialTheme.typography.labelLarge,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
    }
}

// ---- device & queue --------------------------------------------------------------------------

/** What the flash is, plus the two persistent queue knobs; the checked segment is the live value. */
@Composable
internal fun DeviceSection(
    state: StorageViewModel.UiState,
    onScheduler: (String) -> Unit,
    onReadAhead: (Int) -> Unit,
    modifier: Modifier = Modifier,
) {
    val snapshot = state.snapshot ?: return
    Column(modifier, verticalArrangement = Arrangement.spacedBy(12.dp)) {
        SectionHeader(title = "设备与队列", supporting = "调度器与预读会持久化")

        SegmentedValueList(
            rows = buildList {
                add("设备" to (snapshot.device.name ?: "—"))
                add("型号" to (snapshot.device.model ?: "—"))
                snapshot.device.sizeKb?.let { add("设备容量" to formatStorageKb(it)) }
                add("介质" to if (snapshot.device.rotational) "机械盘" else "闪存")
                snapshot.device.healthPct?.let { add("剩余寿命（JEDEC 估算）" to "$it%") }
            },
        )

        if (state.options.scheduler.isNotEmpty()) {
            SectionSurface {
                Column(
                    modifier = Modifier.padding(16.dp),
                    verticalArrangement = Arrangement.spacedBy(14.dp),
                ) {
                    ActionGroupHeader(
                        icon = Icons.Rounded.Tune,
                        title = "I/O 调度器",
                        supporting = "当前 ${snapshot.scheduler ?: "—"}",
                    )
                    Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
                        state.options.scheduler.chunked(2).forEach { row ->
                            ConnectedChoiceRow(
                                options = row,
                                selected = snapshot.scheduler,
                                label = { it },
                                onSelect = { option ->
                                    if (option != snapshot.scheduler) onScheduler(option)
                                },
                                enabled = !state.saving,
                                compact = true,
                                slots = 2,
                            )
                        }
                    }
                }
            }
        }

        if (state.options.readAhead.isNotEmpty()) {
            SectionSurface {
                Column(
                    modifier = Modifier.padding(16.dp),
                    verticalArrangement = Arrangement.spacedBy(14.dp),
                ) {
                    ActionGroupHeader(
                        icon = Icons.Rounded.Speed,
                        title = "预读",
                        supporting = "单位 kB · 当前 ${snapshot.readAheadKb ?: "—"}",
                        container = MaterialTheme.colorScheme.tertiaryContainer,
                        content = MaterialTheme.colorScheme.onTertiaryContainer,
                    )
                    Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
                        state.options.readAhead.chunked(3).forEach { row ->
                            ConnectedChoiceRow(
                                options = row,
                                selected = snapshot.readAheadKb,
                                label = { it.toString() },
                                onSelect = { option ->
                                    if (option != snapshot.readAheadKb) onReadAhead(option)
                                },
                                enabled = !state.saving,
                                compact = true,
                                slots = 3,
                            )
                        }
                    }
                }
            }
        }

        if (state.saving) BusyTile("正在写入并读回")
        NoteTile(text = "开机时由模块重新应用；ROM 改回时守护进程会修复。")
    }
}

// ---- maintenance -----------------------------------------------------------------------------

@Composable
internal fun MaintenanceSection(
    state: StorageViewModel.UiState,
    onTrim: () -> Unit,
    onAnalyze: () -> Unit,
    onUsagePath: (String) -> Unit,
    modifier: Modifier = Modifier,
) {
    val scheme = MaterialTheme.colorScheme
    Column(modifier, verticalArrangement = Arrangement.spacedBy(12.dp)) {
        SectionHeader(title = "维护", supporting = "两项都只在你点按时执行")

        SectionSurface {
            Column(
                modifier = Modifier.padding(16.dp),
                verticalArrangement = Arrangement.spacedBy(14.dp),
            ) {
                ActionGroupHeader(
                    icon = Icons.Rounded.DeleteForever,
                    title = "TRIM /data",
                    supporting = "一次性 fstrim，不会开机重放",
                    container = scheme.errorContainer,
                    content = scheme.onErrorContainer,
                )
                state.actionRunning?.let { BusyTile(it) }
                Button(
                    onClick = onTrim,
                    enabled = state.actionRunning == null,
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
                    Text("执行 TRIM", maxLines = 1)
                }
            }
        }

        SectionSurface {
            Column(
                modifier = Modifier.padding(16.dp),
                verticalArrangement = Arrangement.spacedBy(14.dp),
            ) {
                ActionGroupHeader(
                    icon = Icons.Rounded.FolderOpen,
                    title = "目录占用",
                    supporting = "内部存储看照片/下载，应用数据看整块 /data；最多约 3 秒",
                    container = scheme.tertiaryContainer,
                    content = scheme.onTertiaryContainer,
                )
                ConnectedChoiceRow(
                    options = listOf(StorageViewModel.USER_STORAGE, StorageViewModel.SYSTEM_DATA),
                    selected = state.usagePath,
                    label = { usageLabel(it) },
                    onSelect = onUsagePath,
                    enabled = !state.usageLoading,
                )
                NoteTile(
                    text = "内部存储 = /storage/emulated/0（DCIM、Download、Android 等用户文件）；" +
                        "应用数据 = /data 整块分区，包含前者，两个视图不可相加。",
                )
                if (state.usageLoading) BusyTile("扫描中（最多约 3 秒）")
                FilledTonalButton(
                    onClick = onAnalyze,
                    enabled = !state.usageLoading,
                    modifier = Modifier.fillMaxWidth(),
                    shapes = ButtonDefaults.shapes(),
                ) {
                    Icon(
                        Icons.Rounded.FolderOpen,
                        contentDescription = null,
                        modifier = Modifier.size(ButtonDefaults.IconSize),
                    )
                    Spacer(Modifier.size(ButtonDefaults.IconSpacing))
                    Text(
                        if (state.usagePath == StorageViewModel.SYSTEM_DATA) {
                            "分析应用数据"
                        } else {
                            "分析内部存储"
                        },
                        maxLines = 1,
                    )
                }
            }
        }

        state.usage?.let { UsageResult(it) }
    }
}

/** Friendly name for the two supported analysis roots; other paths are shown verbatim. */
internal fun usageLabel(path: String): String = when (path) {
    StorageViewModel.USER_STORAGE -> "内部存储"
    StorageViewModel.SYSTEM_DATA -> "应用数据"
    else -> path
}

/** Heaviest directories as one connected list; the bar is relative to the heaviest entry. */
@Composable
private fun UsageResult(usage: StorageUsage) {
    val scheme = MaterialTheme.colorScheme
    val accent = MaterialTheme.sysFlowColors.containerAccent
    Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
        SectionHeader(
            title = "${usageLabel(usage.path)} 共 ${formatStorageKb(usage.totalKb)}",
            supporting = "耗时 ${usage.elapsedMs} ms · 访问 ${usage.visited} 项" +
                if (usage.denied > 0) " · ${usage.denied} 项无权限" else "",
            action = {
                if (usage.partial) {
                    InfoPill(
                        text = "部分结果",
                        container = scheme.tertiaryContainer,
                        content = scheme.onTertiaryContainer,
                    )
                }
            },
            modifier = Modifier.padding(bottom = 8.dp),
        )
        if (usage.entries.isEmpty()) {
            NoteTile(text = "没有可统计的条目")
            return@Column
        }
        val heaviest = usage.entries.maxOf { it.kb }.coerceAtLeast(1)
        usage.entries.forEachIndexed { index, entry ->
            val top = index == 0
            val content = if (top) scheme.onPrimaryContainer else scheme.onSurface
            Surface(
                modifier = Modifier.fillMaxWidth(),
                shape = segmentedShape(index, usage.entries.size),
                color = if (top) scheme.primaryContainer else scheme.surfaceContainerHigh,
                contentColor = content,
            ) {
                Column(
                    modifier = Modifier.padding(horizontal = 16.dp, vertical = 14.dp),
                    verticalArrangement = Arrangement.spacedBy(6.dp),
                ) {
                    Row(
                        verticalAlignment = Alignment.CenterVertically,
                        horizontalArrangement = Arrangement.spacedBy(12.dp),
                    ) {
                        Text(
                            entry.name,
                            modifier = Modifier.weight(1f),
                            style = MaterialTheme.typography.titleSmallEmphasized,
                            maxLines = 1,
                            overflow = TextOverflow.Ellipsis,
                        )
                        Text(
                            formatStorageKb(entry.kb),
                            style = MaterialTheme.typography.titleSmallEmphasized,
                            maxLines = 1,
                        )
                    }
                    UsageBar(
                        fraction = entry.kb.toFloat() / heaviest,
                        color = if (top) accent else scheme.primary,
                        track = if (top) {
                            scheme.onPrimaryContainer.copy(alpha = 0.16f)
                        } else {
                            scheme.surfaceContainerHighest
                        },
                    )
                    Text(
                        "${entry.dirs} 目录 · ${entry.files} 文件",
                        style = MaterialTheme.typography.labelSmall,
                        color = content.copy(alpha = 0.75f),
                    )
                }
            }
        }
        if (usage.partial) {
            NoteTile(
                text = "预算内只扫描了部分条目；再次点击会重新扫描（结果可能略有不同）。",
                modifier = Modifier.padding(top = 8.dp),
            )
        }
    }
}

/** The action in flight; the controls that would start another one are disabled meanwhile. */
@Composable
internal fun BusyTile(label: String, modifier: Modifier = Modifier) {
    Surface(
        modifier = modifier.fillMaxWidth(),
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
                "${label}…",
                style = MaterialTheme.typography.bodyMedium,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
        }
    }
}
