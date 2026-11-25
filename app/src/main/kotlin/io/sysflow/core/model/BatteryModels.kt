package io.sysflow.core.model

import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.booleanOrNull
import kotlinx.serialization.json.doubleOrNull
import kotlinx.serialization.json.intOrNull
import kotlinx.serialization.json.longOrNull
import kotlin.math.abs

/**
 * Battery telemetry from `snapshot.modules.battery` (protocol §3.15).
 *
 * The module is read-only and omits fields the device does not expose, so every value is nullable
 * and "missing" stays distinct from zero. Units arrive already normalised (mV / mA / mAh / mW /
 * 0.1 °C); never rescale them here.
 */
data class BatterySnapshot(
    val capacity: Int?,
    val status: String?,
    val charging: Boolean,
    val tempDeciC: Int?,
    val voltageMv: Int?,
    val currentMa: Int?,
    val powerMw: Int?,
    val health: String?,
    val healthPercent: Int?,
    val cycleCount: Int?,
    val chargeFullMah: Int?,
    val chargeFullDesignMah: Int?,
    val acOnline: Boolean?,
    val usbOnline: Boolean?,
    val wirelessOnline: Boolean? = null,
    /** Coulomb-counter charge remaining, when the driver exposes `charge_counter`. */
    val chargeMah: Int? = null,
    val usbType: String? = null,
    val inputCurrentLimit: Int? = null,
    val fastCharging: Boolean = false,
) {
    companion object {
        fun from(value: JsonElement?): BatterySnapshot? {
            val obj = value as? JsonObject ?: return null
            if (obj.isEmpty()) return null
            return BatterySnapshot(
                capacity = obj.int("battery.capacity"),
                status = obj.text("battery.status"),
                charging = (obj["battery.charging"] as? JsonPrimitive)?.booleanOrNull == true,
                tempDeciC = obj.int("battery.temp"),
                voltageMv = obj.int("battery.voltage"),
                currentMa = obj.int("battery.current"),
                powerMw = obj.int("battery.power"),
                health = obj.text("battery.health"),
                healthPercent = obj.int("battery.health_percent"),
                cycleCount = obj.int("battery.cycle_count"),
                chargeFullMah = obj.int("battery.charge_full"),
                chargeFullDesignMah = obj.int("battery.charge_full_design"),
                acOnline = obj.boolean("battery.ac_online"),
                usbOnline = obj.boolean("battery.usb_online"),
                wirelessOnline = obj.boolean("battery.wireless_online"),
                chargeMah = obj.int("battery.charge_mah"),
                usbType = obj.text("battery.usb_type"),
                inputCurrentLimit = obj.int("battery.input_current_limit"),
                fastCharging = obj.boolean("battery.fast_charging") == true,
            )
        }
    }
}

private fun JsonObject.int(key: String): Int? =
    (this[key] as? JsonPrimitive)?.intOrNull

private fun JsonObject.boolean(key: String): Boolean? =
    (this[key] as? JsonPrimitive)?.booleanOrNull

private fun JsonObject.text(key: String): String? =
    (this[key] as? JsonPrimitive)?.content?.trim()?.takeIf(String::isNotEmpty)

/** Kernel status strings arrive verbatim and vary in capitalisation. */
fun batteryStatusLabel(status: String?): String {
    if (status == null) return "—"
    return when (status.lowercase()) {
        "charging" -> "充电中"
        "discharging" -> "放电中"
        "full" -> "已充满"
        "not charging" -> "未充电"
        "unknown" -> "状态未知"
        else -> status
    }
}

fun formatPowerMw(mw: Int?): String {
    val value = mw ?: return "—"
    return if (abs(value) >= 1000) "${String.format(java.util.Locale.ROOT, "%.1f", value / 1000.0)} W"
    else "$value mW"
}

fun formatMilliAmp(ma: Int?): String = if (ma == null) "—" else "$ma mA"

fun formatMilliVolt(mv: Int?): String =
    if (mv == null) "—" else "${String.format(java.util.Locale.ROOT, "%.3f", mv / 1000.0)} V"

fun formatMah(mah: Int?): String = if (mah == null) "—" else "$mah mAh"

fun batteryHealthLabel(health: String?, percent: Int?): String {
    if (health == null) return percent?.let { "$it%" } ?: "—"
    return if (percent == null) health else "$health · $percent%"
}

/** Charger line for the overview card, e.g. `USB 充电中`. */
fun chargerLabel(battery: BatterySnapshot): String = when {
    battery.wirelessOnline == true -> if (battery.charging) "无线充电中" else "无线充电板已连接"
    battery.acOnline == true -> "AC 供电"
    battery.usbOnline == true -> if (battery.charging) "USB 充电中" else "USB 已连接"
    battery.charging -> "充电中"
    else -> batteryStatusLabel(battery.status)
}

