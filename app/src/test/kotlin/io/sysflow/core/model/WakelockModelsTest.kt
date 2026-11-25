package io.sysflow.core.model

import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.jsonObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/** Parsing for `wakelock.*` payloads (protocol §3.18–§3.19). */
class WakelockModelsTest {
    private val payload = Json.parseToJsonElement(
        """
        {
          "wakelock.kernel_sources": [
            {"name":"PowerManagerService.WakeLocks","active":true,"active_count":4,
             "event_count":6,"wakeup_count":2,"expire_count":0,"active_since_ms":1234,
             "total_ms":482348,"max_ms":300000,"prevent_suspend_ms":0},
            {"name":"wlan_wow_wl","active":false,"active_count":0,"event_count":0,
             "wakeup_count":5,"expire_count":0,"active_since_ms":0,
             "total_ms":12044,"max_ms":12000,"prevent_suspend_ms":900}
          ],
          "wakelock.ranked": [
            {"name":"PowerManagerService.WakeLocks","awake_ms":300000,"share":0.75},
            {"name":"wlan_wow_wl","awake_ms":100000,"share":0.25}
          ],
          "wakelock.summary": {
            "available":true,"known":2,"active":1,"window_s":3600,
            "updated_s":1795452252,"ranking_available":true
          }
        }
        """.trimIndent(),
    ).jsonObject

    @Test
    fun parsesKernelTelemetry() {
        val state = WakelockState.from(payload)
        assertNotNull(state)
        assertEquals(2, state!!.sources.size)
        assertTrue(state.sources[0].active)
        assertEquals(482_348L, state.sources[0].totalMs)
        assertEquals(5, state.sources[1].wakeupCount)
        assertEquals(2, state.ranked.size)
        assertEquals(0.75, state.ranked[0].share, 1e-9)
        assertEquals(1, state.summary.active)
        assertEquals(3600L, state.summary.windowSeconds)
        assertTrue(state.summary.rankingAvailable)
    }

    @Test
    fun missingSummaryMeansNoState() {
        assertNull(WakelockState.from(null))
        assertNull(WakelockState.from(Json.parseToJsonElement("{}").jsonObject))
    }

    @Test
    fun unavailableKernelTableStillParses() {
        val state = WakelockState.from(
            Json.parseToJsonElement(
                """{"wakelock.kernel_sources":[],"wakelock.ranked":[],
                   "wakelock.summary":{"available":false,"known":0,"active":0,
                   "window_s":0,"updated_s":0,"ranking_available":false}}""",
            ).jsonObject,
        )
        assertNotNull(state)
        assertFalse(state!!.summary.available)
        assertTrue(state.sources.isEmpty())
    }

    @Test
    fun parsesHeldLocksAndTreatsUnavailableAsNull() {
        val held = WakelockHeld.from(
            Json.parseToJsonElement(
                """{"available":true,"count":1,"cached":true,"held":[
                   {"kind":"PARTIAL_WAKE_LOCK","tag":"AudioMix","held_ms":62000,
                    "uid":1041,"pid":1234,"package":"com.android.systemui"}]}""",
            ).jsonObject,
        )
        assertNotNull(held)
        assertEquals(1, held!!.count)
        assertEquals("AudioMix", held.locks[0].tag)
        assertEquals(62_000L, held.locks[0].heldMs)
        assertEquals("com.android.systemui", held.locks[0].packageName)
        assertTrue(held.cached)

        assertNull(
            WakelockHeld.from(
                Json.parseToJsonElement("""{"available":false,"held":[]}""").jsonObject,
            ),
        )
    }

    @Test
    fun parsesPerAppWakeTime() {
        val apps = WakelockApps.from(
            Json.parseToJsonElement(
                """{"available":true,"generated_at":1,"cached":false,"total_ms":100000,
                   "entries":[
                     {"uid":"u0a346","numeric_uid":10346,"name":"com.example.worker",
                      "packages":["com.example.worker"],"total_ms":60000,"count":5,"share":0.6,
                      "tags":[{"tag":"*job*e//worker","total_ms":60000,"count":5}]},
                     {"uid":"1000","numeric_uid":1000,"name":"Android 系统","packages":[],
                      "total_ms":40000,"count":175,"share":0.4,"tags":[]}]}""",
            ).jsonObject,
        )
        assertNotNull(apps)
        assertEquals(2, apps!!.entries.size)
        assertEquals(60_000L, apps.entries[0].totalMs)
        assertEquals(10_346, apps.entries[0].numericUid)
        assertEquals("*job*e//worker", apps.entries[0].tags[0].tag)
        assertEquals(1000, apps.entries[1].numericUid)
        assertEquals(100_000L, apps.totalMs)
    }

