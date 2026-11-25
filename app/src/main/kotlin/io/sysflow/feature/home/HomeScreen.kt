@file:OptIn(ExperimentalMaterial3ExpressiveApi::class)

package io.sysflow.feature.home

import io.sysflow.core.event.SystemEventEntity
import io.sysflow.core.ui.components.WavyMeter
import io.sysflow.core.ui.components.IndexBadge
import androidx.compose.animation.animateColorAsState
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.BatteryFull
import androidx.compose.material.icons.rounded.Bolt
import androidx.compose.material.icons.rounded.ChevronRight
import androidx.compose.material.icons.rounded.Dns
import androidx.compose.material.icons.rounded.Memory
import androidx.compose.material.icons.rounded.Shield
import androidx.compose.material.icons.rounded.Storage
import androidx.compose.material.icons.rounded.Tune
import androidx.compose.material3.ExperimentalMaterial3ExpressiveApi
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialShapes
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.toShape
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.buildAnnotatedString
import androidx.compose.ui.text.withStyle
import androidx.compose.foundation.border
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import io.sysflow.core.model.BatterySnapshot
import io.sysflow.core.model.CapsData
import io.sysflow.core.model.CpuLiveTracker
import io.sysflow.core.model.CpuSnapshot
import io.sysflow.core.model.SafeStatus
import io.sysflow.core.model.SnapshotData
import io.sysflow.core.model.chargerLabel
import io.sysflow.core.model.batteryStatusLabel
import io.sysflow.core.model.formatKhz
import io.sysflow.core.model.formatTemp
import io.sysflow.core.ui.components.StatusPill
import io.sysflow.core.ui.components.segmentedShape
import io.sysflow.core.ui.theme.sysFlowColors
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale
import kotlin.math.roundToInt