/**
 * Remaining charge in mAh.
 *
 * The coulomb counter is measured charge; `capacity% × full` is inferred and quantised to whole
 * percent steps, so it is only the fallback.
 */
fun remainingMah(battery: BatterySnapshot): Int? {
    battery.chargeMah?.let { return it }
    val capacity = battery.capacity ?: return null
    val full = battery.chargeFullMah ?: return null
    return Math.round(capacity / 100.0 * full).toInt()
}

/** Capacity lost against the design spec, in mAh. */
fun capacityLossMah(battery: BatterySnapshot): Int? {
    val full = battery.chargeFullMah ?: return null
    val design = battery.chargeFullDesignMah ?: return null
    return (design - full).coerceAtLeast(0)
}

data class BatteryRuntime(val minutes: Int, val charging: Boolean)

/**
 * Rough time-to-full or time-to-empty.
 *
 * Direction comes from [BatterySnapshot.charging], never from the sign of `current`: kernels
 * disagree about which sign means discharge, so `|current|` is used as well. The value is an
 * estimate because `current` is an instantaneous reading, so the UI must label it as such.
 */
fun estimateRuntime(battery: BatterySnapshot): BatteryRuntime? {
    val current = battery.currentMa ?: return null
    val full = battery.chargeFullMah ?: return null
    val capacity = battery.capacity ?: return null
    val flow = kotlin.math.abs(current)
    if (flow <= 0) return null
    val remaining = capacity / 100.0 * full
    val minutes = if (battery.charging) (full - remaining) / flow * 60 else remaining / flow * 60
    if (!minutes.isFinite() || minutes <= 0 || minutes > 60 * 24) return null
    return BatteryRuntime(minutes = Math.round(minutes).toInt(), charging = battery.charging)
}

fun formatDuration(minutes: Int?): String {
    if (minutes == null || minutes <= 0) return "—"
    if (minutes < 60) return "约 $minutes 分钟"
    val hours = minutes / 60
    val rest = minutes % 60
    return if (rest == 0) "约 $hours 小时" else "约 $hours 小时 $rest 分"
}

enum class BatteryTempTone { Normal, Warn, Danger }

/** Charging above ~45 °C is worth flagging; above 50 °C the cell is being stressed. */
fun batteryTempTone(deciC: Int?): BatteryTempTone = when {
    deciC == null -> BatteryTempTone.Normal
    deciC >= 500 -> BatteryTempTone.Danger
    deciC >= 450 -> BatteryTempTone.Warn
    else -> BatteryTempTone.Normal
}

/** One row of the BatteryStats per-UID power ranking (`sysflow battery apps`). */
data class BatteryAppEntry(
    val uid: String,
    val numericUid: Int?,
    val name: String?,
    val packages: List<String>,
    val powerMah: Double,
    /** 0..1 share of the reported total. */
    val share: Double,
) {
    val isSystemUid: Boolean get() = numericUid != null && numericUid < 10_000
}

data class BatteryApps(
    val available: Boolean,
    val reason: String?,
    val totalMah: Double?,
    val capacityMah: Double?,
    val count: Int,
    val entries: List<BatteryAppEntry>,
) {
    companion object {
        fun from(value: JsonObject?): BatteryApps? {
            val obj = value ?: return null
            val entryArray = obj["entries"] as? kotlinx.serialization.json.JsonArray
            val entries = entryArray.orEmpty().mapNotNull { element ->
                val row = element as? JsonObject ?: return@mapNotNull null
                val uid = (row["uid"] as? JsonPrimitive)?.content ?: return@mapNotNull null
                BatteryAppEntry(
                    uid = uid,
                    numericUid = (row["numeric_uid"] as? JsonPrimitive)?.intOrNull,
                    name = (row["name"] as? JsonPrimitive)?.content?.takeIf(String::isNotBlank),
                    packages = (row["packages"] as? kotlinx.serialization.json.JsonArray)
                        .orEmpty()
                        .mapNotNull { (it as? JsonPrimitive)?.content },
                    powerMah = (row["power_mah"] as? JsonPrimitive)?.doubleOrNull ?: 0.0,
                    share = (row["share"] as? JsonPrimitive)?.doubleOrNull ?: 0.0,
                )
            }
            return BatteryApps(
                available = (obj["available"] as? JsonPrimitive)?.booleanOrNull == true,
                reason = (obj["reason"] as? JsonPrimitive)?.content?.takeIf(String::isNotBlank),
                totalMah = (obj["total_mah"] as? JsonPrimitive)?.doubleOrNull,
                capacityMah = (obj["capacity_mah"] as? JsonPrimitive)?.doubleOrNull,
                count = (obj["count"] as? JsonPrimitive)?.intOrNull ?: entries.size,
                entries = entries,
            )
        }
    }
}

