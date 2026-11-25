package io.sysflow.core.model

import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonArray
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.booleanOrNull
import kotlinx.serialization.json.intOrNull
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.longOrNull
import kotlinx.serialization.json.put
import java.util.Locale
import kotlin.math.abs
import kotlin.math.roundToLong

const val ITEM_ONLINE = "cpu.online"
const val ITEM_MAX = "cpu.max_freq"
const val ITEM_MIN = "cpu.min_freq"
const val ITEM_GOV = "cpu.governor"
const val ITEM_CUR = "cpu.cur_freq"
const val ITEM_CLUSTERS = "cpu.clusters"
const val ITEM_CLUSTER_CUR = "cpu.cluster_cur_freq"
const val ITEM_CLUSTER_HW_CUR = "cpu.cluster_hw_cur_freq"
const val ITEM_TIME_IN_STATE = "cpu.time_in_state"
const val ITEM_FREQ_TABLE = "cpu.freq_table"
const val ITEM_GOVERNOR_TABLE = "cpu.governor_table"
const val ITEM_USAGE_STAT = "cpu.usage_stat"
const val ITEM_HW_MIN = "cpu.hw_min_freq"
const val ITEM_HW_MAX = "cpu.hw_max_freq"
const val ITEM_TEMP = "cpu.temp"

data class CpuSnapshot(
    val online: List<Int>,
    val clusters: Map<String, List<Int>>,
    val maxFreq: Map<String, Long>,
    val minFreq: Map<String, Long>,
    val governor: Map<String, String>,
    val curFreq: Map<String, Long>,
    val clusterCurFreq: Map<String, Long>,
    val clusterHwCurFreq: Map<String, Long>,
    val timeInState: Map<String, Map<Long, Long>>,
    val frequencyTable: Map<String, List<Long>>,
    val governorTable: Map<String, List<String>>,
    val usageStat: Map<String, CpuUsageCounter>,
    val hwMin: Map<String, Long>,
    val hwMax: Map<String, Long>,
    /** User-saved static limits, distinct from live sysfs values that adaptive control may cap. */
    val configuredMinFreq: Map<String, Long>,
    val configuredMaxFreq: Map<String, Long>,
    val configuredGovernor: Map<String, String>,
    /** First-run defaults captured by the module manager for per-cluster restore. */
    val defaultMinFreq: Map<String, Long>,
    val defaultMaxFreq: Map<String, Long>,
    val defaultGovernor: Map<String, String>,
    val adaptiveEnabled: Boolean,
    val screenOffActive: Boolean,
    val tempDeciCelsius: Long?,
    val raw: JsonObject,
) {
    val onlineCount: Int get() = online.count { it == 1 }

    fun staticMin(id: String): Long = configuredMinFreq[id]
        ?: defaultMinFreq[id] ?: hwMin[id] ?: minFreq[id] ?: 0L

    fun staticMax(id: String): Long = configuredMaxFreq[id]
        ?: defaultMaxFreq[id] ?: hwMax[id] ?: maxFreq[id] ?: 0L

    fun resetMin(id: String): Long = defaultMinFreq[id] ?: hwMin[id] ?: minFreq[id] ?: 0L

    fun resetMax(id: String): Long = defaultMaxFreq[id] ?: hwMax[id] ?: maxFreq[id] ?: 0L

    fun clusterCurrent(id: String): Long =
        clusterHwCurFreq[id]
            ?: clusterCurFreq[id]
            ?: clusters[id]?.firstOrNull()?.let { curFreq[it.toString()] }
            ?: 0L

    companion object {
        fun from(
            value: JsonElement?,
            settings: JsonObject? = null,
            defaults: JsonObject? = null,
        ): CpuSnapshot? {
            val obj = value as? JsonObject ?: return null
            if (obj.isEmpty()) return null
            val cpuSettings = settings?.get("cpu") as? JsonObject
            val cpuDefaults = defaults?.get("cpu") as? JsonObject
            return CpuSnapshot(
                online = obj.intList(ITEM_ONLINE),
                clusters = obj.entries(ITEM_CLUSTERS).mapValues { (_, v) -> (v as? JsonArray)?.intList().orEmpty() },
                maxFreq = obj.entries(ITEM_MAX).longMap(),
                minFreq = obj.entries(ITEM_MIN).longMap(),
                governor = obj.entries(ITEM_GOV).stringMap(),
                curFreq = obj.entries(ITEM_CUR).longMap(),
                clusterCurFreq = obj.entries(ITEM_CLUSTER_CUR).longMap(),
                clusterHwCurFreq = obj.entries(ITEM_CLUSTER_HW_CUR).longMap(),
                timeInState = obj.nestedLongMaps(ITEM_TIME_IN_STATE),
                frequencyTable = obj.longLists(ITEM_FREQ_TABLE),
                governorTable = obj.stringLists(ITEM_GOVERNOR_TABLE),
                usageStat = obj.usageCounters(ITEM_USAGE_STAT),
                hwMin = obj.entries(ITEM_HW_MIN).longMap(),
                hwMax = obj.entries(ITEM_HW_MAX).longMap(),
                configuredMinFreq = cpuSettings?.entries(ITEM_MIN)?.longMap().orEmpty(),
                configuredMaxFreq = cpuSettings?.entries(ITEM_MAX)?.longMap().orEmpty(),
                configuredGovernor = cpuSettings?.entries(ITEM_GOV)?.stringMap().orEmpty(),
                defaultMinFreq = cpuDefaults?.entries(ITEM_MIN)?.longMap().orEmpty(),
                defaultMaxFreq = cpuDefaults?.entries(ITEM_MAX)?.longMap().orEmpty(),
                defaultGovernor = cpuDefaults?.entries(ITEM_GOV)?.stringMap().orEmpty(),
                adaptiveEnabled = obj["cpu.adaptive"]?.jsonPrimitive?.booleanOrNull ?: false,
                screenOffActive = obj["cpu.screen_off_active"]?.jsonPrimitive?.booleanOrNull ?: false,
                tempDeciCelsius = obj[ITEM_TEMP]?.jsonPrimitive?.longOrNull,
                raw = obj,
            )
        }
    }
}