    @Test
    fun unavailableAppStatsAreNull() {
        assertNull(
            WakelockApps.from(
                Json.parseToJsonElement("""{"available":false,"entries":[]}""").jsonObject,
            ),
        )
    }

    @Test
    fun mergesRankingAndBlockedRulesIntoOneTable() {
        val apps = WakelockApps.from(
            Json.parseToJsonElement(
                """{"available":true,"total_ms":100000,"entries":[
                     {"uid":"u0a346","numeric_uid":10346,"name":"Worker",
                      "packages":["com.example.worker"],"total_ms":60000,"count":5,"share":0.6,
                      "tags":[{"tag":"*job*e//worker","total_ms":60000,"count":5}]},
                     {"uid":"1000","numeric_uid":1000,"name":"Android 系统","packages":[],
                      "total_ms":40000,"count":175,"share":0.4,"tags":[]}]}""",
            ).jsonObject,
        )

        val rows = apps.mergeWithRules(setOf("com.example.worker", "com.example.gone"))

        assertEquals(3, rows.size)
        // Ranking order first: longest time wins.
        assertEquals("Worker", rows[0].title)
        assertTrue("the rule set marks this row blocked", rows[0].blocked)
        assertEquals("*job*e//worker", rows[0].caption)
        // A system UID has no package, so it can never be blocked.
        assertNull(rows[1].packageName)
        assertFalse(rows[1].blocked)
        // Blocked but absent from this ranking: shown with no time so the rule stays reachable.
        assertEquals("com.example.gone", rows[2].title)
        assertNull(rows[2].totalMs)
        assertTrue(rows[2].blocked)
    }

    @Test
    fun mergeWorksBeforeTheRankingIsLoaded() {
        val rows = (null as WakelockApps?).mergeWithRules(setOf("com.example.blocked"))

        assertEquals(1, rows.size)
        assertEquals("com.example.blocked", rows.single().title)
        assertNull(rows.single().totalMs)
        assertTrue(rows.single().blocked)
    }

    @Test
    fun mergeFlagsProtectedApps() {
        val rows = (null as WakelockApps?).mergeWithRules(setOf("com.android.phone"))

        assertTrue(rows.single().protected)
    }

    @Test
    fun explainsWhyTheRankingIsEmpty() {
        val base = WakelockSummary(
            available = true,
            known = 191,
            active = 2,
            windowSeconds = 0,
            updatedSeconds = 0,
            rankingAvailable = false,
        )
        // Not sampled twice yet: "collecting", never "nothing happened".
        assertTrue(wakeRankHint(base).contains("正在收集"))
        // Window just opened: still collecting, now with an age.
        val young = base.copy(rankingAvailable = true, windowSeconds = 60, updatedSeconds = 1)
        assertTrue(wakeRankHint(young).contains("正在收集"))
        assertTrue(wakeRankHint(young).contains("1m0s"))
        // A full window with no movement really is idle.
        val idle = young.copy(windowSeconds = 3_600)
        assertTrue(wakeRankHint(idle).contains("空闲"))
        // Unreadable kernel table is a third, different case.
        assertTrue(wakeRankHint(base.copy(available = false)).contains("不可读"))
    }

    @Test
    fun groupsRankingByDriverFamilyBiggestFirst() {
        val entries = listOf(
            WakelockRankEntry("rmnet_ipa%d", "network", 60_000, 0.5),
            WakelockRankEntry("rmnet_ctl", "network", 20_000, 0.2),
            WakelockRankEntry("hal_bluetooth_lock", "bluetooth", 30_000, 0.25),
            WakelockRankEntry("TSP", "touch", 5_000, 0.05),
        )

        val groups = entries.rankingGroups()

        assertEquals(listOf("network", "bluetooth", "touch"), groups.map { it.category })
        assertEquals(80_000L, groups[0].totalMs)
        assertEquals("网络", groups[0].label)
        // Within a group the longest source comes first.
        assertEquals(listOf("rmnet_ipa%d", "rmnet_ctl"), groups[0].rows.map { it.name })
    }