/** Cycle count and health, including vendor fallbacks the sysfs-only snapshot cannot see. */
data class BatteryHealth(
    val cycleCount: Int?,
    val cycleSource: String?,
    val healthPercent: Int?,
    val healthSource: String?,
) {
    companion object {
        fun from(value: JsonObject?): BatteryHealth? {
            val obj = value ?: return null
            return BatteryHealth(
                cycleCount = (obj["cycle_count"] as? JsonPrimitive)?.intOrNull,
                cycleSource = (obj["cycle_source"] as? JsonPrimitive)?.content?.takeIf(String::isNotBlank),
                healthPercent = (obj["health_percent"] as? JsonPrimitive)?.intOrNull,
                healthSource = (obj["health_source"] as? JsonPrimitive)?.content?.takeIf(String::isNotBlank),
            )
        }
    }
}

/**
 * One recorded history point: unix seconds plus the metric values.
 *
 * [screen] is the daemon's sampling context (`1` interactive / `0` not), not a battery measurement.
 * It is null for samples recorded before the daemon tracked display state, which is why the drain
 * split treats missing values as "unknown" instead of guessing.
 */
data class BatteryHistoryPoint(
    val ts: Long,
    val level: Int?,
    val temp: Int?,
    val charging: Boolean,
    val screen: Boolean? = null,
)

/** `sysflow history battery` decoded into what a chart needs. */
data class BatteryHistory(
    val stepSeconds: Long,
    val points: List<BatteryHistoryPoint>,
    val daemonRunning: Boolean,
) {
    /** Level curve in chronological order, for the sparkline. */
    val levels: List<Int> get() = points.mapNotNull { it.level }

    /**
     * Charge lost since the battery last left a charging state.
     *
     * Returns null when the window does not contain a charge (the curve would otherwise attribute
     * an unrelated drop to "this charge") or when there is not enough data.
     */
    fun drainSinceLastCharge(): BatteryDrain? {
        val lastCharging = points.indexOfLast { it.charging }
        if (lastCharging < 0) return null
        val start = points.getOrNull(lastCharging) ?: return null
        val tail = points.drop(lastCharging + 1)
        if (tail.isEmpty()) return null
        val startLevel = start.level ?: tail.firstNotNullOfOrNull { it.level } ?: return null
        val endPoint = tail.lastOrNull { it.level != null } ?: return null
        val endLevel = endPoint.level ?: return null
        val hours = (endPoint.ts - start.ts) / 3600.0
        if (hours <= 0.0) return null
        return BatteryDrain(
            droppedPercent = (startLevel - endLevel).coerceAtLeast(0),
            hours = hours,
        )
    }

    companion object {
        fun from(value: JsonObject?): BatteryHistory? {
            val obj = value ?: return null
            val series = obj["series"] as? JsonObject
            val level = series?.get("level") as? kotlinx.serialization.json.JsonArray
            if (level == null || level.isEmpty()) return null

            // The series are column-oriented; walk the level column and look each ts up elsewhere.
            val temp = columnByTs(series["temp"] as? kotlinx.serialization.json.JsonArray)
            val charging = columnByTs(series["charging"] as? kotlinx.serialization.json.JsonArray)
            val screen = columnByTs(series["screen"] as? kotlinx.serialization.json.JsonArray)
            val points = level.mapNotNull { element ->
                val pair = element as? kotlinx.serialization.json.JsonArray ?: return@mapNotNull null
                val ts = (pair.getOrNull(0) as? JsonPrimitive)?.longOrNull ?: return@mapNotNull null
                val value = (pair.getOrNull(1) as? JsonPrimitive)?.intOrNull
                BatteryHistoryPoint(
                    ts = ts,
                    level = value,
                    temp = temp[ts],
                    charging = (charging[ts] ?: 0) != 0,
                    screen = screen[ts]?.let { it != 0 },
                )
            }
            if (points.isEmpty()) return null
            return BatteryHistory(
                stepSeconds = (obj["step_s"] as? JsonPrimitive)?.longOrNull ?: 3600,
                points = points,
                daemonRunning = (obj["daemon_running"] as? JsonPrimitive)?.booleanOrNull == true,
            )
        }

        private fun columnByTs(column: kotlinx.serialization.json.JsonArray?): Map<Long, Int> {
            if (column == null) return emptyMap()
            return column.mapNotNull { element ->
                val pair = element as? kotlinx.serialization.json.JsonArray ?: return@mapNotNull null
                val ts = (pair.getOrNull(0) as? JsonPrimitive)?.longOrNull ?: return@mapNotNull null
                val value = (pair.getOrNull(1) as? JsonPrimitive)?.intOrNull ?: return@mapNotNull null
                ts to value
            }.toMap()
        }
    }
}

data class BatteryDrain(val droppedPercent: Int, val hours: Double) {
    /** Average loss per hour while off the charger. */
    val percentPerHour: Double get() = if (hours > 0) droppedPercent / hours else 0.0
}