/** Cumulative /proc/stat ticks used to derive a percentage between two snapshots. */
data class CpuUsageCounter(val busy: Long, val total: Long)

/** Per-core values derived from two consecutive module snapshots. */
data class CpuLiveMetrics(
    val coreUsage: Map<Int, Float> = emptyMap(),
    val coreFreqKhz: Map<Int, Long> = emptyMap(),
    val totalUsage: Float? = null,
)

/**
 * Reproduces SoCFlow's live monitor: CPU load is calculated from /proc/stat deltas and the
 * effective cluster frequency is calculated from the time_in_state delta, falling back to
 * cpuinfo_cur_freq and then scaling_cur_freq.
 */
object CpuLiveTracker {
    fun update(previous: CpuSnapshot?, current: CpuSnapshot): CpuLiveMetrics {
        val usage = buildMap {
            if (previous != null) {
                current.usageStat.forEach { (key, now) ->
                    val before = previous.usageStat[key] ?: return@forEach
                    val deltaTotal = now.total - before.total
                    val deltaBusy = now.busy - before.busy
                    if (deltaTotal > 0L && deltaBusy >= 0L) {
                        put(key, (deltaBusy.toDouble() / deltaTotal).coerceIn(0.0, 1.0).toFloat())
                    }
                }
            }
        }

        val policyFrequency = buildMap<String, Long> {
            current.clusters.forEach { (policy, cpus) ->
                val average = averageFrequency(
                    previous?.timeInState?.get(policy),
                    current.timeInState[policy],
                )
                val fallback = current.clusterHwCurFreq[policy]
                    ?.takeIf { it > 0L }
                    ?: current.clusterCurFreq[policy]?.takeIf { it > 0L }
                    ?: cpus.firstNotNullOfOrNull { current.curFreq[it.toString()]?.takeIf { f -> f > 0L } }
                    ?: 0L
                put(policy, average.takeIf { it > 0L } ?: fallback)
            }
        }

        val coreFrequency = buildMap<Int, Long> {
            current.clusters.forEach { (policy, cpus) ->
                val frequency = policyFrequency[policy] ?: 0L
                cpus.forEach { cpu -> if (frequency > 0L) put(cpu, frequency) }
            }
            current.curFreq.forEach { (cpu, frequency) ->
                val id = cpu.toIntOrNull() ?: return@forEach
                if (frequency > 0L && id !in this) put(id, frequency)
            }
        }

        return CpuLiveMetrics(
            coreUsage = current.online.indices.mapNotNull { cpu ->
                usage["cpu$cpu"]?.let { cpu to it }
            }.toMap(),
            coreFreqKhz = coreFrequency,
            totalUsage = usage["cpu"],
        )
    }