@Composable
fun HomeScreen(
    snapshot: SnapshotData,
    caps: CapsData,
    safe: SafeStatus?,
    deviceInfo: DeviceInfoDisplay?,
    activityEvents: List<SystemEventEntity>,
    battery: BatterySnapshot?,
    onOpenCpu: () -> Unit,
    onClearSafe: () -> Unit,
    onOpenEvents: (String?) -> Unit,
    onOpenActivity: () -> Unit,
    onOpenBattery: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val cpu = remember(snapshot.modules) { CpuSnapshot.from(snapshot.modules["cpu"]) }
    val cpuConnected = caps.modules["cpu"]?.supported == true && cpu != null
    // The overview is refreshed by AppViewModel every ~2s; keep the previous sample here so the
    // /proc/stat and time_in_state deltas can be turned into a live load and frequency.
    val previousCpu = remember { CpuSampleHolder() }
    val cpuSummary = remember(cpu, cpuConnected) {
        cpu?.takeIf { cpuConnected }?.let { value ->
            val live = CpuLiveTracker.update(previousCpu.sample, value)
            previousCpu.sample = value
            val performanceCluster = value.clusters.keys
                .sortedBy { it.toIntOrNull() ?: 0 }
                .lastOrNull()
            val freqKhz = performanceCluster
                ?.let { id -> value.clusters[id]?.firstNotNullOfOrNull { cpuId -> live.coreFreqKhz[cpuId] } }
                ?: performanceCluster?.let(value::clusterCurrent)
                ?: 0L
            CpuSummary(
                usage = live.totalUsage,
                online = "${value.onlineCount}/${value.online.size}",
                coreStates = value.online.map { it == 1 },
                allOnline = value.onlineCount == value.online.size,
                freq = formatKhz(freqKhz),
                tempCelsius = value.tempDeciCelsius?.let { it / 10f },
            )
        }
    }
    val activeModules = remember(caps.modules) {
        caps.modules.filterValues { it.supported }.keys.map(::moduleDisplayName)
    }
    val updatedAt = remember(snapshot.ts) {
        SimpleDateFormat("HH:mm:ss", Locale.getDefault()).format(Date(snapshot.ts * 1000))
    }

    LazyColumn(
        modifier = modifier.fillMaxSize(),
        contentPadding = PaddingValues(start = 20.dp, end = 20.dp, top = 8.dp, bottom = 120.dp),
        verticalArrangement = Arrangement.spacedBy(22.dp),
    ) {
        if (safe?.safe_mode == true) {
            item { SafeBanner(onClearSafe = onClearSafe) }
        }

        item {
            OverviewHeading(
                safeMode = safe?.safe_mode == true,
                hasConnectedModule = activeModules.isNotEmpty(),
                deviceInfo = deviceInfo,
                moduleVersion = caps.sysflow_version,
            )
        }

        item {
            Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
                SectionHeading(index = "01", title = "设备状态", trailing = "设备快照")
                Row(horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                    DeviceStatusCard(
                        modifier = Modifier.weight(1f),
                        title = "CPU",
                        icon = Icons.Rounded.Memory,
                        accent = MaterialTheme.colorScheme.surface.copy(alpha = 0.64f),
                        accentContent = MaterialTheme.colorScheme.onSurface,
                        value = cpuSummary?.usage?.let { "${(it * 100f).roundToInt()}%" } ?: "—",
                        valueSuffix = if (cpuSummary != null) "使用率" else "",
                        meter = cpuSummary?.let { it.usage ?: 0f },
                        detail = AnnotatedString(cpuSummary?.let { "${it.freq}" } ?: "CPU 数据暂不可用"),
                        detailAnnotated = cpuSummary?.let { cpuDetailText(it) },
                        status = when {
                            cpuSummary == null -> "未连接"
                            else -> "${cpuSummary.online} 在线"
                        },
                        statusActive = cpuConnected,
                        statusWarn = cpuSummary?.allOnline == false,
                        coreDots = cpuSummary?.coreStates?.takeIf { it.size in 2..8 },
                        onClick = if (cpuConnected) onOpenCpu else null,
                    )
                    DeviceStatusCard(
                        modifier = Modifier.weight(1f),
                        title = "BATTERY",
                        icon = Icons.Rounded.BatteryFull,
                        accent = MaterialTheme.colorScheme.tertiaryContainer.copy(alpha = 0.62f),
                        accentContent = MaterialTheme.colorScheme.onTertiaryContainer,
                        value = battery?.capacity?.toString() ?: "—",
                        valueSuffix = if (battery?.capacity != null) "%" else "",
                        detail = AnnotatedString(
                            if (battery != null) {
                                "${batteryStatusLabel(battery.status)} · ${formatTemp(battery.tempDeciC?.toLong())}"
                            } else {
                                "电池电量 · 等待模块接入"
                            },
                        ),
                        status = if (battery != null) chargerLabel(battery) else "未接入",
                        statusActive = battery != null,
                        statusWarn = battery?.let { it.capacity != null && it.capacity <= 15 && !it.charging } == true,
                        meter = battery?.capacity?.let { it.coerceIn(0, 100) / 100f },
                        onClick = onOpenBattery,
                    )
                }
                Row(horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                    DeviceStatusCard(
                        modifier = Modifier.weight(1f),
                        title = "MEMORY",
                        icon = Icons.Rounded.Dns,
                        accent = MaterialTheme.colorScheme.secondaryContainer.copy(alpha = 0.72f),
                        value = "—",
                        detail = AnnotatedString("已用内存 / 总内存"),
                        status = "等待模块接入",
                    )
                    DeviceStatusCard(
                        modifier = Modifier.weight(1f),
                        title = "STORAGE",
                        icon = Icons.Rounded.Storage,
                        accent = MaterialTheme.colorScheme.secondaryContainer.copy(alpha = 0.72f),
                        value = "—",
                        detail = AnnotatedString("已用空间 / 总容量"),
                        status = "等待模块接入",
                    )
                }
            }
        }

        item {
            SystemActivitySection(
                events = activityEvents,
                onOpenModule = { module -> onOpenEvents(module) },
                onOpenAll = onOpenActivity,
            )
        }

        item {
            Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
                SectionHeading(index = "03", title = "系统运行状态", trailing = "SYSFLOW CORE")
                RuntimeStatusCard(
                    rootGranted = true,
                    protocol = caps.protocol,
                    activeModules = activeModules,
                    version = caps.sysflow_version,
                )
            }
        }

        item {
            Column(
                modifier = Modifier
                    .fillMaxWidth()
                    .padding(top = 2.dp, bottom = 8.dp),
                horizontalAlignment = Alignment.CenterHorizontally,
                verticalArrangement = Arrangement.spacedBy(5.dp),
            ) {
                Text(
                    "最后更新：$updatedAt",
                    style = MaterialTheme.typography.labelMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
                Text(
                    "数据由 SysFlow 模块快照提供",
                    style = MaterialTheme.typography.labelSmall,
                    color = MaterialTheme.colorScheme.outline,
                )
            }
        }
    }
}

