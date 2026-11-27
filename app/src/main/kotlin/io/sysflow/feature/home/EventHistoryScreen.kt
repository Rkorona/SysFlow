@file:OptIn(
    ExperimentalMaterial3Api::class,
    ExperimentalMaterial3ExpressiveApi::class,
    ExperimentalFoundationApi::class,
    ExperimentalLayoutApi::class,
)

package io.sysflow.feature.home

import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.animateColorAsState
import androidx.compose.animation.core.FiniteAnimationSpec
import androidx.compose.animation.expandVertically
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.shrinkVertically
import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.background
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.ArrowDropDown
import androidx.compose.material.icons.rounded.Category
import androidx.compose.material.icons.rounded.Check
import androidx.compose.material.icons.rounded.Close
import androidx.compose.material.icons.rounded.History
import androidx.compose.material.icons.rounded.Memory
import androidx.compose.material.icons.rounded.Refresh
import androidx.compose.material.icons.rounded.Warning
import androidx.compose.material3.AssistChip
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.ExperimentalMaterial3ExpressiveApi
import androidx.compose.material3.FilledIconButton
import androidx.compose.material3.FilledTonalButton
import androidx.compose.material3.FilterChip
import androidx.compose.material3.Icon
import androidx.compose.material3.LoadingIndicator
import androidx.compose.material3.LocalContentColor
import androidx.compose.material3.MaterialShapes
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.material3.toShape
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.derivedStateOf
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Shape
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.IntSize
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import io.sysflow.core.event.EventRepository
import io.sysflow.core.event.SystemEventEntity
import io.sysflow.core.ui.components.InfoPill
import io.sysflow.core.ui.components.StatusPill
import io.sysflow.core.ui.components.segmentedShape
import kotlinx.coroutines.launch
import java.text.SimpleDateFormat
import java.util.Calendar
import java.util.Date
import java.util.Locale
import kotlin.math.abs
import kotlin.math.roundToLong

/**
 * System events, laid out the Material 3 Expressive way:
 *
 *  - a tonal summary card with the running count and the sync action,
 *  - filter chips that open a bottom sheet instead of three dropdown menus,
 *  - events grouped by day under sticky headers, each day one connected "segmented" list,
 *  - repeated records collapsed into a single row with a count,
 *  - rows that expand in place to show the raw fields.
 */
