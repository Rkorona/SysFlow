package io.sysflow.feature.mem

import io.sysflow.core.ui.components.ChartPoint
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.booleanOrNull
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.doubleOrNull
import kotlinx.serialization.json.intOrNull
import kotlinx.serialization.json.longOrNull
import java.util.Locale

/**
 * Model layer for the memory page.
 *
 * Everything here is a pure function over the protocol's JSON (`docs/protocol.md` §3.22–§3.23), so
 * the screen keeps no knowledge of key names and the parsing can be unit-tested without Android.
 */

/**
 * What a slice of the bar actually is. The distinction matters: only [Measured] and [Other] are
 * "used", [Pool] is a capacity, [Reclaimable] can be handed back at any time, and [Idle] is free.
 */
enum class MemKind { Measured, Pool, Other, Reclaimable, Idle }

/** One slice of the composition bar. */
data class MemSegment(
    val label: String,
    val kb: Long,
    val kind: MemKind = MemKind.Measured,
)

data class MemSnapshot(
    val totalKb: Long,
    val availableKb: Long,
    val freeKb: Long?,
    val cachedKb: Long?,
    val anonKb: Long?,
    val slabKb: Long?,
    val dirtyKb: Long?,
    val swapTotalKb: Long?,
    val swapFreeKb: Long?,
    /** Hundredths of a percent: 317 means 3.17 %. */
    val psiFullPct: Int?,
    val psiSomePct: Int?,
    val zramCount: Int?,
    val zramDisksizeKb: Long?,
    val zramOrigKb: Long?,
    val zramComprKb: Long?,
    val zramMemUsedKb: Long?,
    /** Integer percentage of compression: 295 means 2.95×. */
    val zramRatio: Int?,
    val zramWriteback: Boolean,
    /** Multi-generational LRU: it weakens what `swappiness` can promise. */
    val mglru: Boolean,
    /** Live values of the writable items, `null` when the kernel does not offer them. */
    val swappiness: Int?,
    val vfsCachePressure: Int?,
    val measured: List<MemSegment>,
    /** `cma` / `rbin` / `dmaheap`: pool sizes, kept out of the measured slices. */
    val pools: List<MemSegment>,
    /** Used memory not covered by anything above, clamped at zero. */
    val otherKb: Long,
) {
    /** "Used" in the sense Android reports it: total minus available, not total minus free. */
    val usedKb: Long get() = (totalKb - availableKb).coerceAtLeast(0)

    val availablePercent: Int
        get() = if (totalKb <= 0) 0 else ((availableKb * 100) / totalKb).toInt()

    val swapUsedKb: Long?
        get() = if (swapTotalKb == null || swapFreeKb == null) null else (swapTotalKb - swapFreeKb).coerceAtLeast(0)

    /** Slice of the bar that adds up to the *used* memory (no cache, no free). */
    val segments: List<MemSegment>
        get() = measured + pools + listOf(MemSegment("其他", otherKb, MemKind.Other))

    /** Page cache: shown on the bar, but as reclaimable — never as "used". */
    val cachedSegment: MemSegment?
        get() = cachedKb?.takeIf { it > 0 }?.let { MemSegment("缓存（可回收）", it, MemKind.Reclaimable) }

    /** Free memory that is not accounted for by the cache figure. */
    val idleKb: Long
        get() = (totalKb - usedKb - (cachedKb ?: 0)).coerceAtLeast(0)

    /**
     * The whole bar, in drawing order: used slices, reclaimable cache, then idle.
     *
     * This is the phone-settings view of RAM ("what is this device holding?") whereas [segments] is
     * the "what is used" view. They are different questions, so both exist.
     */
    val fullSegments: List<MemSegment>
        get() = (measured + pools + listOfNotNull(
            MemSegment("其他", otherKb, MemKind.Other).takeIf { it.kb > 0 },
            cachedSegment,
            MemSegment("空闲", idleKb, MemKind.Idle).takeIf { it.kb > 0 },
        ))
}

/** One row of `sysflow mem apps`. */
data class MemAppEntry(
    val uid: String,
    val numericUid: Long,
    val name: String,
    val packages: List<String>,
    val pssKb: Long,
    val swapPssKb: Long,
    val procs: Int,
    val share: Double,
)

