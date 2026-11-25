package io.sysflow.feature.mem

import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.jsonObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/** Parsing rules for `docs/protocol.md` §3.22–§3.23, on the values a real device reported. */
class MemModelsTest {

    private fun json(text: String): JsonObject = Json.parseToJsonElement(text).jsonObject

    private val deviceSnapshot = json(
        """
        {
          "mem.total": 11379692, "mem.available": 4799700, "mem.free": 213804,
          "mem.cached": 4628356, "mem.anon": 2666388, "mem.slab": 860048, "mem.dirty": 948,
          "mem.swap_total": 12582908, "mem.swap_free": 9709704,
          "mem.psi.full_avg10": 317, "mem.psi.some_avg10": 372,
          "mem.zram.count": 1, "mem.zram.disksize": 12582912,
          "mem.zram.orig": 2872472, "mem.zram.compr": 975272,
          "mem.zram.mem_used": 1006020, "mem.zram.ratio": 295, "mem.zram.writeback": 1,
          "mem.mglru": 1, "mem.swappiness": 130, "mem.vfs_cache_pressure": 100,
          "mem.kernel_stack": 116456, "mem.page_tables": 244788, "mem.percpu": 13344,
          "mem.vmalloc_used": 358584, "mem.gpu": 407392,
          "mem.vendor.rbin": 409600, "mem.vendor.cma": 638976, "mem.vendor.dmaheap": 22528
        }
        """,
    )

    @Test
    fun snapshot_parses_the_protocol_keys_into_segments() {
        val snapshot = requireNotNull(parseMemSnapshot(deviceSnapshot))
        assertEquals(11_379_692L, snapshot.totalKb)
        assertEquals(4_799_700L, snapshot.availableKb)
        assertEquals(42, snapshot.availablePercent)
        assertEquals(6_579_992L, snapshot.usedKb)
        assertEquals(2_873_204L, snapshot.swapUsedKb)
        assertEquals(317, snapshot.psiFullPct)
        assertEquals(295, snapshot.zramRatio)
        assertTrue(snapshot.zramWriteback)
        assertTrue(snapshot.mglru)
        assertEquals(130, snapshot.swappiness)

        // Measured slices: anon, slab, gpu, kernel stack, page tables, vmalloc, percpu.
        // `Cached` is not one of them — it is reclaimable and already inside `MemAvailable`.
        assertEquals(7, snapshot.measured.size)
        assertEquals(3, snapshot.pools.size)
        assertTrue(snapshot.pools.all { it.kind == MemKind.Pool })

        // The "used" slices have to add up to what the device says is used, or the caption lies.
        assertEquals(snapshot.usedKb, snapshot.segments.sumOf { it.kb })
        assertTrue(snapshot.otherKb > 0)

        // The whole-memory bar adds cache and idle on top: used + cache + idle == total.
        assertEquals(snapshot.totalKb, snapshot.fullSegments.sumOf { it.kb })
        assertEquals(MemKind.Reclaimable, snapshot.cachedSegment?.kind)
        assertTrue(snapshot.idleKb >= 0)
        assertTrue(snapshot.fullSegments.any { it.kind == MemKind.Idle })
        // Cache is reclaimable, so it must never appear among the "used" slices.
        assertTrue(snapshot.segments.none { it.kind == MemKind.Reclaimable })
    }

    @Test
    fun pools_are_clamped_when_they_do_not_fit_in_what_is_left() {
        // 100 kB free of 1000, but the vendor pools claim 400 kB of capacity.
        val snapshot = parseMemSnapshot(
            json(
                """
                {"mem.total": 1000, "mem.available": 100, "mem.anon": 800,
                 "mem.vendor.cma": 400}
                """,
            ),
        )!!
        // used = 900, measured = 800, so only 100 kB can belong to pools.
        assertEquals(900L, snapshot.usedKb)
        assertEquals(100L, snapshot.pools.sumOf { it.kb })
        assertEquals(0L, snapshot.otherKb)
        assertEquals(900L, snapshot.segments.sumOf { it.kb })
    }

    @Test
    fun a_snapshot_without_the_required_keys_is_absent() {
        assertNull(parseMemSnapshot(null))
        assertNull(parseMemSnapshot(json("{}")))
    }

    @Test
    fun apps_parse_entries_flags_and_the_estimate_marker() {
        val apps = parseMemApps(
            json(
                """
                {"available": true, "cached": true, "total_pss_kb": 1050000, "skipped": 1,
                 "pss_for_top_n_only": true,
                 "entries": [
                   {"uid": "u0a346", "numeric_uid": 10346, "name": "com.example.app",
                    "packages": ["com.example.app"], "pss_kb": 480000, "swap_pss_kb": 96000,
                    "procs": 2, "share": 0.457}
                 ]}
                """,
            ),
        )!!
        assertTrue(apps.available)
        assertTrue(apps.cached)
        assertTrue(apps.estimatedTail)
        assertEquals(1, apps.skipped)
        assertEquals(1, apps.entries.size)
        assertEquals("com.example.app", apps.entries[0].name)
        assertEquals(480_000L, apps.entries[0].pssKb)
        assertEquals(2, apps.entries[0].procs)
    }

    @Test
    fun an_unavailable_ranking_is_modelled_rather_than_thrown_away() {
        val apps = parseMemApps(json("""{"available": false, "entries": []}"""))!!
        assertFalse(apps.available)
        assertTrue(apps.entries.isEmpty())
    }

    @Test
    fun breakdown_parses_the_system_and_user_split() {
        val breakdown = parseMemBreakdown(
            json(
                """
                {"available": true, "system_pss_kb": 2100000, "user_pss_kb": 2600000,
                 "total_pss_kb": 4700000, "processes": 523, "measured": 519,
                 "estimated": 3, "skipped": 1}
                """,
            ),
        )!!
        assertTrue(breakdown.available)
        assertEquals(2_100_000L, breakdown.systemPssKb)
        assertEquals(2_600_000L, breakdown.userPssKb)
        assertEquals(4_700_000L, breakdown.totalPssKb)
        assertEquals(523, breakdown.processes)
        assertEquals(1, breakdown.skipped)
    }

    @Test
    fun history_parses_series_points_and_reports_empty() {
        val history = parseMemHistory(
            json(
                """
                {"module": "mem", "step_s": 1800,
                 "series": {"psi_full_pct": [[1795573049, 2000], [1795574849, 1000]],
                            "available_kb": [[1795573049, 4799700]]}}
                """,
            ),
        )
        assertEquals(2, history.psiFull.size)
        assertEquals(2000f, history.psiFull[0].y, 0.001f)
        assertEquals(1_795_573_049L, history.psiFull[0].x)
        assertEquals(1, history.available.size)
        assertFalse(history.isEmpty)
        assertTrue(parseMemHistory(json("""{"series": {}}""")).isEmpty)
    }
}
