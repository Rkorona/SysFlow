package io.sysflow.feature.process

import io.sysflow.core.model.CapsData
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.booleanOrNull
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.doubleOrNull
import kotlinx.serialization.json.intOrNull
import kotlinx.serialization.json.longOrNull
import java.util.Locale

/**
 * Model layer for the process page.
 *
 * Pure functions over the protocol JSON (`docs/protocol.md` §3.25), so the screen never sees key
 * names and the parsing can be unit-tested without Android. Everything the module may omit is
 * nullable here: a kernel thread really has no package and no RSS.
 */

data class ProcessEntry(
    val pid: Int,
    val ppid: Int,
    val uid: Int,
    val user: Int,
    val appId: Int,
    val packageName: String?,
    val name: String,
    val cmdline: String,
    val state: String,
    val threads: Int,
    val rssKb: Long?,
    val cpuPct: Double?,
    val cpuTicks: Long?,
    val oomScoreAdj: Int?,
    val cgroup: String?,
    val cpuset: String?,
    val foreground: Boolean,
    val system: Boolean,
    val protected: Boolean,
    val protectedReason: String?,
) {
    /** A label that is always printable, even for kernel threads. */
    val displayName: String get() = name.ifBlank { "pid $pid" }
}

data class ProcessForeground(
    val available: Boolean,
    val packageName: String?,
    val pid: Int?,
    val user: Int,
    val cached: Boolean,
    val disabled: Boolean,
)

data class ProcessList(
    val available: Boolean,
    val reason: String?,
    val total: Int,
    val returned: Int,
    val truncated: Boolean,
    val skipped: Int,
    val sampleMs: Long,
    val clockTicks: Long,
    /** Existing `/dev/cpuset` groups; empty hides the cpuset action. */
    val cpusetGroups: List<String>,
    val foreground: ProcessForeground,
    val entries: List<ProcessEntry>,
)

/** One `get` response: a superset of [ProcessEntry] plus PSS, scheduler and cgroup detail. */
data class ProcessDetail(
    val pid: Int,
    val ppid: Int,
    val uid: Int,
    val user: Int,
    val appId: Int,
    val packageName: String?,
    val packages: List<String>,
    val name: String,
    val cmdline: List<String>,
    val state: String,
    val threads: Int,
    val rssKb: Long?,
    val pssKb: Long?,
    val swapPssKb: Long?,
    val cpuPct: Double?,
    val sampleMs: Long,
    val oomScoreAdj: Int?,
    val nice: Int,
    val rtPriority: Int,
    val policy: Int,
    val cgroup: String?,
    val cpuset: String?,
    val startTimeTicks: Long,
    val foreground: Boolean,
    val system: Boolean,
    val protected: Boolean,
    val protectedReason: String?,
)

/** One process affected by an action. `verified == null` means "not verifiable", not "failed". */
data class ProcessActionTarget(
    val pid: Int,
    val before: String?,
    val applied: String?,
    val verified: Boolean?,
    val error: String?,
    val mechanism: String?,
)

data class ProcessActionResult(
    val target: String?,
    val adjustment: Int?,
    val nice: Int?,
    val group: String?,
    val frozen: Boolean?,
    val mechanism: String?,
    val alreadyStopped: Boolean,
    val pidsBefore: List<Int>,
    val pidsAfter: List<Int>,
    val verified: Boolean?,
    val note: String?,
    val targets: List<ProcessActionTarget>,
)

/** Parsing of `sysflow caps`, kept here so the screen can hide actions the device lacks. */
fun processFeatures(caps: CapsData): Set<String> =
    caps.modules["process"]?.features?.toSet() ?: emptySet()

fun parseProcessList(json: JsonObject): ProcessList {
    val foreground = json.obj("foreground")
    return ProcessList(
        available = json.bool("available") ?: false,
        reason = json.string("reason"),
        total = json.int("total") ?: 0,
        returned = json.int("returned") ?: 0,
        truncated = json.bool("truncated") ?: false,
        skipped = json.int("skipped") ?: 0,
        sampleMs = json.long("sample_ms") ?: 0L,
        clockTicks = json.long("clock_ticks") ?: 100L,
        cpusetGroups = json.array("cpuset_groups").mapNotNull { (it as? JsonPrimitive)?.contentOrNull },
        foreground = ProcessForeground(
            available = foreground?.bool("available") ?: false,
            packageName = foreground?.string("package"),
            pid = foreground?.int("pid"),
            user = foreground?.int("user") ?: 0,
            cached = foreground?.bool("cached") ?: false,
            disabled = foreground?.bool("disabled") ?: false,
        ),
        entries = json.array("processes").mapNotNull { (it as? JsonObject)?.let(::parseProcessEntry) },
    )
}

