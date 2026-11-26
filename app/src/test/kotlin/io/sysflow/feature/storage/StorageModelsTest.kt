package io.sysflow.feature.storage

import io.sysflow.core.model.Schema
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.decodeFromJsonElement
import kotlinx.serialization.json.jsonObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/** Parsing rules for `docs/protocol.md` §3.26, on shapes a real device reports. */
class StorageModelsTest {

    private fun json(text: String): JsonObject = Json.parseToJsonElement(text).jsonObject

    private val snapshotModule = json(
        """
        {
          "storage.data.total_kb": 100000, "storage.data.used_kb": 40000,
          "storage.data.free_kb": 60000, "storage.data.avail_kb": 55000,
          "storage.data.used_pct": 40,
          "storage.device.name": "sda", "storage.device.size_kb": 1000000,
          "storage.device.model": "Samsung UFS", "storage.device.rotational": false,
          "storage.device.health_pct": 80,
          "storage.scheduler": "mq-deadline", "storage.read_ahead_kb": 128,
          "storage.io.read_kb": 10000, "storage.io.write_kb": 4000,
          "storage.io.read_ios": 1000, "storage.io.write_ios": 500
        }
        """,
    )

    @Test
    fun snapshot_parses_capacity_device_and_queue_values() {
        val snapshot = requireNotNull(parseStorageSnapshot(snapshotModule))
        assertEquals(100_000L, snapshot.capacity.totalKb)
        assertEquals(40_000L, snapshot.capacity.usedKb)
        assertEquals(55_000L, snapshot.capacity.availKb)
        assertEquals(40, snapshot.capacity.usedPct)
        assertEquals(0.4f, snapshot.capacity.usedFraction, 0.0001f)
        assertEquals("sda", snapshot.device.name)
        assertEquals("Samsung UFS", snapshot.device.model)
        assertEquals(80, snapshot.device.healthPct)
        assertFalse(snapshot.device.rotational)
        assertEquals("mq-deadline", snapshot.scheduler)
        assertEquals(128, snapshot.readAheadKb)
        assertEquals(10_000L, snapshot.ioReadKb)
        assertNull("no separate /sdcard device in this payload", snapshot.sdcard)
    }

    @Test
    fun a_module_without_capacity_is_treated_as_absent() {
        assertNull(parseStorageSnapshot(null))
        assertNull(parseStorageSnapshot(json("""{"storage.scheduler": "kyber"}""")))
    }

    @Test
    fun io_parses_the_live_sample() {
        val io = parseStorageIo(
            json(
                """
                {"device":"sda","interval_ms":1000,"read_kb":2048,"write_kb":512,
                 "read_iops":12.0,"write_iops":3.0,"util_pct":4.5,
                 "total_read_kb":10000,"total_write_kb":4000}
                """,
            ),
        )
        assertEquals("sda", io.device)
        assertEquals(1000L, io.intervalMs)
        assertEquals(2048L, io.readKb)
        assertEquals(12.0, io.readIops, 0.0001)
        assertEquals(4.5, io.utilPct, 0.0001)
        assertEquals(10_000L, io.totalReadKb)
    }

    @Test
    fun usage_parses_partial_results_and_entries() {
        val usage = parseStorageUsage(
            json(
                """
                {"path":"/data","partial":true,"elapsed_ms":3120,"denied":2,"visited":20000,
                 "limit":20,"total_kb":5242880,
                 "entries":[{"name":"media","path":"/data/media","kb":3145728,"files":12034,"dirs":412},
                            {"name":"app","path":"/data/app","kb":2097152,"files":800,"dirs":200}]}
                """,
            ),
        )
        assertTrue(usage.partial)
        assertEquals(3120L, usage.elapsedMs)
        assertEquals(2L, usage.denied)
        assertEquals(2, usage.entries.size)
        assertEquals("media", usage.entries[0].name)
        assertEquals(3_145_728L, usage.entries[0].kb)
    }

    @Test
    fun history_reads_the_two_series() {
        val history = parseStorageHistory(
            json(
                """
                {"series":{"read_kb":[[1795600000,100],[1795603000,200]],
                           "write_kb":[[1795600000,10]]}}
                """,
            ),
        )
        assertEquals(2, history.read.size)
        assertEquals(1795603000L, history.read[1].x)
        assertEquals(200f, history.read[1].y, 0.001f)
        assertEquals(1, history.write.size)
        assertFalse(history.isEmpty)
        assertTrue(parseStorageHistory(null).isEmpty)
    }

    @Test
    fun schema_options_are_read_for_both_tunables() {
        val schema = Json.decodeFromJsonElement<Schema>(
            json(
                """
                {"module":"storage","supported":true,"items":[
                  {"id":"storage.scheduler","type":"enum","scope":"global","risk":"low","persist":true,
                   "options":["mq-deadline","kyber","none"]},
                  {"id":"storage.read_ahead_kb","type":"int_enum","scope":"global","risk":"low","persist":true,
                   "options":[128,256,1024]}
                ]}
                """,
            ),
        )
        val options = parseStorageOptions(schema)
        assertEquals(listOf("mq-deadline", "kyber", "none"), options.scheduler)
        assertEquals(listOf(128, 256, 1024), options.readAhead)
    }

    @Test
    fun partitions_parse_the_mount_overview() {
        val partitions = parseStoragePartitions(
            json(
                """
                {"available":true,"partitions":[
                  {"mount":"/data","device":"/dev/block/dm-5","fstype":"f2fs",
                   "total_kb":243000000,"used_kb":198000000,"free_kb":45000000,
                   "avail_kb":44000000,"used_pct":81},
                  {"mount":"/system","device":"/dev/block/dm-2","fstype":"erofs",
                   "total_kb":4200000,"used_kb":4100000,"free_kb":100000,
                   "avail_kb":100000,"used_pct":97}]}
                """,
            ),
        )
        assertEquals(2, partitions.size)
        assertEquals("/data", partitions[0].mount)
        assertEquals("f2fs", partitions[0].fstype)
        assertEquals(81, partitions[0].usedPct)
        assertEquals(198_000_000L, partitions[0].usedKb)
        assertEquals(0.81f, partitions[0].usedFraction, 0.01f)
    }

    @Test
    fun formatting_is_readable_and_degrades_with_a_dash() {
        assertEquals("—", formatStorageKb(null))
        assertEquals("1.0 GB", formatStorageKb(1024L * 1024L))
        assertEquals("512.0 MB", formatStorageKb(512L * 1024L))
        assertEquals("640 KB", formatStorageKb(640L))
        assertEquals("2.0 MB/s", formatStorageRate(2048L))
        assertEquals("0 KB/s", formatStorageRate(0L))
        assertEquals("12.3", formatIops(12.34))
        assertEquals("80%", formatStoragePct(80))
        assertEquals("—", formatStoragePct(null))
    }
}
