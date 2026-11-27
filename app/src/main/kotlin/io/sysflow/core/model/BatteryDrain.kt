package io.sysflow.core.model

/**
 * Battery drain split by display state (protocol §3.9 `screen` sampling context).
 *
 * The daemon records the display state next to every battery sample, which is the only way to tell
 * "screen-off drain" from "screen-on drain": the APK is not running in the background and must not
 * infer the state from its own visibility.
 *
 * Kept as pure functions over [BatteryHistory] so it can be unit-tested without Compose or a device.
 */

/** Charge lost over a window, and how long the window was. */
data class BatteryDrainRate(val droppedPercent: Int, val hours: Double) {
    val percentPerHour: Double get() = if (hours > 0.0) droppedPercent / hours else 0.0
}

/** Drain while the display was on and while it was off. Either side may be absent. */
data class BatteryDrainSplit(val screenOn: BatteryDrainRate?, val screenOff: BatteryDrainRate?) {
    val hasData: Boolean get() = screenOn != null || screenOff != null

    companion object {
        val EMPTY = BatteryDrainSplit(screenOn = null, screenOff = null)
    }
}

/**
 * Split drain by display state.
 *
 * Only adjacent samples that agree on the state and are both off the charger count. Charging
 * intervals are skipped because a rising level would otherwise drag the rate down; unknown states
 * are skipped because attributing an interval to the wrong side is worse than omitting it.
 */
fun BatteryHistory.drainSplit(): BatteryDrainSplit {
    val ordered = points.sortedBy { it.ts }
    var onDropped = 0
    var onSeconds = 0L
    var offDropped = 0
    var offSeconds = 0L

    for (index in 1 until ordered.size) {
        val previous = ordered[index - 1]
        val current = ordered[index]
        val state = current.screen ?: continue
        if (previous.screen != state || previous.charging || current.charging) continue
        val before = previous.level ?: continue
        val after = current.level ?: continue
        val seconds = current.ts - previous.ts
        if (seconds <= 0L) continue
        // Off the charger, so a rise is measurement noise rather than stored energy.
        val dropped = (before - after).coerceAtLeast(0)
        if (state) {
            onDropped += dropped
            onSeconds += seconds
        } else {
            offDropped += dropped
            offSeconds += seconds
        }
    }

    return BatteryDrainSplit(
        screenOn = onSeconds.takeIf { it > 0 }?.let {
            BatteryDrainRate(droppedPercent = onDropped, hours = it / 3600.0)
        },
        screenOff = offSeconds.takeIf { it > 0 }?.let {
            BatteryDrainRate(droppedPercent = offDropped, hours = it / 3600.0)
        },
    )
}
