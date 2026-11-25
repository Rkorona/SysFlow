package io.sysflow.core.model

import kotlinx.serialization.json.add
import kotlinx.serialization.json.buildJsonArray
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.put
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/** `withoutCluster` must revert one cluster's pending edits and leave the others untouched. */
class CpuDraftDiscardTest {

    private fun snapshot(): CpuSnapshot = CpuSnapshot.from(
        buildJsonObject {
            put(
                "cpu.clusters",
                buildJsonObject {
                    put("0", buildJsonArray { add(0); add(1) })
                    put("4", buildJsonArray { add(4); add(5) })
                },
            )
            put(
                "cpu.max_freq",
                buildJsonObject {
                    put("0", 1_800_000)
                    put("4", 2_400_000)
                },
            )
            put(
                "cpu.min_freq",
                buildJsonObject {
                    put("0", 300_000)
                    put("4", 500_000)
                },
            )
            put(
                "cpu.governor",
                buildJsonObject {
                    put("0", "schedutil")
                    put("4", "schedutil")
                },
            )
            put(
                "cpu.freq_table",
                buildJsonObject {
                    put("0", buildJsonArray { add(300_000); add(1_800_000) })
                    put("4", buildJsonArray { add(500_000); add(2_400_000) })
                },
            )
        },
    ) ?: error("snapshot fixture must parse")

    @Test
    fun discarding_one_cluster_reverts_only_that_cluster() {
        val snap = snapshot()
        val edited = CpuDraft.from(snap)
            .withRange("0", 500_000, 1_500_000)
            .withRange("4", 800_000, 2_000_000)
        assertTrue(edited.isClusterDirty(snap, "0"))
        assertTrue(edited.isClusterDirty(snap, "4"))

        val reverted = edited.withoutCluster("0")
        assertFalse("cluster 0 should be clean", reverted.isClusterDirty(snap, "0"))
        assertTrue("cluster 4 edits must survive", reverted.isClusterDirty(snap, "4"))
        // Reverting falls back to the snapshot values, it does not zero them out.
        assertEquals(null, reverted.maxFreq["0"])
        assertEquals(2_000_000L, reverted.maxFreq["4"])
    }

    @Test
    fun discarding_the_last_edit_clears_the_dirty_flag() {
        val snap = snapshot()
        val edited = CpuDraft.from(snap).withGovernor("0", "performance")
        assertTrue(edited.isDirty(snap))
        assertFalse(edited.withoutCluster("0").isDirty(snap))
    }
}