@Composable
private fun OverviewHeading(
    safeMode: Boolean,
    hasConnectedModule: Boolean,
    deviceInfo: DeviceInfoDisplay?,
    moduleVersion: String,
) {
    val deviceModel = deviceInfo?.deviceModel?.takeIf { it.isNotBlank() }
    val androidRelease = deviceInfo?.androidRelease?.takeIf { it.isNotBlank() }
    val deviceLine = buildList<String> {
        if (deviceModel != null) add(deviceModel)
        if (androidRelease != null) add("Android $androidRelease")
    }.joinToString(" • ").ifBlank { "设备信息暂不可用" }
    val normalizedModuleVersion = moduleVersion.removePrefix("v").removePrefix("V")
    val kernelVersion = deviceInfo?.kernelRelease?.takeIf { it.isNotBlank() }
        ?: "内核版本暂不可用"

    Column(verticalArrangement = Arrangement.spacedBy(5.dp)) {
        Row(
            modifier = Modifier.fillMaxWidth(),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.SpaceBetween,
        ) {
            Text(
                "SysFlow",
                style = MaterialTheme.typography.headlineLargeEmphasized,
                color = MaterialTheme.colorScheme.onSurface,
            )
            when {
                safeMode -> StatusPill(
                    text = "安全模式",
                    container = MaterialTheme.colorScheme.errorContainer,
                    content = MaterialTheme.colorScheme.onErrorContainer,
                )
                hasConnectedModule -> StatusPill(
                    text = "运行状态",
                    container = MaterialTheme.colorScheme.tertiaryContainer,
                    content = MaterialTheme.colorScheme.onTertiaryContainer,
                )
                else -> StatusPill(text = "等待接入")
            }
        }
        Text(
            "SYSFLOW / DEVICE · MODULE v$normalizedModuleVersion",
            style = MaterialTheme.typography.labelMedium,
            color = MaterialTheme.colorScheme.primary,
            fontWeight = FontWeight.SemiBold,
        )
        Text(
            deviceLine,
            style = MaterialTheme.typography.bodyLarge,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            maxLines = 1,
            overflow = TextOverflow.Ellipsis,
        )
        Text(
            kernelVersion,
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.outline,
            maxLines = 2,
            overflow = TextOverflow.Ellipsis,
        )
    }
}

@Composable
private fun SectionHeading(
    index: String,
    title: String,
    trailing: String,
    onTrailingClick: (() -> Unit)? = null,
) {
    Row(
        modifier = Modifier.fillMaxWidth(),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        IndexBadge(index)
        Text(
            title,
            modifier = Modifier
                .padding(start = 10.dp)
                .weight(1f),
            style = MaterialTheme.typography.titleLargeEmphasized,
        )
        Text(
            trailing,
            modifier = if (onTrailingClick != null) Modifier.clickable(onClick = onTrailingClick) else Modifier,
            style = MaterialTheme.typography.labelMedium,
            color = if (onTrailingClick != null) MaterialTheme.colorScheme.primary else MaterialTheme.colorScheme.outline,
        )
    }
}

