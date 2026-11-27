package io.sysflow.core.model

import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.booleanOrNull
import kotlinx.serialization.json.doubleOrNull
import kotlinx.serialization.json.intOrNull
import kotlinx.serialization.json.longOrNull

/**
 * Read a JSON string, treating `null` as absent.
 *
 * `JsonNull` is itself a `JsonPrimitive`, so the naive `(value as? JsonPrimitive)?.content` turns an
 * explicit JSON `null` into the string `"null"` — which then renders as a literal "null" in the UI.
 */
private fun JsonElement?.textOrNull(): String? {
    val primitive = this as? JsonPrimitive ?: return null
    if (primitive is JsonNull) return null
    return primitive.content.takeIf { it.isNotBlank() }
}

/** Protocol item id of the per-app AppOps switch (protocol §3.18). */
const val WAKELOCK_BLOCKED_ITEM = "wakelock.blocked"

/** Protocol item id of the user whitelist: `true` = never block this app (protocol §3.18). */
const val WAKELOCK_EXEMPT_ITEM = "wakelock.exempt"

/**
 * Wake-lock telemetry from `snapshot.modules.wakelock` (protocol §3.18).
 *
 * The kernel half is read-only telemetry; the AppOps half (`wakelock.blocked`) is a separate,
 * opt-in read because it costs a binder call.
 */
data class WakelockSource(
    val name: String,
    /** Coarse driver family from the module (`network`, `bluetooth`, …); see protocol §3.18. */
    val category: String,
    val active: Boolean,
    val totalMs: Long,
    val maxMs: Long,
    val wakeupCount: Int,
    val eventCount: Int,
    val activeSinceMs: Long,
    val preventSuspendMs: Long,
)

/** One entry of the rolling 24 h ranking. [share] is relative to measured wake time, not wall clock. */
data class WakelockRankEntry(
    val name: String,
    val category: String,
    val awakeMs: Long,
    val share: Double,
    /** Increments in the same window; `0` when the kernel table has no wakeup counter. */
    val wakeups: Long = 0,
)

data class WakelockSummary(
    val available: Boolean,
    val known: Int,
    val active: Int,
    val windowSeconds: Long,
    val updatedSeconds: Long,
    val rankingAvailable: Boolean,
    /**
     * Why the window started: `fresh`, `reboot`, `rollover` or `baseline` (protocol §3.18).
     *
     * After a reboot the counters restart, so the window restarts with them. Without this the
     * empty ranking used to be explained as "the device was idle", which was simply untrue.
     */
    val windowReason: String? = null,
    /** Seconds since boot, when the module could read `/proc/uptime`; `0` when unknown. */
    val sinceBootSeconds: Long = 0,
    /** Total measured wake time, i.e. the denominator behind every `share`. */
    val measuredMs: Long = 0,
    /** Total wakeup count over the same window; `0` when the module cannot count them. */
    val totalWakeups: Long = 0,
    /** Why the kernel table is unreadable, when it is; `null` when it was read fine. */
    val reason: String? = null,
)