    @Test
    fun groupsCumulativeSourcesAndLabelsCategories() {
        val sources = listOf(
            WakelockSource("smp2p-sleepstate", "subsystem", false, 10_000, 10_000, 0, 0, 0, 0),
            WakelockSource("nfc_wake_lock", "nfc", false, 4_000, 4_000, 0, 0, 0, 0),
            WakelockSource("sec-battery-monitor", "power", true, 30_000, 30_000, 0, 0, 0, 0),
        )

        val groups = sources.sourceGroups()

        assertEquals("power", groups[0].category)
        assertEquals("充电与电池", groups[0].label)
        assertEquals("子系统通信", groups[1].label)
        assertEquals("NFC", groups[2].label)
    }

    @Test
    fun unknownCategoryFallsBackToAReadableLabel() {
        // An older module (or a new driver family) must never surface a raw identifier.
        assertEquals("其它驱动", wakeCategoryLabel("quantum_flux"))
        assertEquals("网络", wakeCategoryLabel("network"))
    }

    @Test
    fun parsesCategoryFromPayloadAndDefaultsWhenMissing() {
        val withCategory = WakelockState.from(
            Json.parseToJsonElement(
                """{"wakelock.kernel_sources":[{"name":"TSP","category":"touch","active":false,
                     "total_ms":10,"max_ms":10,"wakeup_count":0,"event_count":0,
                     "active_since_ms":0,"prevent_suspend_ms":0}],
                   "wakelock.ranked":[{"name":"rmnet_ctl","category":"network","awake_ms":5,"share":1.0}],
                   "wakelock.summary":{"available":true,"known":1,"active":0,"window_s":60,
                     "updated_s":1,"ranking_available":true}}""",
            ).jsonObject,
        )
        assertEquals("touch", withCategory!!.sources[0].category)
        assertEquals("network", withCategory.ranked[0].category)

        val legacy = WakelockState.from(payload) // fixture has no category fields
        assertEquals("other", legacy!!.sources[0].category)
        assertEquals("other", legacy.ranked[0].category)
    }

    @Test
    fun detectsWhetherTheModuleSendsCategories() {
        val withCategories = WakelockState.from(
            Json.parseToJsonElement(
                """{"wakelock.kernel_sources":[{"name":"TSP","category":"touch"}],
                   "wakelock.ranked":[{"name":"TSP","category":"touch","awake_ms":1,"share":1.0}],
                   "wakelock.summary":{"available":true,"known":1,"active":0,"window_s":1,
                     "updated_s":1,"ranking_available":true}}""",
            ).jsonObject,
        )
        assertTrue(withCategories!!.categorySupported)

        // The round-1 fixture has no `category` keys at all: that module predates classification and
        // the page must tell the user to update it instead of showing one anonymous bucket.
        val legacy = WakelockState.from(payload)
        assertFalse(legacy!!.categorySupported)
    }

    @Test
    fun formatsSamplingAge() {
        assertEquals("刚刚", formatAgeSeconds(30))
        assertEquals("3 分钟前", formatAgeSeconds(180))
        assertEquals("2 小时前", formatAgeSeconds(7_200))
        assertEquals("2 天前", formatAgeSeconds(172_800))
    }

    @Test
    fun formatsAwakeDurations() {
        assertEquals("0s", formatAwakeMs(0))
        assertEquals("45s", formatAwakeMs(45_000))
        assertEquals("1m2s", formatAwakeMs(62_000))
        assertEquals("1h30m", formatAwakeMs(5_400_000))
        assertEquals("2d3h", formatAwakeMs(183_600_000))
    }

    @Test
    fun mirrorsTheModulesProtectionList() {
        assertTrue(WakelockProtection.isProtected("com.android.phone"))
        assertTrue(WakelockProtection.isProtected("io.sysflow"))
        assertFalse(WakelockProtection.isProtected("com.example.app"))
    }

