package io.sysflow.feature.screenoff

import io.sysflow.core.model.Schema
import io.sysflow.core.model.SnapshotData
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.intOrNull
import kotlinx.serialization.json.booleanOrNull
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.longOrNull
import kotlin.math.roundToLong

const val ITEM_SCREEN_OFF_ENABLED = "cpu.screen_off"
const val ITEM_SCREEN_OFF_MAX = "cpu.screen_off.max_freq"
const val ITEM_SCREEN_OFF_CORES = "cpu.screen_off.cores"
const val ITEM_SCREEN_OFF_ACTIVE = "cpu.screen_off_active"
const val ITEM_SCREEN_OFF_SAVED_ONLINE = "cpu.screen_off_saved_online"
const val ITEM_CPU_ONLINE = "cpu.online"
const val ITEM_CPU_CLUSTERS = "cpu.clusters"
const val ITEM_SCREEN_FREQ_TABLE = "cpu.screen_off.freq_table"
const val ITEM_CPU_HW_MIN = "cpu.hw_min_freq"
const val ITEM_CPU_HW_MAX = "cpu.hw_max_freq"
const val ITEM_CPU_MIN = "cpu.min_freq"
const val ITEM_CPU_MAX = "cpu.max_freq"

/** One policy's discrete screen-off ceiling choices. The final frequency detent is reserved for unlimited. */
data class ScreenOffPolicy(
    val id: String,
    val cpus: List<Int>,
    val steps: List<Long>,
    val hwMin: Long,
    val hwMax: Long,
    val liveMax: Long,
) {
    val usable: Boolean get() = steps.isNotEmpty()
}

data class ScreenOffUiState(
    val loading: Boolean = true,
    val enabled: Boolean = false,
    val active: Boolean = false,
    /** Reported by `daemon status`, not part of the CPU snapshot: liveness and screen detection. */
    val daemonRunning: Boolean = false,
    val screenState: String = "unknown",
    val policies: List<ScreenOffPolicy> = emptyList(),
    /** Policy id -> selected slider index; last index means unlimited. */
    val capIndices: Map<String, Int> = emptyMap(),
    val savedCaps: Map<String, Long> = emptyMap(),
    val coreCount: Int = 0,
    val online: Set<Int> = emptySet(),
    val baseOnline: Set<Int> = emptySet(),
    val locked: Set<Int> = emptySet(),
    val offCores: Set<Int> = emptySet(),
    val savedOffCores: Set<Int> = emptySet(),
    val pending: Boolean = false,
    val safeMode: Boolean = false,
    val message: String? = null,
    val error: String? = null,
) {
    val dirty: Boolean
        get() = offCores != savedOffCores || policies.any { policy ->
            if (!policy.usable) return@any false
            val index = capIndices[policy.id] ?: policy.steps.lastIndex.coerceAtLeast(0)
            val value = capValue(policy, index)
            value != (savedCaps[policy.id] ?: 0L)
        }

    val canSave: Boolean
        get() = dirty && !pending && !safeMode && (coreCount == 0 || baseOnline.any { it !in offCores })

    fun canClose(cpu: Int, selected: Set<Int> = offCores): Boolean {
        if (cpu in locked) return false
        if (cpu !in baseOnline) return cpu in selected
        return cpu in selected || baseOnline.any { it != cpu && it !in selected }
    }

    companion object {
        fun capValue(policy: ScreenOffPolicy, index: Int): Long =
            if (!policy.usable || index >= policy.steps.lastIndex) 0L
            else policy.steps.getOrNull(index.coerceAtLeast(0)) ?: 0L
    }
}