data class WakelockState(
    val sources: List<WakelockSource>,
    val ranked: List<WakelockRankEntry>,
    val summary: WakelockSummary,
    /** Packages the module refuses to block (`wakelock.protected`); empty for older modules. */
    val protectedPackages: List<String> = emptyList(),
    /**
     * Whether the module actually sent `category` fields.
     *
     * Older module builds predate driver classification, so every row would fall back to "其它驱动"
     * and the page would look broken for a reason the user cannot see. Tracking the field's presence
     * lets the UI say "update the module" instead of silently showing one bucket.
     */
    val categorySupported: Boolean = false,
) {
    companion object {
        fun from(value: JsonObject?): WakelockState? {
            val obj = value ?: return null
            val sources = (obj["wakelock.kernel_sources"] as? JsonArray)
                ?.mapNotNull { element ->
                    val item = element as? JsonObject ?: return@mapNotNull null
                    val name = (item["name"] as? JsonPrimitive)?.content ?: return@mapNotNull null
                    WakelockSource(
                        name = name,
                        category = item["category"].textOrNull() ?: "other",
                        active = (item["active"] as? JsonPrimitive)?.booleanOrNull == true,
                        totalMs = (item["total_ms"] as? JsonPrimitive)?.longOrNull ?: 0L,
                        maxMs = (item["max_ms"] as? JsonPrimitive)?.longOrNull ?: 0L,
                        wakeupCount = (item["wakeup_count"] as? JsonPrimitive)?.intOrNull ?: 0,
                        eventCount = (item["event_count"] as? JsonPrimitive)?.intOrNull ?: 0,
                        activeSinceMs = (item["active_since_ms"] as? JsonPrimitive)?.longOrNull ?: 0L,
                        preventSuspendMs = (item["prevent_suspend_ms"] as? JsonPrimitive)?.longOrNull ?: 0L,
                    )
                }
                .orEmpty()

            val ranked = (obj["wakelock.ranked"] as? JsonArray)
                ?.mapNotNull { element ->
                    val item = element as? JsonObject ?: return@mapNotNull null
                    val name = (item["name"] as? JsonPrimitive)?.content ?: return@mapNotNull null
                    WakelockRankEntry(
                        name = name,
                        category = item["category"].textOrNull() ?: "other",
                        awakeMs = (item["awake_ms"] as? JsonPrimitive)?.longOrNull ?: 0L,
                        wakeups = (item["wakeups"] as? JsonPrimitive)?.longOrNull ?: 0L,
                        share = (item["share"] as? JsonPrimitive)?.doubleOrNull ?: 0.0,
                    )
                }
                .orEmpty()

            val summary = (obj["wakelock.summary"] as? JsonObject)?.let { s ->
                WakelockSummary(
                    available = (s["available"] as? JsonPrimitive)?.booleanOrNull == true,
                    known = (s["known"] as? JsonPrimitive)?.intOrNull ?: 0,
                    active = (s["active"] as? JsonPrimitive)?.intOrNull ?: 0,
                    windowSeconds = (s["window_s"] as? JsonPrimitive)?.longOrNull ?: 0L,
                    updatedSeconds = (s["updated_s"] as? JsonPrimitive)?.longOrNull ?: 0L,
                    rankingAvailable = (s["ranking_available"] as? JsonPrimitive)?.booleanOrNull == true,
                    windowReason = s["window_reason"].textOrNull(),
                    sinceBootSeconds = (s["since_boot_s"] as? JsonPrimitive)?.longOrNull ?: 0L,
                    measuredMs = (s["measured_ms"] as? JsonPrimitive)?.longOrNull ?: 0L,
                    totalWakeups = (s["total_wakeups"] as? JsonPrimitive)?.longOrNull ?: 0L,
                    reason = s["reason"].textOrNull(),
                )
            } ?: return null

            val rawSources = obj["wakelock.kernel_sources"] as? JsonArray
            val rawRanked = obj["wakelock.ranked"] as? JsonArray
            val categorySupported = (rawSources.orEmpty().any { it is JsonObject && it.containsKey("category") }) ||
                (rawRanked.orEmpty().any { it is JsonObject && it.containsKey("category") })

            val protectedPackages = (obj["wakelock.protected"] as? JsonArray)
                ?.mapNotNull { it.textOrNull() }
                .orEmpty()

            return WakelockState(
                sources = sources,
                ranked = ranked,
                summary = summary,
                protectedPackages = protectedPackages,
                categorySupported = categorySupported,
            )
        }
    }
}

/** One userspace lock currently held (`sysflow wakelock held`, protocol §3.19). */
data class WakelockHeldLock(
    val kind: String,
    val tag: String,
    val heldMs: Long,
    val uid: Int?,
    val pid: Int?,
    /** Actionable package; `null` when the dump only names a UID (Samsung layout, system UIDs). */
    val packageName: String?,
    /** Display label resolved from the UID by the module (`Android 系统`, `UID 10390`, …). */
    val owner: String? = null,
)