    private fun averageFrequency(
        before: Map<Long, Long>?,
        now: Map<Long, Long>?,
    ): Long {
        if (before == null || now.isNullOrEmpty()) return 0L
        var totalTicks = 0L
        var weightedFrequency = 0.0
        for ((frequency, ticks) in now) {
            val delta = ticks - (before[frequency] ?: 0L)
            if (delta < 0L) return 0L
            if (delta == 0L) continue
            totalTicks = if (Long.MAX_VALUE - totalTicks < delta) Long.MAX_VALUE else totalTicks + delta
            weightedFrequency += delta.toDouble() * frequency.toDouble()
        }
        return if (totalTicks > 0L) (weightedFrequency / totalTicks).roundToLong() else 0L
    }
}

data class CpuDraft(
    val online: List<Int>,
    val maxFreq: Map<String, Long>,
    val minFreq: Map<String, Long>,
    val governor: Map<String, String>,
) {
    val onlineCount: Int get() = online.count { it == 1 }

    fun withCore(index: Int, enabled: Boolean): CpuDraft =
        copy(online = online.toMutableList().also { list -> list[index] = if (enabled) 1 else 0 })

    fun withMax(id: String, value: Long): CpuDraft {
        val nextMin = minOf(minFreq[id] ?: value, value)
        return copy(maxFreq = maxFreq + (id to value), minFreq = minFreq + (id to nextMin))
    }

    fun withMin(id: String, value: Long): CpuDraft {
        val nextMax = maxOf(maxFreq[id] ?: value, value)
        return copy(minFreq = minFreq + (id to value), maxFreq = maxFreq + (id to nextMax))
    }

    fun withRange(id: String, minValue: Long, maxValue: Long): CpuDraft {
        val lo = minOf(minValue, maxValue)
        val hi = maxOf(minValue, maxValue)
        return copy(
            minFreq = minFreq + (id to lo),
            maxFreq = maxFreq + (id to hi),
        )
    }

    fun withGovernor(id: String, value: String): CpuDraft = copy(governor = governor + (id to value))

    /**
     * Drop this cluster's pending edits, reverting to the live snapshot without touching the device.
     *
     * `isClusterDirty` compares against `map[id] ?: snapshot value`, so removing the keys is exactly
     * a revert: the values fall back to what the snapshot already holds.
     */
    fun withoutCluster(id: String): CpuDraft = copy(
        maxFreq = maxFreq - id,
        minFreq = minFreq - id,
        governor = governor - id,
    )

    fun isDirty(snapshot: CpuSnapshot): Boolean =
        online != snapshot.online || snapshot.clusters.keys.any { isClusterDirty(snapshot, it) }

    fun isClusterDirty(snapshot: CpuSnapshot, id: String): Boolean {
        val hasFreqTable = snapshot.frequencyTable[id].orEmpty().filter { it > 0L }.size >= 2
        if (!hasFreqTable) return false
        return snapshot.staticMax(id) != (maxFreq[id] ?: snapshot.staticMax(id)) ||
            snapshot.staticMin(id) != (minFreq[id] ?: snapshot.staticMin(id)) ||
            (snapshot.configuredGovernor[id] ?: snapshot.defaultGovernor[id] ?: snapshot.governor[id]) !=
                (governor[id] ?: snapshot.governor[id])
    }

    /** Only the changed items go to `sysflow cpu set`; a cluster id limits card-level apply. */
    fun diff(snapshot: CpuSnapshot, clusterId: String? = null): JsonObject? {
        val builder = buildJsonObject {
            if (clusterId == null && online != snapshot.online) {
                put(ITEM_ONLINE, buildJsonArray { online.forEach { add(JsonPrimitive(it)) } })
            }
            val maxChanges = maxFreq.filter { (k, v) ->
                (clusterId == null || k == clusterId) &&
                    (snapshot.frequencyTable[k].orEmpty().filter { it > 0L }.size >= 2) &&
                    snapshot.staticMax(k) != v
            }
            if (maxChanges.isNotEmpty()) put(ITEM_MAX, maxChanges.toJsonObject())
            val minChanges = minFreq.filter { (k, v) ->
                (clusterId == null || k == clusterId) &&
                    (snapshot.frequencyTable[k].orEmpty().filter { it > 0L }.size >= 2) &&
                    snapshot.staticMin(k) != v
            }
            if (minChanges.isNotEmpty()) put(ITEM_MIN, minChanges.toJsonObject())
            val governorChanges = governor.filter { (k, v) ->
                (clusterId == null || k == clusterId) &&
                    (snapshot.frequencyTable[k].orEmpty().filter { it > 0L }.size >= 2) &&
                    (snapshot.configuredGovernor[k] ?: snapshot.defaultGovernor[k] ?: snapshot.governor[k]) != v
            }
            if (governorChanges.isNotEmpty()) {
                put(ITEM_GOV, buildJsonObject { governorChanges.forEach { (k, v) -> put(k, v) } })
            }
        }
        return builder.takeIf { it.isNotEmpty() }
    }

    companion object {
        fun from(snapshot: CpuSnapshot): CpuDraft {
            val ids = snapshot.clusters.keys
            val maxFreq = ids.associateWith { id ->
                snapToTable(snapshot.frequencyTable[id].orEmpty(), snapshot.staticMax(id))
            }
            return CpuDraft(
                online = snapshot.online.toList(),
                maxFreq = maxFreq,
                minFreq = ids.associateWith { id ->
                    snapToTable(snapshot.frequencyTable[id].orEmpty(), snapshot.staticMin(id))
                        .coerceAtMost(maxFreq.getValue(id))
                },
                governor = ids.associateWith { id ->
                    snapshot.configuredGovernor[id] ?: snapshot.defaultGovernor[id] ?: snapshot.governor[id].orEmpty()
                },
            )
        }

        /** A stale config may name a frequency that another policy owns; never resend it. */
        private fun snapToTable(table: List<Long>, value: Long): Long =
            table.filter { it > 0L }.minByOrNull { abs(it - value) } ?: value
    }
}

