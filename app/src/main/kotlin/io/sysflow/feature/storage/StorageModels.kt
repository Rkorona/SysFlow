package io.sysflow.feature.storage

import io.sysflow.core.model.Schema
import io.sysflow.core.ui.components.ChartPoint
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.booleanOrNull
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.intOrNull
import kotlinx.serialization.json.longOrNull
import java.util.Locale

/**
 * Model layer for the storage page.
 *
 * Pure functions over the protocol JSON (`docs/protocol.md` §3.26), so the screen never sees key
 * names and parsing can be unit-tested without Android. Missing values stay nullable: a device
 * without a JEDEC health counter, a second filesystem, or I/O stats should just hide that block.
 */

data class StorageCapacity(
    val totalKb: Long,
    val usedKb: Long,
    val freeKb: Long,
    val availKb: Long,
    val usedPct: Int,
) {
    val usedFraction: Float
        get() = if (totalKb <= 0) 0f else (usedKb.toFloat() / totalKb).coerceIn(0f, 1f)
}

data class StorageDevice(
    val name: String?,
    val sizeKb: Long?,
    val model: String?,
    val rotational: Boolean,
    val healthPct: Int?,
)

data class StorageSnapshot(
    val capacity: StorageCapacity,
    val sdcard: StorageCapacity?,
    val device: StorageDevice,
    val scheduler: String?,
    val readAheadKb: Int?,
    val ioReadKb: Long?,
    val ioWriteKb: Long?,
    val ioReadIos: Long?,
    val ioWriteIos: Long?,
)

data class StorageIo(
    val device: String?,
    val intervalMs: Long,
    val readKb: Long,
    val writeKb: Long,
    val readIops: Double,
    val writeIops: Double,
    val utilPct: Double,
    val totalReadKb: Long?,
    val totalWriteKb: Long?,
)

data class StorageUsageEntry(
    val name: String,
    val path: String,
    val kb: Long,
    val files: Long,
    val dirs: Long,
)

data class StorageUsage(
    val path: String,
    val partial: Boolean,
    val elapsedMs: Long,
    val denied: Long,
    val visited: Long,
    val totalKb: Long,
    val entries: List<StorageUsageEntry>,
)

data class StorageHistory(
    val read: List<ChartPoint>,
    val write: List<ChartPoint>,
) {
    val isEmpty: Boolean get() = read.isEmpty() && write.isEmpty()
}

/** Option lists come from the module schema; the current value comes from the snapshot. */
data class StorageOptions(
    val scheduler: List<String>,
    val readAhead: List<Int>,
)

private const val DATA = "storage.data"
private const val SDCARD = "storage.sdcard"

fun parseStorageSnapshot(module: JsonObject?): StorageSnapshot? {
    if (module == null) return null
    val capacity = parseCapacity(module, DATA) ?: return null
    val sdcard = parseCapacity(module, SDCARD)
    val device = StorageDevice(
        name = module.string("storage.device.name"),
        sizeKb = module.long("storage.device.size_kb"),
        model = module.string("storage.device.model"),
        rotational = module.bool("storage.device.rotational") ?: false,
        healthPct = module.int("storage.device.health_pct"),
    )
    return StorageSnapshot(
        capacity = capacity,
        sdcard = sdcard,
        device = device,
        scheduler = module.string("storage.scheduler"),
        readAheadKb = module.int("storage.read_ahead_kb"),
        ioReadKb = module.long("storage.io.read_kb"),
        ioWriteKb = module.long("storage.io.write_kb"),
        ioReadIos = module.long("storage.io.read_ios"),
        ioWriteIos = module.long("storage.io.write_ios"),
    )
}

private fun parseCapacity(module: JsonObject, prefix: String): StorageCapacity? {
    val total = module.long("$prefix.total_kb") ?: return null
    return StorageCapacity(
        totalKb = total,
        usedKb = module.long("$prefix.used_kb") ?: 0,
        freeKb = module.long("$prefix.free_kb") ?: 0,
        availKb = module.long("$prefix.avail_kb") ?: 0,
        usedPct = module.int("$prefix.used_pct") ?: 0,
    )
}