@Composable
private fun DeviceStatusCard(
    title: String,
    icon: androidx.compose.ui.graphics.vector.ImageVector,
    value: String,
    detail: AnnotatedString,
    status: String,
    modifier: Modifier = Modifier,
    valueSuffix: String = "",
    detailAnnotated: AnnotatedString? = null,
    statusWarn: Boolean = false,
    coreDots: List<Boolean>? = null,
    meter: Float? = null,
    accent: Color = MaterialTheme.colorScheme.secondaryContainer,
    accentContent: Color = MaterialTheme.colorScheme.onSecondaryContainer,
    statusActive: Boolean = false,
    onClick: (() -> Unit)? = null,
) {
    Surface(
        modifier = modifier
            .height(172.dp)
            .then(
                if (onClick != null) Modifier.clickable(onClick = onClick) else Modifier,
            ),
        shape = MaterialTheme.shapes.extraLarge,
        color = if (onClick != null) MaterialTheme.colorScheme.primaryContainer
        else MaterialTheme.colorScheme.surfaceContainerLow,
        contentColor = if (onClick != null) MaterialTheme.colorScheme.onPrimaryContainer
        else MaterialTheme.colorScheme.onSurface,
        tonalElevation = 1.dp,
    ) {
        Column(
            modifier = Modifier
                .fillMaxSize()
                .padding(16.dp),
            verticalArrangement = Arrangement.SpaceBetween,
        ) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Box(
                    modifier = Modifier
                        .size(38.dp)
                        .clip(MaterialShapes.Cookie6Sided.toShape())
                        .background(accent),
                    contentAlignment = Alignment.Center,
                ) {
                    Icon(
                        icon,
                        contentDescription = null,
                        tint = accentContent,
                        modifier = Modifier.size(21.dp),
                    )
                }
                Spacer(Modifier.weight(1f))
                Text(
                    title,
                    style = MaterialTheme.typography.labelSmall,
                    color = if (onClick != null) MaterialTheme.colorScheme.onPrimaryContainer.copy(alpha = 0.68f)
                    else MaterialTheme.colorScheme.onSurfaceVariant,
                    fontWeight = FontWeight.Bold,
                )
            }
            Column(verticalArrangement = Arrangement.spacedBy(3.dp)) {
                Row(verticalAlignment = Alignment.Bottom, horizontalArrangement = Arrangement.spacedBy(5.dp)) {
                    Text(
                        value,
                        style = MaterialTheme.typography.headlineMediumEmphasized,
                        color = if (onClick != null) MaterialTheme.colorScheme.onPrimaryContainer
                        else MaterialTheme.colorScheme.onSurface,
                        maxLines = 1,
                    )
                    if (valueSuffix.isNotBlank()) {
                        Text(
                            valueSuffix,
                            modifier = Modifier.padding(bottom = 4.dp),
                            style = MaterialTheme.typography.labelMedium,
                            color = if (onClick != null) MaterialTheme.colorScheme.onPrimaryContainer.copy(alpha = 0.72f)
                            else MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                    }
                }
                Text(
                    detailAnnotated ?: detail,
                    style = MaterialTheme.typography.bodySmall,
                    color = if (onClick != null) MaterialTheme.colorScheme.onPrimaryContainer.copy(alpha = 0.76f)
                    else MaterialTheme.colorScheme.onSurfaceVariant,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
                if (meter != null) {
                    WavyMeter(
                        progress = meter,
                        modifier = Modifier.padding(top = 4.dp),
                        color = MaterialTheme.sysFlowColors.containerAccent,
                        trackColor = MaterialTheme.colorScheme.onPrimaryContainer.copy(alpha = 0.22f),
                    )
                }
            }
            Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                if (coreDots != null) {
                    val dotColor = if (statusWarn) MaterialTheme.sysFlowColors.warning
                    else MaterialTheme.sysFlowColors.containerAccent
                    Row(horizontalArrangement = Arrangement.spacedBy(3.dp), verticalAlignment = Alignment.CenterVertically) {
                        coreDots.forEach { on ->
                            Box(
                                Modifier
                                    .size(6.dp)
                                    .clip(CircleShape)
                                    .then(
                                        if (on) Modifier.background(dotColor)
                                        else Modifier.border(1.dp, dotColor.copy(alpha = 0.7f), CircleShape),
                                    ),
                            )
                        }
                    }
                } else {
                    Box(
                        Modifier
                            .size(6.dp)
                            .clip(MaterialTheme.shapes.small)
                            .background(
                                if (statusActive) MaterialTheme.sysFlowColors.containerAccent
                                else MaterialTheme.colorScheme.outlineVariant,
                            ),
                    )
                }
                Text(
                    status,
                    style = MaterialTheme.typography.labelSmall,
                    color = when {
                        statusWarn -> MaterialTheme.sysFlowColors.warning
                        statusActive -> MaterialTheme.colorScheme.onPrimaryContainer.copy(alpha = 0.76f)
                        else -> MaterialTheme.colorScheme.outline
                    },
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
            }
        }
    }
}