internal fun parseScreenOffState(
    schema: Schema,
    snapshot: SnapshotData,
): ScreenOffUiState {
    val cpu = snapshot.modules["cpu"] as? JsonObject ?: JsonObject(emptyMap())
    val settings = snapshot.settings?.get("cpu") as? JsonObject
    val defaults = snapshot.defaults?.get("cpu") as? JsonObject

    val clusterMap = cpu.objectAt(ITEM_CPU_CLUSTERS)
    val freqTable = cpu.objectAt(ITEM_SCREEN_FREQ_TABLE)
    val hwMin = cpu.longMapAt(ITEM_CPU_HW_MIN)
    val hwMax = cpu.longMapAt(ITEM_CPU_HW_MAX)
    val liveMin = cpu.longMapAt(ITEM_CPU_MIN)
    val liveMax = cpu.longMapAt(ITEM_CPU_MAX)
    val savedMin = (settings?.get(ITEM_CPU_MIN) ?: defaults?.get(ITEM_CPU_MIN)).asLongMap()
    val savedMax = (settings?.get(ITEM_CPU_MAX) ?: defaults?.get(ITEM_CPU_MAX)).asLongMap()
    val rawCaps = cpu.objectAt(ITEM_SCREEN_OFF_MAX)
        .entries.mapNotNull { (key, value) -> value.jsonPrimitive.longOrNull?.let { key to it } }.toMap()

    val policies = clusterMap.entries.mapNotNull { (id, value) ->
        val cpus = (value as? JsonArray)?.mapNotNull { it.jsonPrimitive.intOrNull }.orEmpty()
        if (cpus.isEmpty()) return@mapNotNull null
        val low = hwMin[id]?.takeIf { it > 0L } ?: savedMin[id] ?: liveMin[id] ?: 0L
        val high = hwMax[id]?.takeIf { it > 0L } ?: savedMax[id] ?: liveMax[id] ?: 0L
        val available = freqTable[id].asLongList().distinct().sorted()
        val steps = makeScreenOffSteps(available, low, high)
        ScreenOffPolicy(id, cpus, steps, low, high, liveMax[id] ?: 0L)
    }.sortedBy { it.id.toIntOrNull() ?: Int.MAX_VALUE }

    val coreSchema = schema.items.firstOrNull { it.id == ITEM_SCREEN_OFF_CORES }
    val coreCount = coreSchema?.count ?: 0
    val online = boolIndices(cpu[ITEM_CPU_ONLINE], coreCount)
    val settingsOnline = settings?.get(ITEM_CPU_ONLINE) ?: defaults?.get(ITEM_CPU_ONLINE)
    val savedOnline = boolIndices(cpu[ITEM_SCREEN_OFF_SAVED_ONLINE], coreCount)
    val baseOnline = when {
        cpu[ITEM_SCREEN_OFF_ACTIVE]?.jsonPrimitive?.booleanOrNull == true && savedOnline.isNotEmpty() -> savedOnline
        else -> boolIndices(settingsOnline, coreCount).ifEmpty { online }
    }
    val locked = coreSchema?.locked?.toSet().orEmpty()
    val offCores = boolIndices(cpu[ITEM_SCREEN_OFF_CORES], coreCount)

    val caps = policies.associate { it.id to (rawCaps[it.id] ?: 0L) }
    val indices = policies.associate { policy ->
        val value = caps[policy.id] ?: 0L
        val index = when {
            policy.steps.isEmpty() -> 0
            value <= 0L -> policy.steps.lastIndex
            else -> nearestIndex(policy.steps, value)
        }
        policy.id to index
    }

    return ScreenOffUiState(
        loading = false,
        enabled = cpu[ITEM_SCREEN_OFF_ENABLED]?.jsonPrimitive?.booleanOrNull ?: false,
        active = cpu[ITEM_SCREEN_OFF_ACTIVE]?.jsonPrimitive?.booleanOrNull ?: false,
        policies = policies,
        capIndices = indices,
        savedCaps = policies.associate { policy ->
            val index = indices[policy.id] ?: policy.steps.lastIndex.coerceAtLeast(0)
            policy.id to if (policy.usable) ScreenOffUiState.capValue(policy, index) else (caps[policy.id] ?: 0L)
        },
        coreCount = coreCount,
        online = online,
        baseOnline = baseOnline,
        locked = locked,
        offCores = offCores,
        savedOffCores = offCores,
    )
}

private fun makeScreenOffSteps(freqs: List<Long>, low: Long, high: Long): List<Long> {
    if (freqs.size > 1) return freqs.filter { it > 0L }.distinct().sorted()
    val min = low.coerceAtLeast(0L)
    val max = high.coerceAtLeast(0L)
    if (max == 0L || max < min) return emptyList()
    if (max == min) return listOf(min)
    // Match SoCFlow: when a kernel omits the frequency table, generate 24 evenly spaced
    // positions rounded to 1 MHz. The module validates these against this policy's hw range.
    return buildSet {
        for (step in 0..24) {
            val raw = min + (max - min).toDouble() * step / 24.0
            add(((raw / 1000.0).roundToLong() * 1000L).coerceIn(min, max))
        }
    }.toList().sorted()
}

private fun nearestIndex(values: List<Long>, target: Long): Int {
    var best = 0
    var distance = Long.MAX_VALUE
    values.forEachIndexed { index, value ->
        val next = kotlin.math.abs(value - target)
        if (next < distance) {
            best = index
            distance = next
        }
    }
    return best
}

private fun JsonObject.objectAt(key: String): JsonObject = this[key] as? JsonObject ?: JsonObject(emptyMap())
private fun JsonElement?.asLongMap(): Map<String, Long> = (this as? JsonObject)?.entries?.mapNotNull { (k, v) ->
    v.jsonPrimitive.longOrNull?.let { k to it }
}?.toMap().orEmpty()
private fun JsonObject.longMapAt(key: String): Map<String, Long> = objectAt(key).entries.mapNotNull { (k, v) ->
    v.jsonPrimitive.longOrNull?.let { k to it }
}.toMap()
private fun JsonElement?.asLongList(): List<Long> = (this as? JsonArray)?.mapNotNull { it.jsonPrimitive.longOrNull }.orEmpty()
private fun boolIndices(value: JsonElement?, count: Int): Set<Int> = (value as? JsonArray)
    ?.mapIndexedNotNull { index, item -> index.takeIf { index < count && item.jsonPrimitive.intOrNull == 1 } }
    ?.toSet().orEmpty()
