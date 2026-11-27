package io.sysflow.feature.cpuset

import io.sysflow.core.model.ItemType
import io.sysflow.core.model.Schema
import io.sysflow.core.model.SnapshotData
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.intOrNull
import kotlinx.serialization.json.jsonPrimitive

const val CPUSET_ITEM_PREFIX = "cpu.cpuset."
const val CPU_ONLINE_ITEM = "cpu.online"
const val CPU_CLUSTERS_ITEM = "cpu.clusters"

data class ProcessGroupUi(
    val id: String,
    val count: Int,
    val current: Set<Int>,
    val selected: Set<Int>,
    val configured: Set<Int>,
    val default: Set<Int>,
    val online: Set<Int>,
    val locked: Set<Int>,
    val dirty: Boolean,
) {
    val hasOnlineSelection: Boolean get() = selected.any { it in online }
    val valid: Boolean get() = selected.isNotEmpty() && hasOnlineSelection
}

data class ProcessGroupsUiState(
    val loading: Boolean = true,
    val groups: List<ProcessGroupUi> = emptyList(),
    val clusters: List<Pair<String, List<Int>>> = emptyList(),
    val onlineCount: Int = 0,
    val totalCores: Int = 0,
    val safeMode: Boolean = false,
    val pendingGroups: Set<String> = emptySet(),
    val resettingAll: Boolean = false,
    val message: String? = null,
    val error: String? = null,
) {
    val busy: Boolean get() = pendingGroups.isNotEmpty() || resettingAll
}

internal fun parseCpuSet(value: JsonElement?): Set<Int> =
    (value as? JsonArray)?.mapIndexedNotNull { index, item ->
        index.takeIf { item.jsonPrimitive.intOrNull == 1 }
    }?.toSet().orEmpty()

internal fun buildProcessGroupStates(
    schema: Schema,
    snapshot: SnapshotData,
    previous: List<ProcessGroupUi> = emptyList(),
): ProcessGroupsUiState {
    val cpu = snapshot.modules["cpu"] as? JsonObject ?: JsonObject(emptyMap())
    val settingsCpu = snapshot.settings?.get("cpu") as? JsonObject ?: JsonObject(emptyMap())
    val defaultsCpu = snapshot.defaults?.get("cpu") as? JsonObject ?: JsonObject(emptyMap())
    val onlineArray = cpu[CPU_ONLINE_ITEM] as? JsonArray
    val online = onlineArray?.mapIndexedNotNull { index, value ->
        index.takeIf { value.jsonPrimitive.intOrNull == 1 }
    }?.toSet().orEmpty()
    val clusters = (cpu[CPU_CLUSTERS_ITEM] as? JsonObject)
        ?.entries
        ?.mapNotNull { (id, value) ->
            val cpus = (value as? JsonArray)?.mapNotNull { it.jsonPrimitive.intOrNull }.orEmpty()
            if (cpus.isEmpty()) null else id to cpus
        }
        ?.sortedBy { it.first.toIntOrNull() ?: Int.MAX_VALUE }
        .orEmpty()

    val previousById = previous.associateBy { it.id }
    val groupOrder = listOf("top-app", "foreground", "system-background", "background")
        .withIndex()
        .associate { it.value to it.index }
    val groups = schema.items
        .filter { it.id.startsWith(CPUSET_ITEM_PREFIX) && it.type == ItemType.BoolList }
        .mapNotNull { item ->
            val liveArray = cpu[item.id] as? JsonArray ?: return@mapNotNull null
            val count = item.count ?: liveArray.size
            if (count <= 0) return@mapNotNull null
            val current = parseCpuSet(liveArray)
            val configured = parseCpuSet(settingsCpu[item.id]).ifEmpty { current }
            val default = parseCpuSet(defaultsCpu[item.id]).ifEmpty { current }
            val old = previousById[item.id]
            val selected = if (old?.dirty == true) old.selected else configured
            ProcessGroupUi(
                id = item.id.removePrefix(CPUSET_ITEM_PREFIX),
                count = count,
                current = current,
                selected = selected,
                configured = configured,
                default = default,
                online = online,
                locked = item.locked.toSet(),
                dirty = selected != configured,
            )
        }
        .sortedWith(
            compareBy<ProcessGroupUi> { groupOrder[it.id] ?: Int.MAX_VALUE }
                .thenBy { it.id },
        )

    return ProcessGroupsUiState(
        loading = false,
        groups = groups,
        clusters = clusters,
        onlineCount = online.size,
        totalCores = onlineArray?.size ?: groups.maxOfOrNull { it.count } ?: 0,
        safeMode = snapshot.safe_mode,
    )
}

internal fun encodeCpuSet(group: ProcessGroupUi, selected: Set<Int>): JsonArray =
    JsonArray(List(group.count) { index -> JsonPrimitive(if (index in selected) 1 else 0) })

/** Format a CPU id set as compact ranges, e.g. 0–3,6,8–9. */
fun formatCpuSet(cpus: Collection<Int>): String {
    val sorted = cpus.distinct().sorted()
    if (sorted.isEmpty()) return "—"
    val ranges = mutableListOf<String>()
    var start = sorted.first()
    var end = start
    for (cpu in sorted.drop(1)) {
        if (cpu == end + 1) {
            end = cpu
        } else {
            ranges += if (start == end) "$start" else "$start–$end"
            start = cpu
            end = cpu
        }
    }
    ranges += if (start == end) "$start" else "$start–$end"
    return ranges.joinToString(",")
}