data class MemApps(
    val available: Boolean,
    val cached: Boolean,
    val totalPssKb: Long,
    val skipped: Int,
    /** True when the tail of the ranking was estimated from RSS instead of measured. */
    val estimatedTail: Boolean,
    val entries: List<MemAppEntry>,
)

/** System-vs-user split (`sysflow mem breakdown`): PSS sums, not a share of "used". */
data class MemBreakdown(
    val available: Boolean,
    val systemPssKb: Long,
    val userPssKb: Long,
    val totalPssKb: Long,
    val processes: Int,
    val measured: Int,
    val estimated: Int,
    val skipped: Int,
)

fun parseMemBreakdown(value: JsonObject?): MemBreakdown? {
    val obj = value ?: return null
    return MemBreakdown(
        available = obj.bool("available") == true,
        systemPssKb = obj.long("system_pss_kb") ?: 0,
        userPssKb = obj.long("user_pss_kb") ?: 0,
        totalPssKb = obj.long("total_pss_kb") ?: 0,
        processes = obj.int("processes") ?: 0,
        measured = obj.int("measured") ?: 0,
        estimated = obj.int("estimated") ?: 0,
        skipped = obj.int("skipped") ?: 0,
    )
}

/** Curves read back from `sysflow history mem`. */
data class MemHistory(
    val psiFull: List<ChartPoint>,
    val psiSome: List<ChartPoint>,
    val available: List<ChartPoint>,
) {
    val isEmpty: Boolean get() = psiFull.isEmpty() && psiSome.isEmpty() && available.isEmpty()
}

// ---- parsing -------------------------------------------------------------------------------

private fun JsonObject.long(key: String): Long? =
    (this[key] as? JsonPrimitive)?.longOrNull

private fun JsonObject.int(key: String): Int? =
    (this[key] as? JsonPrimitive)?.intOrNull

private fun JsonObject.bool(key: String): Boolean? =
    (this[key] as? JsonPrimitive)?.booleanOrNull

/** `snapshot.modules.mem`, or `null` when the device does not support the module. */
fun parseMemSnapshot(value: JsonElement?): MemSnapshot? {
    val obj = value as? JsonObject ?: return null
    val total = obj.long("mem.total") ?: return null
    val available = obj.long("mem.available") ?: return null

    fun measured(label: String, kb: Long?): MemSegment? =
        kb?.takeIf { it > 0 }?.let { MemSegment(label, it) }

    // `Cached` is deliberately *not* a slice: page cache is reclaimable and already counted inside
    // `MemAvailable`, so adding it to "used" would push the bar past 100 % (9.3 GB of 6.6 GB on S25).
    val measured = listOfNotNull(
        measured("应用匿名", obj.long("mem.anon")),
        measured("Slab", obj.long("mem.slab")),
        measured("GPU", obj.long("mem.gpu")),
        measured("内核栈", obj.long("mem.kernel_stack")),
        measured("页表", obj.long("mem.page_tables")),
        measured("vmalloc", obj.long("mem.vmalloc_used")),
        measured("percpu", obj.long("mem.percpu")),
    )
    val pools = listOfNotNull(
        measured("CMA 池", obj.long("mem.vendor.cma"))?.copy(kind = MemKind.Pool),
        measured("Rbin 池", obj.long("mem.vendor.rbin"))?.copy(kind = MemKind.Pool),
        measured("DMA heap", obj.long("mem.vendor.dmaheap"))?.copy(kind = MemKind.Pool),
    )

    val used = (total - available).coerceAtLeast(0)
    val measuredKb = measured.sumOf { it.kb }
    val poolsKb = pools.sumOf { it.kb }
    // Pools are capacities, so they can exceed what is left after the measured slices; clamping keeps
    // the bar honest instead of letting the segments overflow the track.
    val remaining = (used - measuredKb).coerceAtLeast(0)
    val clampedPools = if (poolsKb <= remaining) {
        pools
    } else {
        // Shrink proportionally, so the bar keeps its proportions when the pools do not fit.
        pools.map { it.copy(kb = it.kb * remaining / poolsKb.coerceAtLeast(1)) }
    }
    val other = remaining - clampedPools.sumOf { it.kb }

    return MemSnapshot(
        totalKb = total,
        availableKb = available,
        freeKb = obj.long("mem.free"),
        cachedKb = obj.long("mem.cached"),
        anonKb = obj.long("mem.anon"),
        slabKb = obj.long("mem.slab"),
        dirtyKb = obj.long("mem.dirty"),
        swapTotalKb = obj.long("mem.swap_total"),
        swapFreeKb = obj.long("mem.swap_free"),
        psiFullPct = obj.int("mem.psi.full_avg10"),
        psiSomePct = obj.int("mem.psi.some_avg10"),
        zramCount = obj.int("mem.zram.count"),
        zramDisksizeKb = obj.long("mem.zram.disksize"),
        zramOrigKb = obj.long("mem.zram.orig"),
        zramComprKb = obj.long("mem.zram.compr"),
        zramMemUsedKb = obj.long("mem.zram.mem_used"),
        zramRatio = obj.int("mem.zram.ratio"),
        zramWriteback = obj.int("mem.zram.writeback") == 1,
        mglru = obj.int("mem.mglru") == 1,
        swappiness = obj.int("mem.swappiness"),
        vfsCachePressure = obj.int("mem.vfs_cache_pressure"),
        measured = measured,
        pools = clampedPools,
        otherKb = other,
    )
}

