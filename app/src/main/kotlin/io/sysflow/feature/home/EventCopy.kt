package io.sysflow.feature.home

import io.sysflow.core.event.SystemEventEntity
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.booleanOrNull
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.doubleOrNull
import kotlin.math.abs

/**
 * Presentation-only mapping for the overview activity feed, aligned with WebUI `event-copy.ts`.
 *
 * The event protocol is open-world: producers may use any module id, category and event type.
 * Nothing here may hide an unknown event — every helper falls back to the producer's own title and
 * message, so a future module still renders even when none of these tables know it.
 */

enum class ModuleTone { CPU, MEMORY, POWER, NEUTRAL }

data class ActivityEntry(
    val event: SystemEventEntity,
    /** Module-free event name, because the card header already names the module. */
    val label: String,
    val detail: String,
    /** How many identical records this row stands for. */
    val count: Int,
)

/** One module section of the overview: only modules that actually produced events appear. */
data class ModuleActivity(
    val module: String,
    val label: String,
    val tone: ModuleTone,
    val entries: List<ActivityEntry>,
    /** Events of this module inside the window that was fetched. */
    val recentCount: Int,
    val latestMs: Long,
)

private val moduleLabels = mapOf(
    "cpu" to "CPU 管理",
    "system" to "系统服务",
    "display" to "显示",
    "battery" to "电池管理",
    "memory" to "内存管理",
    "mem" to "内存管理",
    "storage" to "储存管理",
    "io" to "储存管理",
    "process" to "进程管理",
    "proc" to "进程管理",
    "wakelock" to "唤醒锁",
    "mock" to "模拟模块",
)

private val moduleTones = mapOf(
    "cpu" to ModuleTone.CPU,
    "memory" to ModuleTone.MEMORY,
    "mem" to ModuleTone.MEMORY,
    "storage" to ModuleTone.MEMORY,
    "io" to ModuleTone.MEMORY,
    "display" to ModuleTone.MEMORY,
    "system" to ModuleTone.POWER,
    "battery" to ModuleTone.POWER,
    "wakelock" to ModuleTone.POWER,
)

/** Short Chinese event names, keyed by the stable machine identifier. */
private val eventLabels = mapOf(
    "wakelock_long_held" to "唤醒锁持有过久",
    "mem_pressure_high" to "内存压力偏高",
    "mem_pressure_recovered" to "内存压力已恢复",
    "mem_oom_kill" to "内核内存不足",
    "mem_setting_contested" to "内存设置被反复改回",
    "storage_low_space" to "储存空间不足",
    "storage_space_recovered" to "储存空间已恢复",
    "settings_applied" to "设置已更新",
    "settings_partially_applied" to "设置部分生效",
    "settings_apply_failed" to "设置写入失败",
    "boot_settings_applied" to "开机设置已应用",
    "boot_apply_partial" to "开机设置部分生效",
    "boot_apply_failed" to "开机设置应用失败",
    "boot_apply_crashed" to "开机设置应用失败",
    "module_service_started" to "模块服务已启动",
    "boot_config_updated" to "开机策略已更新",
    "boot_confirmed" to "开机自检通过",
    "boot_reset_failed" to "开机前清理失败",
    "module_reset" to "已恢复默认设置",
    "module_reset_partial" to "部分恢复默认设置",
    "module_reset_failed" to "恢复默认设置失败",
    "stale_settings_dropped" to "残留设置已丢弃",
    "drift_repaired" to "配置漂移已修复",
    "drift_repair_incomplete" to "配置漂移未修复完整",
    "drift_repair_failed" to "配置漂移修复失败",
    "drift_check_failed" to "配置漂移检查失败",
    "enforcement_validation_failed" to "读回校验未通过",
    "daemon_enforcement_failed" to "守护进程执行失败",
    "adaptive_tick_failed" to "自适应采样失败",
    "thermal_protection_transition" to "温控保护变更",
    "battery_saving_transition" to "省电策略变更",
    "psi_guard_transition" to "内存压力护栏变更",
    "screen_off_profile_released" to "熄屏限制已关闭",
    "screen_off_restore_failed" to "熄屏限制关闭失败",
    "charging_started" to "开始充电",
    "charging_stopped" to "停止充电",
    "fully_charged" to "电池已充满",
    "low_battery" to "电量偏低",
    "battery_temp_high" to "电池温度偏高",
    "screen_turned_on" to "屏幕已点亮",
    "screen_turned_off" to "屏幕已熄灭",
    "walt_guard_boot_applied" to "WALT 护栏已恢复",
    "walt_guard_boot_failed" to "WALT 护栏恢复失败",
    "walt_guard_boot_mismatch" to "WALT 护栏未对齐",
    "runtime_state_mismatch" to "运行时状态不一致",
    "daemon_started" to "守护服务已启动",
    "daemon_stopped" to "守护服务已停止",
    "daemon_config_updated" to "守护间隔已更新",
    "safe_mode_entered" to "已进入安全模式",
    "safe_mode_cleared" to "已退出安全模式",
    "corrupt_config_cleared" to "损坏配置已清理",
    "recovery_restored" to "隔离配置已恢复",
    "recovery_discarded" to "隔离配置已丢弃",
    "process_action" to "进程操作",
)