fun parseStorageIo(json: JsonObject): StorageIo = StorageIo(
    device = json.string("device"),
    intervalMs = json.long("interval_ms") ?: 0,
    readKb = json.long("read_kb") ?: 0,
    writeKb = json.long("write_kb") ?: 0,
    readIops = json.double("read_iops") ?: 0.0,
    writeIops = json.double("write_iops") ?: 0.0,
    utilPct = json.double("util_pct") ?: 0.0,
    totalReadKb = json.long("total_read_kb"),
    totalWriteKb = json.long("total_write_kb"),
)

fun parseStorageUsage(json: JsonObject): StorageUsage = StorageUsage(
    path = json.string("path") ?: "/data",
    partial = json.bool("partial") ?: false,
    elapsedMs = json.long("elapsed_ms") ?: 0,
    denied = json.long("denied") ?: 0,
    visited = json.long("visited") ?: 0,
    totalKb = json.long("total_kb") ?: 0,
    entries = json.array("entries").mapNotNull { element ->
        (element as? JsonObject)?.let { entry ->
            StorageUsageEntry(
                name = entry.string("name") ?: return@let null,
                path = entry.string("path") ?: "",
                kb = entry.long("kb") ?: 0,
                files = entry.long("files") ?: 0,
                dirs = entry.long("dirs") ?: 0,
            )
        }
    },
)

/** `history storage` returns `series.read_kb` / `series.write_kb` as `[ts, value]` pairs. */
fun parseStorageHistory(value: JsonObject?): StorageHistory {
    val series = value?.get("series") as? JsonObject ?: return StorageHistory(emptyList(), emptyList())
    fun points(key: String): List<ChartPoint> =
        ((series[key] as? JsonArray) ?: JsonArray(emptyList())).mapNotNull { element ->
            val pair = element as? JsonArray ?: return@mapNotNull null
            if (pair.size < 2) return@mapNotNull null
            val x = (pair[0] as? JsonPrimitive)?.longOrNull ?: return@mapNotNull null
            val y = (pair[1] as? JsonPrimitive)?.longOrNull ?: return@mapNotNull null
            ChartPoint(x = x, y = y.toFloat())
        }
    return StorageHistory(read = points("read_kb"), write = points("write_kb"))
}

fun parseStorageOptions(schema: Schema): StorageOptions {
    fun item(id: String) = schema.items.firstOrNull { it.id == id }
    val scheduler = item("storage.scheduler")?.options.orEmpty().mapNotNull {
        (it as? JsonPrimitive)?.contentOrNull
    }
    val readAhead = item("storage.read_ahead_kb")?.options.orEmpty().mapNotNull {
        (it as? JsonPrimitive)?.longOrNull?.toInt()
    }
    return StorageOptions(scheduler = scheduler, readAhead = readAhead)
}

// ---------------------------------------------------------------------------------------------
// Formatting
// ---------------------------------------------------------------------------------------------

/** kB → "128.0 GB" / "512.0 MB" / "640 KB", the unit sizes on this page are read in. */
fun formatStorageKb(kb: Long?): String {
    if (kb == null) return "—"
    return when {
        kb >= 1024L * 1024L -> String.format(Locale.ROOT, "%.1f GB", kb / (1024.0 * 1024.0))
        kb >= 1024L -> String.format(Locale.ROOT, "%.1f MB", kb / 1024.0)
        else -> "$kb KB"
    }
}

/** Throughput label: "1.2 MB/s", "0 KB/s". */
fun formatStorageRate(kb: Long?): String = when {
    kb == null -> "—"
    kb >= 1024 -> String.format(Locale.ROOT, "%.1f MB/s", kb / 1024.0)
    else -> "$kb KB/s"
}

fun formatStoragePct(value: Int?): String = value?.let { "$it%" } ?: "—"

/** IOPS with one decimal: "12.0", "0.0". */
fun formatIops(value: Double): String = String.format(Locale.ROOT, "%.1f", value)

private fun JsonObject.array(key: String): JsonArray = this[key] as? JsonArray ?: JsonArray(emptyList())
private fun JsonObject.string(key: String): String? = (this[key] as? JsonPrimitive)?.contentOrNull
private fun JsonObject.long(key: String): Long? = (this[key] as? JsonPrimitive)?.longOrNull
private fun JsonObject.int(key: String): Int? = (this[key] as? JsonPrimitive)?.intOrNull
private fun JsonObject.double(key: String): Double? = (this[key] as? JsonPrimitive)?.contentOrNull?.toDoubleOrNull()
private fun JsonObject.bool(key: String): Boolean? = (this[key] as? JsonPrimitive)?.booleanOrNull