@Composable
fun EventHistoryScreen(
    repository: EventRepository,
    initialModule: String? = null,
) {
    var selectedModule by rememberSaveable { mutableStateOf(initialModule) }
    var selectedSeverity by rememberSaveable { mutableStateOf<String?>(null) }
    var selectedCategory by rememberSaveable { mutableStateOf<String?>(null) }
    var limit by rememberSaveable { mutableStateOf(PAGE_SIZE) }
    var expandedKey by rememberSaveable { mutableStateOf<String?>(null) }
    var picker by remember { mutableStateOf<FilterKind?>(null) }
    var syncing by remember { mutableStateOf(false) }
    val scope = rememberCoroutineScope()
    val listState = rememberLazyListState()

    val modules by repository.observeModules().collectAsStateWithLifecycle(initialValue = emptyList())
    val categories by repository.observeCategories().collectAsStateWithLifecycle(initialValue = emptyList())
    val events by repository.observeFiltered(
        module = selectedModule,
        severity = selectedSeverity,
        category = selectedCategory,
        limit = limit,
        offset = 0,
    ).collectAsStateWithLifecycle(initialValue = emptyList())
    val count by repository.observeFilteredCount(
        module = selectedModule,
        category = selectedCategory,
        severity = selectedSeverity,
    ).collectAsStateWithLifecycle(initialValue = 0)

    val groups = remember(events) { buildDayGroups(events) }
    val hasFilter = selectedModule != null || selectedSeverity != null || selectedCategory != null

    fun sync() {
        scope.launch {
            syncing = true
            runCatching { repository.syncLatest(force = true) }
            syncing = false
        }
    }

    fun resetPaging() {
        limit = PAGE_SIZE
        expandedKey = null
    }

    LaunchedEffect(Unit) { sync() }

    // Grow the window as the reader nears the end; the list never needs a pager.
    val nearEnd by remember {
        derivedStateOf {
            val info = listState.layoutInfo
            val last = info.visibleItemsInfo.lastOrNull()?.index ?: 0
            last >= info.totalItemsCount - 6
        }
    }
    LaunchedEffect(nearEnd, events.size, count, limit) {
        if (nearEnd && events.size >= limit && events.size < count) limit += PAGE_SIZE
    }

    val todayStart = remember(events) { dayStart(System.currentTimeMillis()) }

    LazyColumn(
        state = listState,
        modifier = Modifier.fillMaxSize(),
        contentPadding = PaddingValues(start = 20.dp, end = 20.dp, top = 8.dp, bottom = 32.dp),
        verticalArrangement = Arrangement.spacedBy(2.dp),
    ) {
        item(key = "summary") {
            SummaryCard(
                count = count,
                filtered = hasFilter,
                syncing = syncing,
                onSync = ::sync,
                modifier = Modifier.padding(bottom = 12.dp),
            )
        }
        item(key = "filters") {
            FilterBar(
                module = selectedModule,
                category = selectedCategory,
                severity = selectedSeverity,
                onPick = { picker = it },
                onClear = {
                    selectedModule = null
                    selectedCategory = null
                    selectedSeverity = null
                    resetPaging()
                },
                modifier = Modifier.padding(bottom = 4.dp),
            )
        }

        if (events.isEmpty()) {
            item(key = "empty") {
                Box(
                    modifier = Modifier.fillMaxWidth().padding(vertical = 56.dp),
                    contentAlignment = Alignment.Center,
                ) {
                    if (syncing) {
                        LoadingIndicator()
                    } else {
                        EmptyState(
                            filtered = hasFilter,
                            onClear = {
                                selectedModule = null
                                selectedCategory = null
                                selectedSeverity = null
                                resetPaging()
                            },
                        )
                    }
                }
            }
        }

        groups.forEach { group ->
            stickyHeader(key = "day-${group.dayStartMs}") {
                DayHeader(
                    label = dayLabel(group.dayStartMs, todayStart),
                    count = group.eventCount,
                )
            }
            itemsIndexed(group.rows, key = { _, row -> row.key }) { index, row ->
                EventRow(
                    row = row,
                    index = index,
                    count = group.rows.size,
                    expanded = expandedKey == row.key,
                    showModule = selectedModule == null,
                    onClick = { expandedKey = if (expandedKey == row.key) null else row.key },
                    modifier = Modifier.animateItem(),
                )
            }
        }

        if (events.isNotEmpty()) {
            item(key = "footer") {
                ListFooter(
                    shown = events.size,
                    total = count,
                    onLoadMore = { limit += PAGE_SIZE },
                )
            }
        }
    }

    picker?.let { kind ->
        val (options, current) = when (kind) {
            FilterKind.Module -> modules.map { it to moduleLabel(it) } to selectedModule
            FilterKind.Category -> categories.map { it to it } to selectedCategory
            FilterKind.Severity -> SEVERITIES.map { it to eventSeverityLabel(it) } to selectedSeverity
        }
        FilterSheet(
            title = kind.title,
            allLabel = kind.allLabel,
            options = options,
            current = current,
            onSelect = { value ->
                when (kind) {
                    FilterKind.Module -> selectedModule = value
                    FilterKind.Category -> selectedCategory = value
                    FilterKind.Severity -> selectedSeverity = value
                }
                resetPaging()
                picker = null
            },
            onDismiss = { picker = null },
        )
    }
}

// ───────────────────────────── Header: summary + filters ─────────────────────────────