/** Two identical records closer together than this are the same occurrence, reported twice. */
private const val DUPLICATE_WINDOW_MS = 10 * 60 * 1000L

fun moduleLabel(module: String): String =
    moduleLabels[module.lowercase()] ?: module.uppercase()

fun moduleTone(module: String): ModuleTone =
    moduleTones[module.lowercase()] ?: ModuleTone.NEUTRAL

/** The event's own name, falling back to the producer's wording for unknown types. */
fun activityLabel(event: SystemEventEntity): String =
    eventLabels[event.eventType.lowercase()] ?: event.title

/** `CPU 管理 · 设置已更新` — used where a row has no module heading above it. */
fun activityTitle(event: SystemEventEntity): String {
    val label = activityLabel(event)
    return if (label.isBlank()) moduleLabel(event.module) else "${moduleLabel(event.module)} · $label"
}

/** A single Chinese line explaining what happened, or the producer's message when unknown. */
fun activityDetail(event: SystemEventEntity): String {
    val meta = parseMetadata(event.metadataJson)
    val type = event.eventType.lowercase()
    val keys = meta.listPhrase("keys")
    val mismatch = meta.listPhrase("mismatch")
    val drift = meta.listPhrase("drift")
    val cap = meta.number("cap_percent")
    val level = meta.number("level") ?: meta.number("previous_level")
    val persist = (meta["persist"] as? JsonPrimitive)?.booleanOrNull

    return when (type) {
        "settings_applied" -> when {
            keys != null -> {
                val prefix = if (persist == false) "本次生效" else "已持久化"
                "$prefix：$keys"
            }
            persist == false -> "本次生效，重启后失效"
            else -> "已写入并读回校验"
        }
        "settings_partially_applied" -> mismatch?.let { "未对齐：$it" } ?: "部分设置未能生效"
        "settings_apply_failed" -> "写入失败，请查看模块日志"
        "boot_settings_applied" -> keys?.let { "开机后已恢复：$it" } ?: "开机后已自动恢复保存的设置"
        "boot_apply_partial" -> mismatch?.let { "开机后有未对齐项：$it" } ?: "开机应用后仍有未对齐项"
        "boot_apply_failed", "boot_apply_crashed" -> "开机自动应用未完成，请查看模块日志"
        "module_service_started" -> "Root 模块服务已进入开机后阶段"
        "boot_config_updated" -> "开机自动应用策略已更新"
        "boot_confirmed" -> "模块确认本次开机稳定"
        "boot_reset_failed" -> "开机前的清理未完成"
        "module_reset" -> "模块设置已回到默认值"
        "module_reset_partial" -> "部分设置未能回到默认值"
        "module_reset_failed" -> "恢复默认设置失败，请查看模块日志"
        "stale_settings_dropped" -> "无法使用的持久化设置已被丢弃"
        "drift_repaired" -> drift?.let { "已自动修复：$it" } ?: "检测到设置漂移，已自动修复"
        "drift_repair_incomplete" -> drift?.let { "仍有设置未修复：$it" } ?: "仍有设置未修复"
        "drift_repair_failed", "daemon_enforcement_failed" -> "守护进程修复未完成，请查看模块日志"
        "drift_check_failed" -> "本轮漂移检查未完成"
        "enforcement_validation_failed" -> "读回校验未通过，本次未执行"
        "adaptive_tick_failed" -> "自适应采样失败，本轮已跳过"
        "thermal_protection_transition" -> when {
            level != null && level > 0 ->
                cap?.let { "温度升高，已收紧至 ${it.roundPercent()}%" } ?: "温度升高，已触发温控保护"
            else -> "温度回落，温控保护已解除"
        }
        "battery_saving_transition" -> when {
            level != null && level > 0 ->
                cap?.let { "电量偏低，已收紧至 ${it.roundPercent()}%" } ?: "电量偏低，已启用省电限制"
            else -> "省电限制已解除"
        }
        "psi_guard_transition" -> if (level != null && level > 0) {
            "内存压力升高，已启用护栏"
        } else {
            "内存压力回落，护栏已解除"
        }
        "charging_started" -> "电源已接入，开始充电"
        "charging_stopped" -> "电源已断开或充电结束"
        "fully_charged" -> "电池已充至 100%"
        "low_battery" -> (meta.number("battery_percent")?.let { "电量已降至 ${it.roundPercent()}%" }
            ?: "电量偏低，建议尽快充电")
        "battery_temp_high" -> (meta.number("temperature_c")?.let { "电池温度达到 ${it.roundPercent()} °C" }
            ?: "电池温度偏高，建议暂停充电")
        "screen_off_profile_released" -> "熄屏限制已关闭，CPU 恢复正常上限"
        "screen_off_restore_failed" -> "熄屏限制关闭失败，可能仍占用核心"
        "screen_turned_on" -> "显示进入点亮状态"
        "screen_turned_off" -> "显示进入熄灭状态"
        "walt_guard_boot_applied" -> "开机后 WALT 护栏已恢复到原始值"
        "wakelock_long_held" -> if (meta.string("kind") == "userspace_lock") {
            val tag = meta.string("tag") ?: "唤醒锁"
            "「$tag」已被持有超过 30 分钟"
        } else {
            val name = meta.string("name") ?: "未知"
            "唤醒源「$name」持续活跃超过 30 分钟"
        }
        "walt_guard_boot_failed" -> "开机后 WALT 护栏未能写入"
        "walt_guard_boot_mismatch" -> "开机后 WALT 护栏与原始值不一致"
        "runtime_state_mismatch" -> "运行时状态与设置不一致"
        "daemon_started" -> "后台守护进程已开始运行"
        "daemon_stopped" -> "后台守护进程已停止"
        "daemon_config_updated" -> meta.number("interval_s")
            ?.let { "采样间隔已设为 ${it.roundPercent()} 秒" } ?: "守护间隔已更新"
        "safe_mode_entered" -> "调节功能受限，等待人工确认"
        "safe_mode_cleared" -> "已恢复正常调节"
        "corrupt_config_cleared" -> "损坏的配置备份已删除"
        "recovery_restored" -> "隔离的配置已恢复使用"
        "recovery_discarded" -> "隔离的配置已丢弃"
        "storage_low_space" -> meta.number("used_pct")
            ?.let { "存储空间已使用 ${it.roundPercent()}%，请清理空间" }
            ?: "存储空间不足，请清理空间"
        "storage_space_recovered" -> meta.number("used_pct")
            ?.let { "存储空间使用率已回落到 ${it.roundPercent()}%" }
            ?: "存储空间已恢复"
        "mem_pressure_high" -> "内存压力偏高，已持续 15 分钟" + psiDetail(meta)
        "mem_pressure_recovered" -> "内存压力已恢复正常" + psiDetail(meta)
        "mem_oom_kill" -> meta.number("count")?.roundPercent()?.takeIf { it > 0 }
            ?.let { "内核因内存不足杀死了 $it 个进程" }
            ?: "内核因内存不足杀死了进程"
        "mem_setting_contested" -> {
            // The module gave up on this value: something else keeps resetting it, so SysFlow stops
            // fighting instead of rewriting it every few seconds.
            val name = when (meta.string("item")) {
                "mem.swappiness" -> "swappiness"
                "mem.vfs_cache_pressure" -> "vfs_cache_pressure"
                else -> "内存参数"
            }
            val live = meta.number("live")?.roundPercent()?.let { " $it" } ?: ""
            val hours = meta.number("suppress_s")?.div(3600.0)?.roundPercent()?.takeIf { it > 0 } ?: 6L
            "系统反复把 $name 改回$live，已暂停重新应用 $hours 小时"
        }
        "process_action" -> processActionDetail(meta, event.message)
        else -> event.message
    }
}