fun Schema.item(id: String): SchemaItem? = items.firstOrNull { it.id == id }

fun Schema.lockedCores(): Set<Int> =
    item(ITEM_ONLINE)?.locked?.filterNotNull()?.toSet().orEmpty()

fun Schema.longOptions(id: String): List<Long> =
    item(id)?.options.orEmpty().mapNotNull { it.jsonPrimitive.longOrNull }.sorted()

fun Schema.stringOptions(id: String): List<String> =
    item(id)?.options.orEmpty().mapNotNull { (it as? JsonPrimitive)?.content }.filter { it.isNotBlank() }

fun formatKhz(khz: Long?): String {
    if (khz == null || khz <= 0L) return "—"
    return if (khz >= 1_000_000L) {
        String.format(Locale.US, "%.2f GHz", khz / 1_000_000.0)
    } else {
        String.format(Locale.US, "%d MHz", Math.round(khz / 1000.0))
    }
}

fun formatTemp(deciCelsius: Long?): String =
    if (deciCelsius == null) "—" else String.format(Locale.US, "%.1f °C", deciCelsius / 10.0)

fun ApplyResult.describeMismatch(): List<String> = mismatch.map { key ->
    val requestedVal = requested[key]
    val appliedVal = applied[key]
    val want = requestedVal as? JsonObject
    val have = appliedVal as? JsonObject ?: JsonObject(emptyMap())
    if (want != null && want.isNotEmpty()) {
        val lines = want.entries.mapNotNull { (sub, value) ->
            val actual = have[sub]
            if (actual == value) {
                null
            } else {
                val actualText = if (actual == null) "缺失" else formatMismatchVal(key, actual)
                "$key[$sub] 请求 ${formatMismatchVal(key, value)}，读回 $actualText"
            }
        }
        if (lines.isNotEmpty()) return@map lines.joinToString("；")
        return@map want.entries.joinToString("；") { (sub, value) ->
            val actual = have[sub]
            val actualText = if (actual == null) "缺失" else formatMismatchVal(key, actual)
            "$key[$sub] 请求 ${formatMismatchVal(key, value)}，读回 $actualText"
        }
    }
    if (requestedVal != null) {
        val actualText = if (appliedVal == null) "缺失" else formatMismatchVal(key, appliedVal)
        return@map "$key 请求 ${formatMismatchVal(key, requestedVal)}，读回 $actualText"
    }
    key
}