@Composable
private fun SummaryCard(
    count: Int,
    filtered: Boolean,
    syncing: Boolean,
    onSync: () -> Unit,
    modifier: Modifier = Modifier,
) {
    Surface(
        modifier = modifier.fillMaxWidth(),
        shape = MaterialTheme.shapes.extraLarge,
        color = MaterialTheme.colorScheme.primaryContainer,
        contentColor = MaterialTheme.colorScheme.onPrimaryContainer,
    ) {
        Row(
            modifier = Modifier.padding(horizontal = 20.dp, vertical = 18.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(16.dp),
        ) {
            Box(
                modifier = Modifier
                    .size(60.dp)
                    .clip(MaterialShapes.Cookie9Sided.toShape())
                    .background(MaterialTheme.colorScheme.primary),
                contentAlignment = Alignment.Center,
            ) {
                Icon(
                    Icons.Rounded.History,
                    contentDescription = null,
                    tint = MaterialTheme.colorScheme.onPrimary,
                    modifier = Modifier.size(28.dp),
                )
            }
            Column(modifier = Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(2.dp)) {
                Text(
                    if (filtered) "筛选结果" else "本地历史",
                    style = MaterialTheme.typography.labelLarge,
                )
                Row(verticalAlignment = Alignment.Bottom, horizontalArrangement = Arrangement.spacedBy(4.dp)) {
                    Text("$count", style = MaterialTheme.typography.headlineLargeEmphasized)
                    Text(
                        "条事件",
                        style = MaterialTheme.typography.titleSmall,
                        modifier = Modifier.padding(bottom = 6.dp),
                    )
                }
                Text(
                    "即使 APK 关闭，Root 模块仍会继续采集",
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onPrimaryContainer.copy(alpha = 0.72f),
                )
            }
            FilledIconButton(onClick = onSync, enabled = !syncing) {
                if (syncing) {
                    LoadingIndicator(
                        modifier = Modifier.size(28.dp),
                        color = LocalContentColor.current,
                    )
                } else {
                    Icon(Icons.Rounded.Refresh, contentDescription = "同步")
                }
            }
        }
    }
}

private enum class FilterKind(val title: String, val allLabel: String) {
    Module("选择模块", "全部模块"),
    Category("选择类别", "全部类别"),
    Severity("选择等级", "全部等级"),
}

@Composable
private fun FilterBar(
    module: String?,
    category: String?,
    severity: String?,
    onPick: (FilterKind) -> Unit,
    onClear: () -> Unit,
    modifier: Modifier = Modifier,
) {
    Row(
        modifier = modifier.fillMaxWidth().horizontalScroll(rememberScrollState()),
        horizontalArrangement = Arrangement.spacedBy(8.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        FilterDropChip(
            label = module?.let(::moduleLabel) ?: FilterKind.Module.allLabel,
            selected = module != null,
            icon = Icons.Rounded.Memory,
            onClick = { onPick(FilterKind.Module) },
        )
        FilterDropChip(
            label = category ?: FilterKind.Category.allLabel,
            selected = category != null,
            icon = Icons.Rounded.Category,
            onClick = { onPick(FilterKind.Category) },
        )
        FilterDropChip(
            label = severity?.let(::eventSeverityLabel) ?: FilterKind.Severity.allLabel,
            selected = severity != null,
            icon = Icons.Rounded.Warning,
            onClick = { onPick(FilterKind.Severity) },
        )
        if (module != null || category != null || severity != null) {
            AssistChip(
                onClick = onClear,
                label = { Text("清除") },
                leadingIcon = { Icon(Icons.Rounded.Close, null, Modifier.size(18.dp)) },
            )
        }
    }
}

@Composable
private fun FilterDropChip(
    label: String,
    selected: Boolean,
    icon: androidx.compose.ui.graphics.vector.ImageVector,
    onClick: () -> Unit,
) {
    FilterChip(
        selected = selected,
        onClick = onClick,
        label = { Text(label, maxLines = 1, overflow = TextOverflow.Ellipsis) },
        leadingIcon = {
            Icon(
                if (selected) Icons.Rounded.Check else icon,
                contentDescription = null,
                modifier = Modifier.size(18.dp),
            )
        },
        trailingIcon = {
            Icon(Icons.Rounded.ArrowDropDown, contentDescription = null, modifier = Modifier.size(18.dp))
        },
    )
}

@Composable
private fun FilterSheet(
    title: String,
    allLabel: String,
    options: List<Pair<String, String>>,
    current: String?,
    onSelect: (String?) -> Unit,
    onDismiss: () -> Unit,
) {
    ModalBottomSheet(
        onDismissRequest = onDismiss,
        sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true),
    ) {
        Column(
            modifier = Modifier
                .fillMaxWidth()
                .verticalScroll(rememberScrollState())
                .padding(start = 24.dp, end = 24.dp, bottom = 24.dp),
            verticalArrangement = Arrangement.spacedBy(16.dp),
        ) {
            Text(title, style = MaterialTheme.typography.titleLargeEmphasized)
            FlowRow(
                horizontalArrangement = Arrangement.spacedBy(8.dp),
                verticalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                SheetChip(allLabel, selected = current == null) { onSelect(null) }
                options.forEach { (value, label) ->
                    SheetChip(label, selected = current == value) { onSelect(value) }
                }
            }
        }
    }
}

@Composable
private fun SheetChip(label: String, selected: Boolean, onClick: () -> Unit) {
    FilterChip(
        selected = selected,
        onClick = onClick,
        label = { Text(label) },
        leadingIcon = if (selected) {
            { Icon(Icons.Rounded.Check, contentDescription = null, modifier = Modifier.size(18.dp)) }
        } else {
            null
        },
    )
}

// ───────────────────────────── Day groups ─────────────────────────────

@Composable
private fun DayHeader(label: String, count: Int) {
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .background(MaterialTheme.colorScheme.background)
            .padding(start = 8.dp, end = 8.dp, top = 14.dp, bottom = 8.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Text(
            label,
            modifier = Modifier.weight(1f),
            style = MaterialTheme.typography.titleSmallEmphasized,
            color = MaterialTheme.colorScheme.onSurface,
        )
        Text(
            "$count 条",
            style = MaterialTheme.typography.labelMedium,
            color = MaterialTheme.colorScheme.outline,
        )
    }
}

// ───────────────────────────── Event row ─────────────────────────────

@Composable
private fun EventRow(
    row: EventRowModel,
    index: Int,
    count: Int,
    expanded: Boolean,
    showModule: Boolean,
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val event = row.head
    val scheme = MaterialTheme.colorScheme
    val (badgeContainer, badgeTint) = eventSeverityColors(event.severity, neutralForIconBox = true)
    val (pillContainer, pillContent) = eventSeverityColors(event.severity, neutralForIconBox = false)
    val title = remember(event.id) { activityLabel(event).ifBlank { event.title } }
    val detail = remember(event.id) { activityDetail(event) }
    val time = remember(event.id) { formatClock(event.timestampMs) }
    val container by animateColorAsState(
        targetValue = if (expanded) scheme.surfaceContainerHigh else scheme.surfaceContainerLow,
        animationSpec = MaterialTheme.motionScheme.defaultEffectsSpec(),
        label = "eventRowContainer",
    )
    val spatial: FiniteAnimationSpec<IntSize> = MaterialTheme.motionScheme.defaultSpatialSpec()
    val effects: FiniteAnimationSpec<Float> = MaterialTheme.motionScheme.defaultEffectsSpec()

    Surface(
        onClick = onClick,
        modifier = modifier.fillMaxWidth(),
        shape = segmentedShape(index, count),
        color = container,
    ) {
        Column(modifier = Modifier.padding(horizontal = 16.dp, vertical = 14.dp)) {
            Row(
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(14.dp),
            ) {
                Box(
                    modifier = Modifier
                        .size(48.dp)
                        .clip(severityShape(event.severity))
                        .background(badgeContainer),
                    contentAlignment = Alignment.Center,
                ) {
                    Icon(
                        eventCategoryIcon(event.category),
                        contentDescription = null,
                        tint = badgeTint,
                        modifier = Modifier.size(24.dp),
                    )
                }
                Column(modifier = Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(2.dp)) {
                    if (showModule) {
                        Text(
                            moduleLabel(event.module),
                            style = MaterialTheme.typography.labelSmall,
                            color = scheme.primary,
                            maxLines = 1,
                            overflow = TextOverflow.Ellipsis,
                        )
                    }
                    Text(
                        title,
                        style = MaterialTheme.typography.titleMediumEmphasized,
                        color = scheme.onSurface,
                        maxLines = 2,
                        overflow = TextOverflow.Ellipsis,
                    )
                    if (detail.isNotBlank()) {
                        Text(
                            detail,
                            style = MaterialTheme.typography.bodySmall,
                            color = scheme.onSurfaceVariant,
                            maxLines = if (expanded) Int.MAX_VALUE else 1,
                            overflow = TextOverflow.Ellipsis,
                        )
                    }
                }
                Column(horizontalAlignment = Alignment.End, verticalArrangement = Arrangement.spacedBy(6.dp)) {
                    Text(
                        time,
                        style = MaterialTheme.typography.labelSmall,
                        color = scheme.outline,
                    )
                    val showSeverity = event.severity != "info"
                    if (showSeverity || row.events.size > 1) {
                        Row(horizontalArrangement = Arrangement.spacedBy(4.dp)) {
                            if (showSeverity) {
                                StatusPill(
                                    text = eventSeverityLabel(event.severity),
                                    container = pillContainer,
                                    content = pillContent,
                                )
                            }
                            if (row.events.size > 1) {
                                InfoPill(
                                    text = "×${row.events.size}",
                                    container = scheme.secondaryContainer,
                                    content = scheme.onSecondaryContainer,
                                )
                            }
                        }
                    }
                }
            }
            AnimatedVisibility(
                visible = expanded,
                enter = expandVertically(spatial) + fadeIn(effects),
                exit = shrinkVertically(spatial) + fadeOut(effects),
            ) {
                EventDetail(row = row, summary = detail)
            }
        }
    }
}

@Composable
private fun EventDetail(row: EventRowModel, summary: String) {
    val event = row.head
    val scheme = MaterialTheme.colorScheme
    val metadata = remember(event.id, event.metadataJson) {
        formatEventMetadata(event.metadataJson)
            ?.split(" · ")
            ?.map(String::trim)
            ?.filter(String::isNotEmpty)
            .orEmpty()
    }
    val rows = remember(event.id) {
        buildList {
            add("模块" to moduleLabel(event.module))
            add("类别" to event.category)
            add("类型" to event.eventType)
            add("时间" to formatFull(event.timestampMs))
            if (event.message.isNotBlank() && event.message != summary) add("原文" to event.message)
        }
    }

    Column(
        modifier = Modifier.padding(top = 14.dp),
        verticalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        if (metadata.isNotEmpty()) {
            FlowRow(
                horizontalArrangement = Arrangement.spacedBy(6.dp),
                verticalArrangement = Arrangement.spacedBy(6.dp),
            ) {
                metadata.forEach { item ->
                    InfoPill(item, scheme.secondaryContainer, scheme.onSecondaryContainer)
                }
            }
        }
        if (row.events.size > 1) {
            val shown = row.events.take(MAX_OCCURRENCES)
            Column(verticalArrangement = Arrangement.spacedBy(6.dp)) {
                Text(
                    "连续发生 ${row.events.size} 次",
                    style = MaterialTheme.typography.labelMedium,
                    color = scheme.onSurfaceVariant,
                )
                FlowRow(
                    horizontalArrangement = Arrangement.spacedBy(6.dp),
                    verticalArrangement = Arrangement.spacedBy(6.dp),
                ) {
                    shown.forEach { e ->
                        InfoPill(formatClock(e.timestampMs), scheme.surfaceContainerHighest, scheme.onSurfaceVariant)
                    }
                    if (row.events.size > shown.size) {
                        InfoPill("…", scheme.surfaceContainerHighest, scheme.onSurfaceVariant)
                    }
                }
            }
        }
        Column(verticalArrangement = Arrangement.spacedBy(2.dp)) {
            rows.forEachIndexed { i, (label, value) ->
                Row(
                    modifier = Modifier
                        .fillMaxWidth()
                        .clip(detailShape(i, rows.size))
                        .background(scheme.surfaceContainerHighest)
                        .padding(horizontal = 14.dp, vertical = 10.dp),
                    horizontalArrangement = Arrangement.spacedBy(12.dp),
                ) {
                    Text(
                        label,
                        modifier = Modifier.width(36.dp),
                        style = MaterialTheme.typography.bodySmall,
                        color = scheme.onSurfaceVariant,
                    )
                    Text(
                        value,
                        modifier = Modifier.weight(1f),
                        style = MaterialTheme.typography.bodySmall,
                        color = scheme.onSurface,
                    )
                }
            }
        }
    }
}

// ───────────────────────────── Footer / empty ─────────────────────────────

@Composable
private fun ListFooter(shown: Int, total: Int, onLoadMore: () -> Unit) {
    Box(
        modifier = Modifier.fillMaxWidth().padding(top = 16.dp),
        contentAlignment = Alignment.Center,
    ) {
        when {
            shown < total -> FilledTonalButton(onClick = onLoadMore) {
                Text("加载更多 · 已显示 $shown / $total")
            }
            total > PAGE_SIZE -> Text(
                "已显示全部 $total 条",
                style = MaterialTheme.typography.labelMedium,
                color = MaterialTheme.colorScheme.outline,
            )
        }
    }
}

@Composable
private fun EmptyState(filtered: Boolean, onClear: () -> Unit) {
    Column(
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        Box(
            modifier = Modifier
                .size(96.dp)
                .clip(MaterialShapes.Cookie12Sided.toShape())
                .background(MaterialTheme.colorScheme.surfaceContainerHighest),
            contentAlignment = Alignment.Center,
        ) {
            Icon(
                Icons.Rounded.History,
                contentDescription = null,
                tint = MaterialTheme.colorScheme.onSurfaceVariant,
                modifier = Modifier.size(40.dp),
            )
        }
        Text(
            "暂无符合条件的事件",
            style = MaterialTheme.typography.titleMediumEmphasized,
            modifier = Modifier.padding(top = 8.dp),
        )
        Text(
            "事件由 Root 模块产生并同步到本地",
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.outline,
            textAlign = TextAlign.Center,
        )
        if (filtered) TextButton(onClick = onClear) { Text("清除筛选") }
    }
}

// ───────────────────────────── Shapes & grouping ─────────────────────────────

@Composable
private fun severityShape(severity: String): Shape = when (severity) {
    "critical", "error" -> MaterialShapes.Burst.toShape()
    "warning" -> MaterialShapes.Sunny.toShape()
    "notice" -> MaterialShapes.Clover4Leaf.toShape()
    else -> MaterialShapes.Cookie9Sided.toShape()
}

private fun detailShape(index: Int, count: Int): Shape {
    val outer = 16.dp
    val inner = 4.dp
    val top = if (index == 0) outer else inner
    val bottom = if (index == count - 1) outer else inner
    return RoundedCornerShape(topStart = top, topEnd = top, bottomStart = bottom, bottomEnd = bottom)
}

/** One visible row: a single event, or several identical ones that arrived back to back. */
private data class EventRowModel(val key: String, val events: List<SystemEventEntity>) {
    val head: SystemEventEntity get() = events.first()
}

private data class DayGroup(val dayStartMs: Long, val rows: List<EventRowModel>, val eventCount: Int)

private fun buildDayGroups(events: List<SystemEventEntity>): List<DayGroup> {
    if (events.isEmpty()) return emptyList()
    val byDay = LinkedHashMap<Long, MutableList<SystemEventEntity>>()
    events.forEach { byDay.getOrPut(dayStart(it.timestampMs)) { mutableListOf() } += it }
    return byDay.map { (day, list) -> DayGroup(day, collapseRows(list), list.size) }
}

/** Events arrive newest first; neighbours that report the same thing within a minute are one row. */
private fun collapseRows(events: List<SystemEventEntity>): List<EventRowModel> {
    val rows = mutableListOf<MutableList<SystemEventEntity>>()
    for (event in events) {
        val current = rows.lastOrNull()
        if (current != null && isSameOccurrence(current.last(), event)) {
            current += event
        } else {
            rows += mutableListOf(event)
        }
    }
    return rows.map { EventRowModel(key = it.first().id, events = it) }
}

private fun isSameOccurrence(a: SystemEventEntity, b: SystemEventEntity): Boolean =
    a.module == b.module &&
        a.eventType == b.eventType &&
        a.severity == b.severity &&
        a.message == b.message &&
        abs(a.timestampMs - b.timestampMs) <= COLLAPSE_WINDOW_MS &&
        formatEventMetadata(a.metadataJson) == formatEventMetadata(b.metadataJson)

private fun dayStart(timestampMs: Long): Long = Calendar.getInstance().apply {
    timeInMillis = timestampMs
    set(Calendar.HOUR_OF_DAY, 0)
    set(Calendar.MINUTE, 0)
    set(Calendar.SECOND, 0)
    set(Calendar.MILLISECOND, 0)
}.timeInMillis

private fun dayLabel(dayStartMs: Long, todayStartMs: Long): String {
    val locale = Locale.getDefault()
    val date = SimpleDateFormat("M月d日", locale).format(Date(dayStartMs))
    return when (((todayStartMs - dayStartMs) / DAY_MS.toDouble()).roundToLong()) {
        0L -> "今天 · $date"
        1L -> "昨天 · $date"
        else -> "$date · ${SimpleDateFormat("EEEE", locale).format(Date(dayStartMs))}"
    }
}

private fun formatClock(ms: Long): String =
    SimpleDateFormat("HH:mm:ss", Locale.getDefault()).format(Date(ms))

private fun formatFull(ms: Long): String =
    SimpleDateFormat("yyyy-MM-dd HH:mm:ss", Locale.getDefault()).format(Date(ms))

private const val PAGE_SIZE = 50
private const val MAX_OCCURRENCES = 12
private const val COLLAPSE_WINDOW_MS = 60_000L
private const val DAY_MS = 86_400_000L
private val SEVERITIES = listOf("info", "notice", "warning", "error", "critical")