/**
 * Collapses identical consecutive records into one row that carries a count. Producers re-emit the
 * same observation on every tick, and the overview should not repeat it.
 */
private fun collapseEntries(events: List<SystemEventEntity>): List<ActivityEntry> {
    val collapsed = mutableListOf<ActivityEntry>()
    for (event in events) {
        val previous = collapsed.lastOrNull()
        if (
            previous != null &&
            previous.event.module == event.module &&
            previous.event.eventType == event.eventType &&
            previous.event.title == event.title &&
            previous.event.message == event.message &&
            abs(previous.event.timestampMs - event.timestampMs) <= DUPLICATE_WINDOW_MS
        ) {
            collapsed[collapsed.lastIndex] = previous.copy(count = previous.count + 1)
            continue
        }
        collapsed += ActivityEntry(
            event = event,
            label = activityLabel(event),
            detail = activityDetail(event),
            count = 1,
        )
    }
    return collapsed
}

/**
 * Groups the fetched window into one section per module, newest module first.
 *
 * The module list is derived from the events themselves, never from a registry: a module appears as
 * soon as it emits its first event, and disappears when the local cache no longer holds any of its
 * records. Modules are therefore open-world, exactly like the protocol.
 */
fun buildModuleActivity(
    events: List<SystemEventEntity>,
    perModule: Int = 3,
): List<ModuleActivity> {
    if (events.isEmpty()) return emptyList()
    val previews = LinkedHashMap<String, MutableList<ActivityEntry>>()
    val totals = LinkedHashMap<String, Int>()
    val latest = LinkedHashMap<String, Long>()
    for (entry in collapseEntries(events)) {
        val module = entry.event.module
        totals[module] = (totals[module] ?: 0) + 1
        latest.getOrPut(module) { entry.event.timestampMs }
        val shown = previews.getOrPut(module) { mutableListOf() }
        // Two lines reading exactly the same tell the reader nothing, so only distinct previews show.
        val duplicate = shown.any { it.label == entry.label && it.detail == entry.detail }
        if (!duplicate && shown.size < perModule) shown += entry
    }
    return previews.map { (module, entries) ->
        ModuleActivity(
            module = module,
            label = moduleLabel(module),
            tone = moduleTone(module),
            entries = entries,
            recentCount = totals[module] ?: entries.size,
            latestMs = latest[module] ?: entries.first().event.timestampMs,
        )
    }.sortedByDescending { it.latestMs }
}

