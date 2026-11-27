package io.sysflow.core.model

import kotlinx.serialization.json.Json
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.longOrNull
import kotlinx.serialization.json.buildJsonObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Test

class CpuLiveTrackerTest {
    @Test
    fun calculatesPerCoreLoadAndTimeInStateAverage() {
        val before = snapshot(
            stats = mapOf("cpu" to CpuUsageCounter(800, 1600), "cpu0" to CpuUsageCounter(100, 200)),
            tis = mapOf("0" to mapOf(300_000L to 10L, 1_800_000L to 20L)),
        )
        val current = snapshot(
            stats = mapOf("cpu" to CpuUsageCounter(900, 1800), "cpu0" to CpuUsageCounter(130, 250)),
            tis = mapOf("0" to mapOf(300_000L to 20L, 1_800_000L to 40L)),
        )

        val metrics = CpuLiveTracker.update(before, current)

        assertEquals(0.6f, metrics.coreUsage.getValue(0), 0.0001f)
        assertEquals(0.5f, metrics.totalUsage!!, 0.0001f)
        assertEquals(1_300_000L, metrics.coreFreqKhz.getValue(0))
    }

    @Test
    fun usesHardwareThenScalingFrequencyWhenTimeInStateCannotBeDifferenced() {
        val before = snapshot(
            tis = mapOf("0" to mapOf(300_000L to 10L)),
        )
        val current = snapshot(
            clusterHw = mapOf("0" to 1_100_000L),
            clusterCur = mapOf("0" to 1_500_000L),
            tis = mapOf("0" to mapOf(300_000L to 10L)),
        )

        val metrics = CpuLiveTracker.update(before, current)

        assertEquals(1_100_000L, metrics.coreFreqKhz.getValue(0))
    }

    @Test
    fun distinguishesStaticLimitsFromAdaptiveEffectiveLimits() {
        val live = Json.parseToJsonElement(
            """{
                "cpu.online": [1],
                "cpu.clusters": {"0": [0]},
                "cpu.max_freq": {"0": 1200000},
                "cpu.min_freq": {"0": 500000},
                "cpu.governor": {"0": "walt"},
                "cpu.hw_min_freq": {"0": 384000},
                "cpu.hw_max_freq": {"0": 2500000},
                "cpu.governor_table": {"0": ["walt", "schedutil"]},
                "cpu.freq_table": {"0": [384000, 500000, 1200000, 1800000, 2000000, 2500000]},
                "cpu.adaptive": true
            }""".trimIndent(),
        )
        val settings = Json.parseToJsonElement(
            """{"cpu": {
                "cpu.min_freq": {"0": 384000},
                "cpu.max_freq": {"0": 2000000},
                "cpu.governor": {"0": "walt"}
            }}""",
        ) as kotlinx.serialization.json.JsonObject
        val defaults = Json.parseToJsonElement(
            """{"cpu": {
                "cpu.min_freq": {"0": 384000},
                "cpu.max_freq": {"0": 2500000},
                "cpu.governor": {"0": "schedutil"}
            }}""",
        ) as kotlinx.serialization.json.JsonObject

        val snapshot = CpuSnapshot.from(live, settings, defaults)!!
        val draft = CpuDraft.from(snapshot)

        assertEquals(1200000L, snapshot.maxFreq.getValue("0"))
        assertEquals(2000000L, snapshot.staticMax("0"))
        assertEquals(384000L, snapshot.staticMin("0"))
        assertEquals(listOf("walt", "schedutil"), snapshot.governorTable.getValue("0"))
        assertTrue(snapshot.adaptiveEnabled)
        assertFalse(draft.isDirty(snapshot))

        val changed = draft.withRange("0", 500000L, 1800000L)
        assertTrue(changed.isDirty(snapshot))
        assertEquals(1800000L, changed.diff(snapshot)!![ITEM_MAX]
            ?.jsonObject?.get("0")?.jsonPrimitive?.longOrNull)
    }

    @Test
    fun clusterApplyDiffDoesNotIncludeOtherPolicies() {
        val base = snapshot().copy(
            clusters = mapOf("0" to listOf(0), "6" to listOf(6)),
            configuredMinFreq = mapOf("0" to 384_000L, "6" to 1_020_000L),
            configuredMaxFreq = mapOf("0" to 2_000_000L, "6" to 1_900_000L),
            configuredGovernor = mapOf("0" to "walt", "6" to "schedutil"),
        )
        val draft = CpuDraft.from(base)
            .withRange("0", 500_000L, 1_800_000L)
            .withRange("6", 1_100_000L, 1_700_000L)

        val clusterZero = draft.diff(base, "0")!!

        assertEquals(setOf("0"), clusterZero[ITEM_MAX]?.jsonObject?.keys)
        assertEquals(setOf("0"), clusterZero[ITEM_MIN]?.jsonObject?.keys)
        assertNull(clusterZero[ITEM_ONLINE])
    }

    @Test
    fun firstSampleDoesNotPretendToKnowLoad() {
        val current = snapshot(
            stats = mapOf("cpu0" to CpuUsageCounter(100, 200)),
            tis = mapOf("0" to mapOf(300_000L to 10L)),
        )

        val metrics = CpuLiveTracker.update(null, current)

        assertNotNull(metrics.coreFreqKhz[0])
        assertNull(metrics.coreUsage[0])
    }

    private fun snapshot(
        stats: Map<String, CpuUsageCounter> = emptyMap(),
        tis: Map<String, Map<Long, Long>> = emptyMap(),
        clusterHw: Map<String, Long> = emptyMap(),
        clusterCur: Map<String, Long> = emptyMap(),
    ) = CpuSnapshot(
        online = listOf(1),
        clusters = mapOf("0" to listOf(0)),
        maxFreq = emptyMap(),
        minFreq = emptyMap(),
        governor = emptyMap(),
        curFreq = mapOf("0" to 900_000L),
        clusterCurFreq = clusterCur,
        clusterHwCurFreq = clusterHw,
        timeInState = tis,
        // `CpuDraft.diff` only emits a cluster when it has a usable table (>=2 positive entries), so
        // the fixture must provide one for every cluster a test exercises.
        frequencyTable = mapOf(
            "0" to listOf(300_000L, 1_200_000L, 1_800_000L, 2_000_000L),
            "6" to listOf(1_020_000L, 1_400_000L, 1_700_000L, 1_900_000L),
        ),
        governorTable = emptyMap(),
        usageStat = stats,
        hwMin = emptyMap(),
        hwMax = emptyMap(),
        configuredMinFreq = emptyMap(),
        configuredMaxFreq = emptyMap(),
        configuredGovernor = emptyMap(),
        defaultMinFreq = emptyMap(),
        defaultMaxFreq = emptyMap(),
        defaultGovernor = emptyMap(),
        adaptiveEnabled = false,
        screenOffActive = false,
        tempDeciCelsius = null,
        raw = buildJsonObject {},
    )
}