data class WakelockHeld(val count: Int, val locks: List<WakelockHeldLock>, val cached: Boolean) {
    companion object {
        fun from(value: JsonObject?): WakelockHeld? {
            val obj = value ?: return null
            val locks = (obj["held"] as? JsonArray)
                ?.mapNotNull { element ->
                    val item = element as? JsonObject ?: return@mapNotNull null
                    WakelockHeldLock(
                        kind = (item["kind"] as? JsonPrimitive)?.content ?: "WAKE_LOCK",
                        tag = (item["tag"] as? JsonPrimitive)?.content ?: "?",
                        heldMs = (item["held_ms"] as? JsonPrimitive)?.longOrNull ?: 0L,
                        uid = (item["uid"] as? JsonPrimitive)?.intOrNull,
                        pid = (item["pid"] as? JsonPrimitive)?.intOrNull,
                        // `textOrNull` because a JSON `null` is a JsonPrimitive whose content is
                        // "null" — that used to become a package literally named "null".
                        packageName = item["package"].textOrNull(),
                        owner = item["owner"].textOrNull(),
                    )
                }
                .orEmpty()
            if ((obj["available"] as? JsonPrimitive)?.booleanOrNull == false) return null
            return WakelockHeld(
                count = (obj["count"] as? JsonPrimitive)?.intOrNull ?: locks.size,
                locks = locks,
                cached = (obj["cached"] as? JsonPrimitive)?.booleanOrNull == true,
            )
        }
    }
}

/** One wake-lock tag held by an app, from `wakelock apps` (protocol §3.20). */
data class WakelockAppTag(val tag: String, val totalMs: Long, val count: Int)

/** Per-UID wake-lock time since the last charge. */
data class WakelockAppStat(
    val uid: String,
    val numericUid: Int?,
    val name: String,
    val packages: List<String>,
    val totalMs: Long,
    val count: Int,
    val share: Double,
    val tags: List<WakelockAppTag>,
)

data class WakelockApps(
    val totalMs: Long,
    val entries: List<WakelockAppStat>,
    val cached: Boolean,
) {
    companion object {
        fun from(value: JsonObject?): WakelockApps? {
            val obj = value ?: return null
            if ((obj["available"] as? JsonPrimitive)?.booleanOrNull == false) return null
            val entries = (obj["entries"] as? JsonArray)
                ?.mapNotNull { element ->
                    val item = element as? JsonObject ?: return@mapNotNull null
                    val name = (item["name"] as? JsonPrimitive)?.content ?: return@mapNotNull null
                    WakelockAppStat(
                        uid = (item["uid"] as? JsonPrimitive)?.content ?: "",
                        numericUid = (item["numeric_uid"] as? JsonPrimitive)?.intOrNull,
                        name = name,
                        packages = (item["packages"] as? JsonArray)
                            ?.mapNotNull { (it as? JsonPrimitive)?.content }
                            .orEmpty(),
                        totalMs = (item["total_ms"] as? JsonPrimitive)?.longOrNull ?: 0L,
                        count = (item["count"] as? JsonPrimitive)?.intOrNull ?: 0,
                        share = (item["share"] as? JsonPrimitive)?.doubleOrNull ?: 0.0,
                        tags = (item["tags"] as? JsonArray)
                            ?.mapNotNull { tagElement ->
                                val tag = tagElement as? JsonObject ?: return@mapNotNull null
                                WakelockAppTag(
                                    tag = (tag["tag"] as? JsonPrimitive)?.content ?: "",
                                    totalMs = (tag["total_ms"] as? JsonPrimitive)?.longOrNull ?: 0L,
                                    count = (tag["count"] as? JsonPrimitive)?.intOrNull ?: 0,
                                )
                            }
                            .orEmpty(),
                    )
                }
                .orEmpty()
            return WakelockApps(
                totalMs = (obj["total_ms"] as? JsonPrimitive)?.longOrNull ?: 0L,
                entries = entries,
                cached = (obj["cached"] as? JsonPrimitive)?.booleanOrNull == true,
            )
        }
    }
}

/** One bucket of the recorded wake-lock trend. */
data class WakelockTrendPoint(
    val ts: Long,
    /** Longest source in that bucket (ms): "how awake" the device was. */
    val topMs: Long,
    /** Wakeups counted in that bucket: "how often" it woke. */
    val wakeups: Long,
)