private val metadataParser = Json { ignoreUnknownKeys = true; isLenient = true }

private fun parseMetadata(metadataJson: String): JsonObject {
    val trimmed = metadataJson.trim()
    if (trimmed.isEmpty() || trimmed == "{}" || trimmed == "null") return JsonObject(emptyMap())
    return runCatching { metadataParser.parseToJsonElement(trimmed) }.getOrNull() as? JsonObject
        ?: JsonObject(emptyMap())
}

private fun JsonObject.string(key: String): String? =
    (this[key] as? JsonPrimitive)?.contentOrNull?.takeIf { it.isNotBlank() }

private fun JsonObject.number(key: String): Double? =
    (this[key] as? JsonPrimitive)?.doubleOrNull

/** `a、b 等 3 项` — bounded so a long key list cannot overflow the row. */
private fun JsonObject.listPhrase(key: String): String? {
    val array = this[key] as? JsonArray ?: return null
    val items = array.mapNotNull { (it as? JsonPrimitive)?.contentOrNull?.takeIf(String::isNotBlank) }
    if (items.isEmpty()) return null
    return if (items.size <= 2) items.joinToString("、") else "${items[0]}、${items[1]} 等 ${items.size} 项"
}

private fun Double.roundPercent(): Long = kotlin.math.round(this).toLong()

/**
 * "（PSI 3%，可用 4.6 GB）" for the memory-pressure events.
 *
 * `psi_full_pct` arrives in hundredths of a percent (protocol §3.22), so 300 is 3 %.
 */
private fun psiDetail(meta: JsonObject): String {
    val parts = mutableListOf<String>()
    meta.number("psi_full_pct")?.div(100.0)?.roundPercent()?.let { parts.add("PSI $it%") }
    meta.number("available_kb")?.div(1_048_576.0)?.roundPercent()?.let { parts.add("可用 $it GB") }
    meta.number("swap_used_kb")?.div(1_048_576.0)?.roundPercent()?.let { parts.add("swap $it GB") }
    return if (parts.isEmpty()) "" else "（${parts.joinToString("，")}）"
}

private fun processActionDetail(meta: JsonObject, fallback: String): String {
    val action = meta.string("action") ?: return fallback
    val target = meta.string("target") ?: return fallback
    val verified = (meta["verified"] as? JsonPrimitive)?.booleanOrNull
    val suffix = if (verified == false) "（未完全生效）" else ""
    return when (action) {
        "stop" -> "强制停止 $target$suffix"
        "oom" -> meta.number("applied")
            ?.let { "调整 $target 的 oom_score_adj 为 ${it.roundPercent()}$suffix" }
            ?: "调整 $target 的 oom_score_adj$suffix"
        "renice" -> meta.number("applied")
            ?.let { "调整 $target 的 nice 为 ${it.roundPercent()}$suffix" }
            ?: "调整 $target 的 nice$suffix"
        "sched" -> {
            val policy = meta.string("policy") ?: "调度策略"
            val prio = meta.number("priority")?.roundPercent()
            if (prio != null && prio > 0) "设置 $target 为 $policy（优先级 $prio）$suffix"
            else "设置 $target 为 $policy$suffix"
        }
        "cpuset" -> meta.string("group")
            ?.let { "将 $target 移入 cpuset $it$suffix" }
            ?: "调整 $target 的 cpuset$suffix"
        "freeze" -> "冻结 $target$suffix"
        "unfreeze" -> "解冻 $target$suffix"
        else -> fallback
    }
}