fun parseProcessEntry(json: JsonObject): ProcessEntry = ProcessEntry(
    pid = json.int("pid") ?: 0,
    ppid = json.int("ppid") ?: 0,
    uid = json.int("uid") ?: 0,
    user = json.int("user") ?: 0,
    appId = json.int("app_id") ?: 0,
    packageName = json.string("package"),
    name = json.string("name").orEmpty(),
    cmdline = json.string("cmdline").orEmpty(),
    state = json.string("state").orEmpty(),
    threads = json.int("threads") ?: 0,
    rssKb = json.long("rss_kb"),
    cpuPct = json.double("cpu_pct"),
    cpuTicks = json.long("cpu_ticks"),
    oomScoreAdj = json.int("oom_score_adj"),
    cgroup = json.string("cgroup"),
    cpuset = json.string("cpuset"),
    foreground = json.bool("foreground") ?: false,
    system = json.bool("system") ?: false,
    protected = json.bool("protected") ?: false,
    protectedReason = json.string("protected_reason"),
)

fun parseProcessDetail(json: JsonObject): ProcessDetail = ProcessDetail(
    pid = json.int("pid") ?: 0,
    ppid = json.int("ppid") ?: 0,
    uid = json.int("uid") ?: 0,
    user = json.int("user") ?: 0,
    appId = json.int("app_id") ?: 0,
    packageName = json.string("package"),
    packages = json.array("packages").mapNotNull { (it as? JsonPrimitive)?.contentOrNull },
    name = json.string("name").orEmpty(),
    cmdline = json.array("cmdline").mapNotNull { (it as? JsonPrimitive)?.contentOrNull },
    state = json.string("state").orEmpty(),
    threads = json.int("threads") ?: 0,
    rssKb = json.long("rss_kb"),
    pssKb = json.long("pss_kb"),
    swapPssKb = json.long("swap_pss_kb"),
    cpuPct = json.double("cpu_pct"),
    sampleMs = json.long("sample_ms") ?: 0L,
    oomScoreAdj = json.int("oom_score_adj"),
    nice = json.int("nice") ?: 0,
    rtPriority = json.int("rt_priority") ?: 0,
    policy = json.int("policy") ?: 0,
    cgroup = json.string("cgroup"),
    cpuset = json.string("cpuset"),
    startTimeTicks = json.long("start_time_ticks") ?: 0L,
    foreground = json.bool("foreground") ?: false,
    system = json.bool("system") ?: false,
    protected = json.bool("protected") ?: false,
    protectedReason = json.string("protected_reason"),
)

fun parseProcessAction(json: JsonObject): ProcessActionResult = ProcessActionResult(
    target = json.string("target"),
    adjustment = json.int("adjustment"),
    nice = json.int("nice"),
    group = json.string("group"),
    frozen = json.bool("frozen"),
    mechanism = json.string("mechanism"),
    alreadyStopped = json.bool("already_stopped") ?: false,
    pidsBefore = json.array("pids_before").mapNotNull { (it as? JsonPrimitive)?.intOrNull },
    pidsAfter = json.array("pids_after").mapNotNull { (it as? JsonPrimitive)?.intOrNull },
    verified = json.bool("verified"),
    note = json.string("note"),
    targets = json.array("targets").mapNotNull { element ->
        (element as? JsonObject)?.let { target ->
            ProcessActionTarget(
                pid = target.int("pid") ?: 0,
                before = target.string("before"),
                applied = target.string("applied"),
                verified = target.bool("verified"),
                error = target.string("error"),
                mechanism = target.string("mechanism"),
            )
        }
    },
)

// ---------------------------------------------------------------------------------------------
// Formatting shared by the screen and its tests
// ---------------------------------------------------------------------------------------------

/** 136_684 kB → "133.5 MB"; memory rows only need two significant units. */
fun formatProcessKb(kb: Long?): String {
    if (kb == null) return "—"
    return when {
        kb >= 1024L * 1024L -> String.format(Locale.ROOT, "%.1f GB", kb / (1024.0 * 1024.0))
        kb >= 1024L -> String.format(Locale.ROOT, "%.1f MB", kb / 1024.0)
        else -> "$kb KB"
    }
}

/** Per-core CPU percentage: one decimal, and never a fake `0.0` when the value is unknown. */
fun formatProcessCpu(pct: Double?): String =
    if (pct == null) "—" else String.format(Locale.ROOT, "%.1f%%", pct)

/** The one-line subtitle of a row: UID, threads and OOM score when the device reports them. */
fun processSubtitle(entry: ProcessEntry): String = buildString {
    append("UID ")
    append(entry.uid)
    if (entry.threads > 0) append(" · ${entry.threads} 线程")
    entry.oomScoreAdj?.let { append(" · OOA $it") }
    if (entry.state.isNotBlank() && entry.state != "S") append(" · ${entry.state}")
}

private fun JsonObject.obj(key: String): JsonObject? = this[key] as? JsonObject
private fun JsonObject.array(key: String): JsonArray = this[key] as? JsonArray ?: JsonArray(emptyList())
private fun JsonObject.string(key: String): String? = (this[key] as? JsonPrimitive)?.contentOrNull
private fun JsonObject.int(key: String): Int? = (this[key] as? JsonPrimitive)?.intOrNull
private fun JsonObject.long(key: String): Long? = (this[key] as? JsonPrimitive)?.longOrNull
private fun JsonObject.double(key: String): Double? = (this[key] as? JsonPrimitive)?.doubleOrNull
private fun JsonObject.bool(key: String): Boolean? = (this[key] as? JsonPrimitive)?.booleanOrNull