/**
 * `sysflow history wakelock` decoded (protocol §3.2).
 *
 * The daemon samples every five minutes, so the series is sparse by design; the two metrics come
 * from the same samples and are joined by timestamp.
 */
data class WakelockHistory(
    val points: List<WakelockTrendPoint>,
    val daemonRunning: Boolean,
) {
    companion object {
        fun from(value: JsonObject?): WakelockHistory? {
            val obj = value ?: return null
            val series = obj["series"] as? JsonObject ?: return null
            fun metric(name: String): Map<Long, Long> =
                (series[name] as? JsonArray)
                    ?.mapNotNull { row ->
                        val pair = row as? JsonArray ?: return@mapNotNull null
                        val ts = (pair.getOrNull(0) as? JsonPrimitive)?.longOrNull
                            ?: return@mapNotNull null
                        val value = (pair.getOrNull(1) as? JsonPrimitive)?.longOrNull ?: 0L
                        ts to value
                    }
                    ?.toMap()
                    .orEmpty()

            val topMs = metric("top_ms")
            val wakeups = metric("wakeups")
            val points = (topMs.keys + wakeups.keys)
                .sorted()
                .map { ts -> WakelockTrendPoint(ts, topMs[ts] ?: 0L, wakeups[ts] ?: 0L) }
            return WakelockHistory(
                points = points,
                daemonRunning = (obj["daemon_running"] as? JsonPrimitive)?.booleanOrNull == true,
            )
        }
    }
}

/**
 * The last wakeup reason (`sysflow wakelock wakeup`, protocol §3.21).
 *
 * [available] is false when this ROM's `dumpsys power` does not print the field at all — that is a
 * different answer from "nothing woke the device", and [note] carries the explanation.
 */
data class WakelockWakeup(
    val available: Boolean,
    val reason: String?,
    val ageMs: Long?,
    val note: String?,
    val cached: Boolean,
) {
    companion object {
        fun from(value: JsonObject?): WakelockWakeup? {
            val obj = value ?: return null
            return WakelockWakeup(
                available = (obj["available"] as? JsonPrimitive)?.booleanOrNull == true,
                reason = obj["reason"].textOrNull(),
                ageMs = (obj["age_ms"] as? JsonPrimitive)?.longOrNull,
                note = obj["note"].textOrNull(),
                cached = (obj["cached"] as? JsonPrimitive)?.booleanOrNull == true,
            )
        }
    }
}

/**
 * One row of the wake-lock page's 应用 table.
 *
 * It merges two independent sources: BatteryStats' wake-lock time (how long the app held locks since
 * the last charge) and the live AppOps state (whether its locks are blocked). A blocked app that held
 * nothing this cycle has [totalMs] `null` and must still appear, otherwise its rule becomes
 * unreachable from the UI.
 */
data class WakelockAppRow(
    val key: String,
    val title: String,
    val packageName: String?,
    val totalMs: Long?,
    val caption: String?,
    val blocked: Boolean,
    val protected: Boolean,
    /** On the user whitelist: never blocked, and shown with an "已保护" pill. */
    val exempt: Boolean = false,
)

/**
 * Merge the BatteryStats ranking with the set of blocked packages.
 *
 * Ranking rows come first (by time, then name); blocked packages missing from the ranking follow with
 * no time. System UIDs without a package cannot be blocked and are not part of the blocked set, so
 * they only ever appear from the ranking.
 */