    @Test
    fun aRebootIsExplainedInsteadOfCalledIdle() {
        val rebooted = WakelockSummary(
            available = true,
            known = 191,
            active = 2,
            windowSeconds = 120,
            updatedSeconds = 1,
            rankingAvailable = true,
            windowReason = "reboot",
        )
        val fresh = wakeRankHint(rebooted)
        assertTrue(fresh.contains("重启"))
        assertFalse(fresh.contains("空闲"))

        // Window older than one sampling interval: still not "idle", because it only covers the
        // time since the reboot.
        val older = rebooted.copy(windowSeconds = 3_600)
        assertTrue(wakeRankHint(older).contains("自重启以来"))
        assertFalse(wakeRankHint(older).contains("窗口内没有新增"))
    }

    @Test
    fun parsesWindowReasonAndMeasuredTotal() {
        val state = WakelockState.from(
            Json.parseToJsonElement(
                """{"wakelock.summary":{"available":true,"known":10,"active":1,
                     "window_s":600,"updated_s":1795452252,"ranking_available":true,
                     "window_reason":"reboot","since_boot_s":600,"measured_ms":123456}}""",
            ).jsonObject,
        )!!
        assertEquals("reboot", state.summary.windowReason)
        assertEquals(600L, state.summary.sinceBootSeconds)
        assertEquals(123_456L, state.summary.measuredMs)
    }

    @Test
    fun parsesTheProtectionPolicyFromTheModule() {
        val state = WakelockState.from(
            Json.parseToJsonElement(
                """{"wakelock.summary":{"available":true,"known":1,"active":0,"window_s":1,
                     "updated_s":1,"ranking_available":true},
                    "wakelock.protected":["com.android.phone","com.example.custom"]}""",
            ).jsonObject,
        )!!
        assertEquals(listOf("com.android.phone", "com.example.custom"), state.protectedPackages)
    }

    @Test
    fun thePublishedPolicyWinsOverTheLocalFallback() {
        assertTrue(isProtectedPackage("com.example.custom", setOf("com.example.custom")))
        // A package the local copy lists is *not* protected if the module says otherwise: the
        // module is the authority, so an OEM-specific exception can be honoured.
        assertFalse(isProtectedPackage("com.android.phone", setOf("com.example.custom")))
    }

    @Test
    fun anEmptyPolicyFallsBackToTheLocalList() {
        assertTrue(isProtectedPackage("com.android.phone"))
        assertFalse(isProtectedPackage("com.example.app"))
    }

    @Test
    fun mergeUsesThePublishedPolicy() {
        val rows = (null as WakelockApps?).mergeWithRules(
            blocked = setOf("com.example.custom", "com.android.phone"),
            protectedPackages = setOf("com.example.custom"),
        )
        val byName = rows.associateBy { it.packageName }
        assertEquals(true, byName.getValue("com.example.custom").protected)
        // The module omitted the phone app from its policy, so the UI must not disable it.
        assertEquals(false, byName.getValue("com.android.phone").protected)
    }

    @Test
    fun parsesWakeupCountsAndTheWindowTotal() {
        val state = WakelockState.from(
            Json.parseToJsonElement(
                """{"wakelock.ranked":[{"name":"TSP","category":"touch","awake_ms":1000,
                     "share":0.1,"wakeups":42}],
                    "wakelock.summary":{"available":true,"known":2,"active":0,"window_s":600,
                     "updated_s":1,"ranking_available":true,"measured_ms":9000,
                     "total_wakeups":42}}""",
            ).jsonObject,
        )!!
        assertEquals(42L, state.ranked.single().wakeups)
        assertEquals(42L, state.summary.totalWakeups)
    }

    @Test
    fun groupsTheRankingByCountWhenAsked() {
        // A chatty timer versus a rare long hogs: different winners for time and for count.
        val ranked = listOf(
            WakelockRankEntry("rmnet_ipa%d", "network", 1_000, 0.02, wakeups = 50),
            WakelockRankEntry("alarmtimer.0.auto", "timer", 60_000, 0.9, wakeups = 1),
        )
        val byTime = ranked.rankingGroups()
        assertEquals("timer", byTime.first().category)
        val byCount = ranked.rankingGroups(byCount = true)
        assertEquals("network", byCount.first().category)
        assertEquals(50L, byCount.first().totalMs)
    }