@Composable
fun ModuleFeatureCard(
    title: String,
    subtitle: String,
    icon: androidx.compose.ui.graphics.vector.ImageVector,
    modifier: Modifier = Modifier,
    ready: Boolean = false,
    onClick: (() -> Unit)? = null,
) {
    Surface(
        modifier = modifier
            .height(112.dp)
            .then(if (onClick != null) Modifier.clickable(onClick = onClick) else Modifier),
        shape = MaterialTheme.shapes.extraLarge,
        color = if (ready) MaterialTheme.colorScheme.primaryContainer
        else MaterialTheme.colorScheme.surfaceContainerLow,
        contentColor = if (ready) MaterialTheme.colorScheme.onPrimaryContainer
        else MaterialTheme.colorScheme.onSurface,
        tonalElevation = 1.dp,
    ) {
        Row(
            modifier = Modifier
                .fillMaxSize()
                .padding(horizontal = 13.dp, vertical = 14.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(11.dp),
        ) {
            Box(
                modifier = Modifier
                    .size(46.dp)
                    .clip(MaterialShapes.Cookie7Sided.toShape())
                    .background(
                        if (ready) MaterialTheme.colorScheme.surface.copy(alpha = 0.64f)
                        else MaterialTheme.colorScheme.secondaryContainer.copy(alpha = 0.72f),
                    ),
                contentAlignment = Alignment.Center,
            ) {
                Icon(
                    icon,
                    contentDescription = null,
                    tint = if (ready) MaterialTheme.colorScheme.onSurface
                    else MaterialTheme.colorScheme.onSecondaryContainer,
                    modifier = Modifier.size(23.dp),
                )
            }
            Column(
                modifier = Modifier.weight(1f),
                verticalArrangement = Arrangement.spacedBy(4.dp),
            ) {
                Text(
                    title,
                    style = MaterialTheme.typography.titleSmallEmphasized,
                    color = if (ready) MaterialTheme.colorScheme.onPrimaryContainer
                    else MaterialTheme.colorScheme.onSurface,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
                Text(
                    subtitle,
                    style = MaterialTheme.typography.labelSmall,
                    color = if (ready) MaterialTheme.colorScheme.onPrimaryContainer.copy(alpha = 0.76f)
                    else MaterialTheme.colorScheme.onSurfaceVariant,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
                Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(5.dp)) {
                    Box(
                        Modifier
                            .size(5.dp)
                            .clip(MaterialTheme.shapes.small)
                            .background(
                                if (ready) MaterialTheme.sysFlowColors.containerAccent
                                else MaterialTheme.colorScheme.outlineVariant,
                            ),
                    )
                    Text(
                        if (ready) "已接入" else "规划中",
                        style = MaterialTheme.typography.labelSmall,
                        color = if (ready) MaterialTheme.colorScheme.onPrimaryContainer.copy(alpha = 0.76f)
                        else MaterialTheme.colorScheme.outline,
                    )
                }
            }
            if (onClick != null) {
                Icon(
                    Icons.Rounded.ChevronRight,
                    contentDescription = null,
                    tint = if (ready) MaterialTheme.colorScheme.onPrimaryContainer
                    else MaterialTheme.colorScheme.onSurfaceVariant,
                    modifier = Modifier.size(18.dp),
                )
            }
        }
    }
}

/* The overview card holds a fixed number of module rows so it never grows with module count. */
private const val MODULE_ROW_LIMIT = 4
private const val MODULE_ROW_EVENTS = 2

@Composable
private fun SystemActivitySection(
    events: List<SystemEventEntity>,
    onOpenModule: (String) -> Unit,
    onOpenAll: () -> Unit,
) {
    val groups = remember(events) { buildModuleActivity(events, MODULE_ROW_EVENTS) }
    val stats = remember(events) { moduleStats(events) }
    Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
        SectionHeading(index = "02", title = "系统动态", trailing = "按模块汇总")
        if (groups.isEmpty()) {
            Surface(
                modifier = Modifier.fillMaxWidth(),
                shape = MaterialTheme.shapes.extraLarge,
                color = MaterialTheme.colorScheme.surfaceContainerLow,
            ) {
                EmptyActivityState()
            }
        } else {
            val visible = groups.take(MODULE_ROW_LIMIT)
            // Module tiles plus one trailing "view all" tile form a single connected list.
            val segments = visible.size + 1
            Column(verticalArrangement = Arrangement.spacedBy(2.dp)) {
                visible.forEachIndexed { index, group ->
                    ModuleSummaryRow(
                        group = group,
                        stats = stats[group.module] ?: ModuleStats(0, 0, false),
                        index = index,
                        count = segments,
                        onOpen = { onOpenModule(group.module) },
                    )
                }
                ViewAllActivityRow(
                    index = visible.size,
                    count = segments,
                    moduleCount = groups.size,
                    onClick = onOpenAll,
                )
            }
        }
    }
}

/**
 * One module tile: an Expressive-shaped badge, the module name, up to [MODULE_ROW_EVENTS] recent
 * event lines (each led by a severity dot) and the latest time. A fixed number of lines per module
 * is what holds the overview height constant.
 */
@Composable
private fun ModuleSummaryRow(
    group: ModuleActivity,
    stats: ModuleStats,
    index: Int,
    count: Int,
    onOpen: () -> Unit,
) {
    val scheme = MaterialTheme.colorScheme
    val latest = remember(group.latestMs) {
        SimpleDateFormat("HH:mm", Locale.getDefault()).format(Date(group.latestMs))
    }
    val (iconContainer, iconTint) = moduleColors(group.tone)
    val (attentionContainer, attentionContent) = eventSeverityColors(
        if (stats.hasError) "error" else "warning",
        neutralForIconBox = false,
    )

    Surface(
        onClick = onOpen,
        modifier = Modifier.fillMaxWidth(),
        shape = segmentedShape(index, count),
        color = scheme.surfaceContainerLow,
    ) {
        Row(
            modifier = Modifier.padding(horizontal = 16.dp, vertical = 14.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(14.dp),
        ) {
            Box(
                modifier = Modifier
                    .size(46.dp)
                    .clip(moduleShape(group.tone))
                    .background(iconContainer),
                contentAlignment = Alignment.Center,
            ) {
                Icon(
                    moduleIcon(group.module),
                    contentDescription = null,
                    tint = iconTint,
                    modifier = Modifier.size(23.dp),
                )
            }
            Column(
                modifier = Modifier.weight(1f),
                verticalArrangement = Arrangement.spacedBy(3.dp),
            ) {
                Row(
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(8.dp),
                ) {
                    Text(
                        group.label,
                        modifier = Modifier.weight(1f, fill = false),
                        style = MaterialTheme.typography.titleMediumEmphasized,
                        color = scheme.onSurface,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                    )
                    if (stats.attention > 0) {
                        StatusPill(
                            text = "${stats.attention} 需关注",
                            container = attentionContainer,
                            content = attentionContent,
                        )
                    }
                }
                group.entries.forEach { entry ->
                    Row(
                        verticalAlignment = Alignment.CenterVertically,
                        horizontalArrangement = Arrangement.spacedBy(7.dp),
                    ) {
                        Box(
                            modifier = Modifier
                                .size(6.dp)
                                .clip(CircleShape)
                                .background(severityDotColor(entry.event.severity)),
                        )
                        Text(
                            buildString {
                                append(entry.label)
                                if (entry.detail.isNotBlank()) append(" · ${entry.detail}")
                                if (entry.count > 1) append(" · 共 ${entry.count} 次")
                            },
                            style = MaterialTheme.typography.bodySmall,
                            color = scheme.onSurfaceVariant,
                            maxLines = 1,
                            overflow = TextOverflow.Ellipsis,
                        )
                    }
                }
            }
            Column(
                horizontalAlignment = Alignment.End,
                verticalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                Text(
                    latest,
                    style = MaterialTheme.typography.labelSmall,
                    color = scheme.outline,
                )
                Icon(
                    Icons.Rounded.ChevronRight,
                    contentDescription = null,
                    tint = scheme.outline,
                    modifier = Modifier.size(20.dp),
                )
            }
        }
    }
}

/** Closing tile of the connected list; replaces the old footer link inside the card. */
@Composable
private fun ViewAllActivityRow(index: Int, count: Int, moduleCount: Int, onClick: () -> Unit) {
    val scheme = MaterialTheme.colorScheme
    Surface(
        onClick = onClick,
        modifier = Modifier.fillMaxWidth(),
        shape = segmentedShape(index, count),
        color = scheme.secondaryContainer,
        contentColor = scheme.onSecondaryContainer,
    ) {
        Row(
            modifier = Modifier.padding(horizontal = 20.dp, vertical = 14.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            Text(
                "查看全部系统动态",
                modifier = Modifier.weight(1f),
                style = MaterialTheme.typography.titleSmallEmphasized,
            )
            Text(
                "$moduleCount 个模块",
                style = MaterialTheme.typography.labelMedium,
                color = scheme.onSecondaryContainer.copy(alpha = 0.72f),
            )
            Icon(
                Icons.Rounded.ChevronRight,
                contentDescription = null,
                modifier = Modifier.size(20.dp),
            )
        }
    }
}

@Composable
private fun severityDotColor(severity: String): Color {
    val extended = MaterialTheme.sysFlowColors
    return when (severity) {
        "critical", "error" -> MaterialTheme.colorScheme.error
        "warning" -> extended.warning
        "notice" -> extended.info
        else -> MaterialTheme.colorScheme.outline
    }
}

@Composable
private fun EmptyActivityState() {
    Column(
        modifier = Modifier
            .fillMaxWidth()
            .padding(horizontal = 10.dp, vertical = 13.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        Box(
            modifier = Modifier
                .size(52.dp)
                .clip(MaterialShapes.Cookie6Sided.toShape())
                .background(MaterialTheme.colorScheme.secondaryContainer.copy(alpha = 0.72f)),
            contentAlignment = Alignment.Center,
        ) {
            Icon(
                Icons.Rounded.Dns,
                contentDescription = null,
                tint = MaterialTheme.colorScheme.onSecondaryContainer,
                modifier = Modifier.size(25.dp),
            )
        }
        Text(
            "暂无新的系统动态",
            style = MaterialTheme.typography.titleSmallEmphasized,
            color = MaterialTheme.colorScheme.onSurface,
        )
        Text(
            "当管理模块产生重要状态变化或策略事件时，会在这里统一呈现",
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            textAlign = androidx.compose.ui.text.style.TextAlign.Center,
        )
    }
}

@Composable
private fun RuntimeStatusCard(
    rootGranted: Boolean,
    protocol: Int,
    activeModules: List<String>,
    version: String,
) {
    Surface(
        shape = MaterialTheme.shapes.extraLarge,
        color = MaterialTheme.colorScheme.surfaceContainerLow,
        tonalElevation = 1.dp,
    ) {
        Column(
            modifier = Modifier
                .fillMaxWidth()
                .padding(horizontal = 18.dp, vertical = 12.dp),
        ) {
            RuntimeStatusRow(
                icon = Icons.Rounded.Shield,
                label = "权限状态",
                value = if (rootGranted) "Root 已授权" else "未授权",
                positive = rootGranted,
            )
            HorizontalDivider(color = MaterialTheme.colorScheme.outlineVariant.copy(alpha = 0.55f))
            RuntimeStatusRow(
                icon = Icons.Rounded.Dns,
                label = "模块协议",
                value = "Protocol v$protocol",
            )
            HorizontalDivider(color = MaterialTheme.colorScheme.outlineVariant.copy(alpha = 0.55f))
            RuntimeStatusRow(
                icon = Icons.Rounded.Bolt,
                label = "当前接入",
                value = if (activeModules.isEmpty()) "暂无模块" else "${activeModules.joinToString("、")} · v$version",
                positive = activeModules.isNotEmpty(),
            )
        }
    }
}

@Composable
private fun RuntimeStatusRow(
    icon: androidx.compose.ui.graphics.vector.ImageVector,
    label: String,
    value: String,
    positive: Boolean = false,
) {
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .height(48.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        Icon(
            icon,
            contentDescription = null,
            tint = MaterialTheme.colorScheme.onSurfaceVariant,
            modifier = Modifier.size(19.dp),
        )
        Text(
            label,
            modifier = Modifier.weight(1f),
            style = MaterialTheme.typography.bodyMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        StatusPill(
            text = value,
            container = if (positive) MaterialTheme.colorScheme.primaryContainer
            else MaterialTheme.colorScheme.surfaceContainerHighest,
            content = if (positive) MaterialTheme.colorScheme.onPrimaryContainer
            else MaterialTheme.colorScheme.onSurfaceVariant,
        )
    }
}

@Composable
private fun SafeBanner(onClearSafe: () -> Unit) {
    val colors = MaterialTheme.colorScheme
    val container by animateColorAsState(colors.errorContainer, label = "safeBanner")
    Surface(
        onClick = onClearSafe,
        modifier = Modifier.fillMaxWidth(),
        shape = MaterialTheme.shapes.large,
        color = container,
    ) {
        Row(
            modifier = Modifier.padding(16.dp),
            horizontalArrangement = Arrangement.spacedBy(12.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Icon(Icons.Rounded.Tune, contentDescription = null, tint = colors.onErrorContainer)
            Column(Modifier.weight(1f)) {
                Text("安全模式已开启", style = MaterialTheme.typography.titleSmall, color = colors.onErrorContainer)
                Text(
                    "调节功能受到限制，点按尝试退出安全模式",
                    style = MaterialTheme.typography.bodySmall,
                    color = colors.onErrorContainer.copy(alpha = 0.78f),
                )
            }
            Icon(Icons.Rounded.ChevronRight, contentDescription = null, tint = colors.onErrorContainer)
        }
    }
}

private class CpuSampleHolder {
    var sample: CpuSnapshot? = null
}

private data class CpuSummary(
    /** 0..1 total load from /proc/stat deltas; null until two samples exist. */
    val usage: Float?,
    val online: String,
    val coreStates: List<Boolean>,
    val allOnline: Boolean,
    val freq: String,
    val tempCelsius: Float?,
)

/** "47.6°C · 1.02 GHz" with the temperature tinted once it gets warm. */
@Composable
private fun cpuDetailText(summary: CpuSummary): AnnotatedString {
    val warn = MaterialTheme.sysFlowColors.warning
    val hot = MaterialTheme.colorScheme.error
    val temp = summary.tempCelsius
    val tempColor = when {
        temp == null -> Color.Unspecified
        temp >= 75f -> hot
        temp >= 60f -> warn
        else -> Color.Unspecified
    }
    return buildAnnotatedString {
        if (temp != null) {
            withStyle(SpanStyle(color = tempColor, fontWeight = FontWeight.SemiBold)) {
                append(String.format(Locale.US, "%.1f°C", temp))
            }
            append(" · ")
        }
        append(summary.freq)
    }
}

private fun moduleDisplayName(id: String): String = when (id) {
    "cpu" -> "CPU"
    "battery" -> "电池"
    "wakelock" -> "唤醒锁"
    "memory", "mem" -> "内存"
    "process", "proc" -> "进程"
    "storage", "io" -> "储存"
    else -> id.uppercase(Locale.ROOT)
}