fun WakelockApps?.mergeWithRules(
    blocked: Set<String>,
    protectedPackages: Set<String> = emptySet(),
    exemptPackages: Set<String> = emptySet(),
): List<WakelockAppRow> {
    val rows = LinkedHashMap<String, WakelockAppRow>()
    this?.entries.orEmpty().forEach { entry ->
        val packageName = entry.packages.firstOrNull()
        val key = packageName ?: entry.uid
        rows[key] = WakelockAppRow(
            key = key,
            title = entry.name,
            packageName = packageName,
            totalMs = entry.totalMs,
            caption = entry.tags.firstOrNull()?.tag?.take(72) ?: packageName,
            blocked = packageName != null && packageName in blocked,
            protected = packageName != null && isProtectedPackage(packageName, protectedPackages),
            exempt = packageName != null && packageName in exemptPackages,
        )
    }
    blocked.sorted().forEach { packageName ->
        if (rows.containsKey(packageName)) return@forEach
        rows[packageName] = WakelockAppRow(
            key = packageName,
            title = packageName,
            packageName = packageName,
            totalMs = null,
            caption = "本次统计内没有持锁记录",
            blocked = true,
            protected = isProtectedPackage(packageName, protectedPackages),
        )
    }
    exemptPackages.sorted().forEach { packageName ->
        if (rows.containsKey(packageName)) return@forEach
        rows[packageName] = WakelockAppRow(
            key = packageName,
            title = packageName,
            packageName = packageName,
            totalMs = null,
            caption = "已保护：不会拦截其唤醒锁",
            blocked = false,
            protected = isProtectedPackage(packageName, protectedPackages),
            exempt = true,
        )
    }
    return rows.values.sortedWith(
        compareByDescending<WakelockAppRow> { it.totalMs ?: -1L }.thenBy { it.title.lowercase() },
    )
}

/**
 * Whether the module refuses to block this package.
 *
 * The module publishes the policy as `wakelock.protected` (protocol §3.18); the local list is only
 * a fallback for modules that predate that item, so no third copy of the rule can drift. An empty
 * server list means "the module did not tell us", never "nothing is protected".
 */
fun isProtectedPackage(packageName: String, published: Set<String> = emptySet()): Boolean =
    if (published.isEmpty()) {
        WakelockProtection.isProtected(packageName)
    } else {
        packageName in published
    }

/**
 * Apps the module refuses to block (protocol §3.18), kept as a fallback.
 *
 * Mirrored from `core/sysflow-modules/src/wakelock/policy.rs` so an older module — one that does not
 * send `wakelock.protected` — still renders a disabled switch instead of letting the user tap it
 * and receive an `invalid_arg`. The module remains the authority: this list only avoids a pointless
 * round trip.
 */
object WakelockProtection {
    private val PACKAGES = setOf(
        "android",
        "com.android.systemui",
        "com.android.phone",
        "com.android.server.telecom",
        "com.android.dialer",
        "com.google.android.dialer",
        "com.android.mms",
        "com.android.messaging",
        "com.android.providers.telephony",
        "com.android.settings",
        "io.sysflow",
    )

    fun isProtected(packageName: String): Boolean = packageName in PACKAGES
}

/**
 * Chinese label for a wakeup-source category (protocol §3.18 `category`).
 *
 * Unknown identifiers fall through to "其它驱动" instead of being shown raw: a client that is older
 * than the module must still render something readable.
 */
fun wakeCategoryLabel(category: String): String = when (category) {
    "system" -> "系统"
    "bluetooth" -> "蓝牙"
    "nfc" -> "NFC"
    "network" -> "网络"
    "timer" -> "定时器与闹钟"
    "touch" -> "触摸与手写笔"
    "display" -> "显示"
    "power" -> "充电与电池"
    "audio" -> "音频"
    "camera" -> "相机"
    "sensor" -> "传感器"
    "storage" -> "存储"
    "usb" -> "USB"
    "accessory" -> "配件（保护盖等）"
    "subsystem" -> "子系统通信"
    "serial" -> "串口与低速总线"
    else -> "其它驱动"
}

/** A category with its rows, ordered by total time descending. */
data class WakeGroup<T>(
    val category: String,
    val label: String,
    val totalMs: Long,
    val rows: List<T>,
)

private fun <T> groupByCategory(
    rows: List<T>,
    categoryOf: (T) -> String,
    timeOf: (T) -> Long,
): List<WakeGroup<T>> = rows
    .groupBy(categoryOf)
    .map { (category, group) ->
        WakeGroup(
            category = category,
            label = wakeCategoryLabel(category),
            totalMs = group.sumOf(timeOf),
            rows = group.sortedByDescending(timeOf),
        )
    }
    .sortedByDescending { it.totalMs }

/**
 * Group the 24 h ranking by driver family.
 *
 * The two list types get different names because Kotlin/JVM erases the generic receiver: two
 * `List<T>.groupByCategory()` extensions would clash on the same JVM signature.
 */