    @Test
    fun parsesTheLastWakeupAndItsUnavailableShape() {
        val reported = WakelockWakeup.from(
            Json.parseToJsonElement(
                """{"available":true,"reason":"1234","age_ms":60000,"cached":false}""",
            ).jsonObject,
        )!!
        assertEquals("1234", reported.reason)
        assertEquals(60_000L, reported.ageMs)
        assertTrue(reported.available)

        val unsupported = WakelockWakeup.from(
            Json.parseToJsonElement(
                """{"available":false,"reason":null,"age_ms":null,"note":"ROM 未提供"}""",
            ).jsonObject,
        )!!
        assertFalse(unsupported.available)
        assertNull(unsupported.reason)
        assertEquals("ROM 未提供", unsupported.note)
        assertNull(WakelockWakeup.from(null))
    }

    @Test
    fun joinsTheHistorySeriesByTimestamp() {
        val history = WakelockHistory.from(
            Json.parseToJsonElement(
                """{"module":"wakelock","step_s":1800,"points":2,"daemon_running":true,
                     "series":{"top_ms":[[100,1000],[200,3000]],
                               "wakeups":[[100,5],[300,9]]}}""",
            ).jsonObject,
        )!!
        assertEquals(listOf(100L, 200L, 300L), history.points.map { it.ts })
        assertEquals(3_000L, history.points[1].topMs)
        // A bucket may carry only one of the two metrics.
        assertEquals(0L, history.points[1].wakeups)
        assertEquals(9L, history.points[2].wakeups)
        assertTrue(history.daemonRunning)
    }

    @Test
    fun aHeldLockWithoutAPackageKeepsItsResolvedOwner() {
        // Samsung dumps carry only `uid=`; the module resolves a display owner and sends a real
        // JSON null for the package. That must stay null, not become a package named "null".
        val held = WakelockHeld.from(
            Json.parseToJsonElement(
                """{"available":true,"count":1,"cached":false,"held":[
                     {"kind":"PARTIAL_WAKE_LOCK","tag":"*vibrator*","held_ms":39,
                      "uid":1000,"pid":14922,"package":null,"owner":"Android 系统"}]}""",
            ).jsonObject,
        )!!
        val lock = held.locks.single()
        assertNull(lock.packageName)
        assertEquals("Android 系统", lock.owner)
        assertEquals(1000, lock.uid)
    }

    // ---- apply read-back (protocol §3.18) -----------------------------------

    private fun applyResult(applied: String, mismatch: List<String> = emptyList()): ApplyResult =
        ApplyResult(
            applied = Json.parseToJsonElement(applied).jsonObject,
            requested = JsonObject(emptyMap()),
            mismatch = mismatch,
        )

    @Test
    fun appliedReadbackIsUnpackedFromTheItemEnvelope() {
        val result = applyResult("""{"wakelock.blocked":{"com.example.app":true}}""")
        assertEquals(mapOf("com.example.app" to true), appliedBlocked(result))
    }

    @Test
    fun aRejectedApplyReadsBackAsFalseInsteadOfSuccess() {
        // What a ROM that silently keeps the old mode actually returns.
        val result = applyResult(
            """{"wakelock.blocked":{"com.example.app":false}}""",
            mismatch = listOf(WAKELOCK_BLOCKED_ITEM),
        )
        assertEquals(false, appliedBlocked(result)["com.example.app"])
    }

    @Test
    fun missingReadbackStaysUnknown() {
        assertNull(appliedBlocked(applyResult("{}"))["com.example.app"])

        // A multi-package request where only one package was accepted: the other stays unknown
        // instead of being assumed applied.
        val partial = applyResult(
            """{"wakelock.blocked":{"com.ok":true}}""",
            mismatch = listOf(WAKELOCK_BLOCKED_ITEM),
        )
        assertEquals(mapOf("com.ok" to true), appliedBlocked(partial))
        assertNull(appliedBlocked(partial)["com.rejected"])
    }

    @Test
    fun flattenedReadbackStillParses() {
        // Tolerated so a module that answers without the item envelope keeps working.
        assertEquals(
            mapOf("com.example.app" to true),
            appliedBlocked(applyResult("""{"com.example.app":true}""")),
        )
    }
}
