package io.sysflow.core.model

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/** Drain split from the daemon's `screen` sampling context (protocol §3.9). */
class BatteryDrainTest {
    private fun history(vararg points: BatteryHistoryPoint) = BatteryHistory(
        stepSeconds = 3600,
        points = points.toList(),
        daemonRunning = true,
    )

    private fun point(ts: Long, level: Int?, screen: Boolean?, charging: Boolean = false) =
        BatteryHistoryPoint(ts = ts, level = level, temp = null, charging = charging, screen = screen)

    @Test
    fun splitsDrainByDisplayState() {
        // 2 h screen on: 100 -> 96 (2 %/h); 2 h screen off: 96 -> 92 (also 2 %/h).
        val split = history(
            point(0, 100, screen = true),
            point(3600, 98, screen = true),
            point(7200, 96, screen = false),
            point(7200 + 3600, 94, screen = false),
            point(7200 + 7200, 92, screen = false),
        ).drainSplit()

        assertTrue(split.hasData)
        assertNotNull(split.screenOn)
        assertEquals(2.0, split.screenOn!!.percentPerHour, 1e-9)
        assertNotNull(split.screenOff)
        // Off window is 2 h with a 4 % drop.
        assertEquals(2.0, split.screenOff!!.percentPerHour, 1e-9)
    }

    @Test
    fun excludesChargingIntervals() {
        val split = history(
            point(0, 60, screen = false, charging = true),
            point(3600, 80, screen = false, charging = true),
            point(7200, 80, screen = false),
            point(7200 + 3600, 79, screen = false),
        ).drainSplit()

        assertNull("the charging window must not count", split.screenOn)
        assertEquals(1.0, split.screenOff!!.percentPerHour, 1e-9)
    }

    @Test
    fun skipsIntervalsWithUnknownOrChangingState() {
        val split = history(
            point(0, 90, screen = null),
            point(3600, 88, screen = null),
            point(7200, 86, screen = true),
            // State changes mid-interval: the previous sample is screen-on, this one off.
            point(7200 + 3600, 84, screen = false),
            point(7200 + 7200, 82, screen = false),
        ).drainSplit()

        // Only the final pair is attributable: 2 % over 1 h, screen off.
        assertNull(split.screenOn)
        assertEquals(2.0, split.screenOff!!.percentPerHour, 1e-9)
    }

    @Test
    fun hasNoSplitWithoutScreenContext() {
        val split = history(
            point(0, 90, screen = null),
            point(3600, 88, screen = null),
        ).drainSplit()

        assertFalse(split.hasData)
        assertNull(split.screenOn)
        assertNull(split.screenOff)
    }

    @Test
    fun ignoresLevelRisesOffTheCharger() {
        // Temperature/rounding can bump the reported level by a point while discharging.
        val split = history(
            point(0, 80, screen = true),
            point(3600, 81, screen = true),
            point(7200, 79, screen = true),
        ).drainSplit()

        // The +1 point rise contributes nothing; only the real 2 % drop counts, over 2 h total.
        assertEquals(2, split.screenOn!!.droppedPercent)
        assertEquals(1.0, split.screenOn!!.percentPerHour, 1e-9)
    }
}