fun List<WakelockRankEntry>.rankingGroups(
    byCount: Boolean = false,
): List<WakeGroup<WakelockRankEntry>> =
    groupByCategory(this, { it.category }, { if (byCount) it.wakeups else it.awakeMs })

/** Group the cumulative source list by driver family. */
fun List<WakelockSource>.sourceGroups(): List<WakeGroup<WakelockSource>> =
    groupByCategory(this, { it.category }, { it.totalMs })

/**
 * Relative age of a sampling timestamp, e.g. "刚刚" / "3 分钟前".
 *
 * The wakeup ranking is empty until two samples exist, so the page has to show *when* the last one
 * happened — otherwise "no data" is indistinguishable from "the daemon is not running".
 */
fun formatAgeSeconds(ageSeconds: Long): String = when {
    ageSeconds < 60 -> "刚刚"
    ageSeconds < 3600 -> "${ageSeconds / 60} 分钟前"
    ageSeconds < 86_400 -> "${ageSeconds / 3600} 小时前"
    else -> "${ageSeconds / 86_400} 天前"
}

/**
 * Why the 24 h ranking has no rows yet.
 *
 * Four distinct cases, because the fix differs: still collecting (wait), window too young (wait a
 * little), a fresh reboot (the window legitimately restarted) or genuinely idle (nothing to wait
 * for). Reporting a reboot as "idle" was wrong for up to a day.
 */
fun wakeRankHint(summary: WakelockSummary, sampleIntervalSeconds: Long = 300): String {
    if (!summary.available) return "内核唤醒源不可读，暂时无法生成排行。"
    val rebooted = summary.windowReason == "reboot"
    val window = formatAwakeMs(summary.windowSeconds * 1000)
    return when {
        !summary.rankingAvailable ->
            "正在收集：守护进程每 5 分钟采样一次，第二次采样后这里会出现排行。"
        summary.windowSeconds < sampleIntervalSeconds && rebooted ->
            "设备刚重启，统计窗口从重启时刻重新开始，下一次采样后出现排行。"
        summary.windowSeconds < sampleIntervalSeconds ->
            "正在收集：统计窗口才建立 $window，下一次采样（5 分钟一次）后出现增量。"
        rebooted ->
            "自重启以来（$window）没有新增唤醒时间——设备在这段时间里基本处于空闲。"
        else -> "窗口内没有新增唤醒时间——设备在这段时间里基本处于空闲。"
    }
}

/** `90_000` -> `1m30s`, `5_400_000` -> `1h30m`. Used for held times and source totals. */
fun formatAwakeMs(ms: Long): String {
    if (ms <= 0L) return "0s"
    val totalSeconds = ms / 1000
    val days = totalSeconds / 86_400
    val hours = (totalSeconds % 86_400) / 3_600
    val minutes = (totalSeconds % 3_600) / 60
    val seconds = totalSeconds % 60
    return when {
        days > 0 -> "${days}d${hours}h"
        hours > 0 -> "${hours}h${minutes}m"
        minutes > 0 -> "${minutes}m${seconds}s"
        else -> "${seconds}s"
    }
}

/**
 * Packages the module reports as blocked after an `apply` (protocol §3.18).
 *
 * `applied` is keyed by **item id**, not by package: `{"wakelock.blocked": {"com.x": true}}`.
 * Reading it as `applied["com.x"]` always yields `null`, which an earlier version hid behind
 * `?: blocked` — that turned every silent ROM rejection into a success message. A flattened
 * payload (`{"com.x": true}`) is accepted too, so a future module shape keeps working.
 *
 * A package missing from the result is *unknown*, not "unchanged": callers must not treat it as
 * success.
 */
fun appliedBlocked(result: ApplyResult): Map<String, Boolean> {
    val nested = result.applied[WAKELOCK_BLOCKED_ITEM] as? JsonObject
    val source = nested ?: result.applied
    return source.mapNotNull { (packageName, value) ->
        val flag = (value as? JsonPrimitive)?.booleanOrNull ?: return@mapNotNull null
        packageName to flag
    }.toMap()
}