fun parseMemApps(value: JsonObject?): MemApps? {
    val obj = value ?: return null
    if (obj.bool("available") != true) {
        return MemApps(
            available = false,
            cached = false,
            totalPssKb = 0,
            skipped = 0,
            estimatedTail = false,
            entries = emptyList(),
        )
    }
    val entries = (obj["entries"] as? JsonArray).orEmpty().mapNotNull { element ->
        val row = element as? JsonObject ?: return@mapNotNull null
        val name = row.string("name") ?: return@mapNotNull null
        val packages = (row["packages"] as? JsonArray).orEmpty()
            .mapNotNull { (it as? JsonPrimitive)?.contentOrNull }
        MemAppEntry(
            uid = row.string("uid") ?: "?",
            numericUid = row.long("numeric_uid") ?: 0,
            name = name,
            packages = packages,
            pssKb = row.long("pss_kb") ?: 0,
            swapPssKb = row.long("swap_pss_kb") ?: 0,
            procs = row.int("procs") ?: 0,
            share = (row["share"] as? JsonPrimitive)?.doubleOrNull ?: 0.0,
        )
    }
    return MemApps(
        available = true,
        cached = obj.bool("cached") == true,
        totalPssKb = obj.long("total_pss_kb") ?: 0,
        skipped = obj.int("skipped") ?: 0,
        estimatedTail = obj.bool("pss_for_top_n_only") == true,
        entries = entries,
    )
}

fun parseMemHistory(value: JsonObject?): MemHistory {
    val series = value?.get("series") as? JsonObject ?: return MemHistory(emptyList(), emptyList(), emptyList())
    fun points(key: String): List<ChartPoint> =
        (series[key] as? JsonArray).orEmpty().mapNotNull { element ->
            val pair = element as? JsonArray ?: return@mapNotNull null
            if (pair.size < 2) return@mapNotNull null
            val x = (pair[0] as? JsonPrimitive)?.longOrNull ?: return@mapNotNull null
            val y = (pair[1] as? JsonPrimitive)?.longOrNull ?: return@mapNotNull null
            ChartPoint(x = x, y = y.toFloat())
        }
    return MemHistory(
        psiFull = points("psi_full_pct"),
        psiSome = points("psi_some_pct"),
        available = points("available_kb"),
    )
}

private fun JsonObject.string(key: String): String? =
    (this[key] as? JsonPrimitive)?.contentOrNull

private fun JsonArray?.orEmpty(): List<JsonElement> = this ?: emptyList()

// ---- formatting ----------------------------------------------------------------------------

/** kB → "4.58 GB" with two decimals, the unit the rest of the app shows for memory. */
fun formatKb(kb: Long): String = String.format(Locale.ROOT, "%.2f GB", kb / 1_048_576.0)

/** kB → "1.82 GB" style short form for a segment label. */
fun formatKbShort(kb: Long): String = String.format(Locale.ROOT, "%.2f GB", kb / 1_048_576.0)

/** Hundredths of a percent → "3.17 %". */
fun formatPsi(hundredths: Int?): String =
    if (hundredths == null) "—" else String.format(Locale.ROOT, "%.2f %%", hundredths / 100.0)

/** Compression ratio as "2.95×". */
fun formatRatio(percent: Int?): String =
    if (percent == null) "—" else String.format(Locale.ROOT, "%.2f×", percent / 100.0)