private fun formatMismatchVal(key: String, value: JsonElement?): String {
    if (value == null) return "缺失"
    val primitive = value as? JsonPrimitive ?: return value.toString()
    if (primitive.isString) return primitive.content
    primitive.longOrNull?.let { number ->
        if (key.contains("freq")) return formatKhz(number)
    }
    return primitive.content
}

fun clusterLabel(id: String, cpus: List<Int>): String =
    if (cpus.isEmpty()) "Cluster $id" else "Cluster $id · CPU ${cpus.first()}–${cpus.last()}"

private fun JsonObject.intList(key: String): List<Int> {
    val arr = this[key] as? JsonArray ?: return emptyList()
    return arr.mapNotNull { it.jsonPrimitive.intOrNull }
}

private fun JsonObject.entries(key: String): Map<String, JsonElement> =
    (this[key] as? JsonObject)?.toMap().orEmpty()

private fun JsonObject.longLists(key: String): Map<String, List<Long>> =
    entries(key).mapNotNull { (outerKey, value) ->
        val values = (value as? JsonArray)?.mapNotNull { it.jsonPrimitive.longOrNull }.orEmpty()
        if (values.isEmpty()) null else outerKey to values
    }.toMap()

private fun JsonObject.stringLists(key: String): Map<String, List<String>> =
    entries(key).mapNotNull { (outerKey, value) ->
        val values = (value as? JsonArray)?.mapNotNull { element ->
            element.jsonPrimitive.content.takeIf { it.isNotBlank() }
        }.orEmpty()
        if (values.isEmpty()) null else outerKey to values
    }.toMap()

private fun JsonObject.nestedLongMaps(key: String): Map<String, Map<Long, Long>> =
    entries(key).mapNotNull { (outerKey, value) ->
        val inner = (value as? JsonObject)?.mapNotNull { (frequency, ticks) ->
            val f = frequency.toLongOrNull()
            val t = (ticks as? JsonPrimitive)?.longOrNull
            if (f != null && t != null && f > 0L) f to t else null
        }?.toMap().orEmpty()
        if (inner.isEmpty()) null else outerKey to inner
    }.toMap()

private fun JsonObject.usageCounters(key: String): Map<String, CpuUsageCounter> =
    entries(key).mapNotNull { (cpu, value) ->
        val counter = value as? JsonObject ?: return@mapNotNull null
        val busy = counter["busy"]?.jsonPrimitive?.longOrNull ?: return@mapNotNull null
        val total = counter["total"]?.jsonPrimitive?.longOrNull ?: return@mapNotNull null
        cpu to CpuUsageCounter(busy, total)
    }.toMap()

private fun Map<String, JsonElement>.longMap(): Map<String, Long> = mapNotNull { (k, v) ->
    (v as? JsonPrimitive)?.longOrNull?.let { k to it }
}.toMap()

private fun Map<String, JsonElement>.stringMap(): Map<String, String> = mapNotNull { (k, v) ->
    (v as? JsonPrimitive)?.content?.let { k to it }
}.toMap()

private fun Map<String, Long>.toJsonObject(): JsonObject =
    buildJsonObject { forEach { (k, v) -> put(k, JsonPrimitive(v)) } }

private fun JsonArray.intList(): List<Int> = mapNotNull { (it as? JsonPrimitive)?.intOrNull }
