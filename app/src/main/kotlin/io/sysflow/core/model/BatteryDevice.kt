package io.sysflow.core.model

import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.os.BatteryManager
import android.os.Build
import kotlin.math.abs

/**
 * Battery readings that only Android exposes through `BatteryManager`, not sysfs.
 *
 * These are the framework's own values, so they outrank anything we shell out for: the module's
 * `battery health` verb has to guess between vendor nodes, `dumpsys` output and a capacity ratio,
 * while the platform reports the same numbers through a stable API. Nothing here needs root.
 */
object BatteryDevice {

    /** Value returned by `getIntProperty` when the platform cannot answer. */
    private const val UNSUPPORTED = Int.MIN_VALUE
    private const val LONG_UNSUPPORTED = Long.MIN_VALUE

    /**
     * Real charge remaining, from the coulomb counter. More accurate than `capacity% × design`,
     * because it measures charge rather than inferring it from a percentage.
     */
    fun chargeCounterMah(context: Context): Int? {
        val manager = context.getSystemService(BatteryManager::class.java) ?: return null
        val raw = runCatching {
            manager.getIntProperty(BatteryManager.BATTERY_PROPERTY_CHARGE_COUNTER)
        }.getOrNull() ?: return null
        if (raw == UNSUPPORTED) return null
        return normalizeMilliAmpHours(abs(raw.toLong()))
    }

    /**
     * Cycle count published by Android 14's battery-health extras. Available since API 34 and only
     * on devices whose HAL implements it.
     */
    fun cycleCount(context: Context): Int? {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.UPSIDE_DOWN_CAKE) return null
        val intent = runCatching {
            context.registerReceiver(null, IntentFilter(Intent.ACTION_BATTERY_CHANGED))
        }.getOrNull() ?: return null
        val value = intent.getIntExtra(EXTRA_CYCLE_COUNT, -1)
        return value.takeIf { it in 1..9999 }
    }

    /** Battery capacity percentage straight from the framework (0..100). */
    fun levelPercent(context: Context): Int? {
        val manager = context.getSystemService(BatteryManager::class.java) ?: return null
        val raw = runCatching {
            manager.getIntProperty(BatteryManager.BATTERY_PROPERTY_CAPACITY)
        }.getOrNull() ?: return null
        if (raw == UNSUPPORTED) return null
        return raw.takeIf { it in 0..100 }
    }

    /** `android.os.extra.CYCLE_COUNT`, guarded because the constant only exists on API 34+. */
    private val EXTRA_CYCLE_COUNT: String = "android.os.extra.CYCLE_COUNT"

    /** µAh and mAh both occur; decide by magnitude, exactly like the Rust side does. */
    private fun normalizeMilliAmpHours(raw: Long): Int? {
        if (raw <= 0 || raw == LONG_UNSUPPORTED) return null
        val value = if (raw > 100_000) raw / 1000 else raw
        return value.takeIf { it in 1..100_000 }?.toInt()
    }
}
