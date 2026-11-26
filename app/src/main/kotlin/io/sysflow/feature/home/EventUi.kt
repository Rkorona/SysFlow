package io.sysflow.feature.home

import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.AutoGraph
import androidx.compose.material.icons.rounded.BatteryFull
import androidx.compose.material.icons.rounded.Bolt
import androidx.compose.material.icons.rounded.Dns
import androidx.compose.material.icons.rounded.Lock
import androidx.compose.material.icons.rounded.Memory
import androidx.compose.material.icons.rounded.Storage
import androidx.compose.material.icons.rounded.PowerSettingsNew
import androidx.compose.material.icons.rounded.Shield
import androidx.compose.material.icons.rounded.Thermostat
import androidx.compose.material.icons.rounded.Timeline
import androidx.compose.material.icons.rounded.Tune
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.Composable
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector
import io.sysflow.core.ui.theme.sysFlowColors
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.booleanOrNull
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.doubleOrNull
import kotlinx.serialization.json.longOrNull
import java.util.Locale

/** Shared icon, severity, and metadata presentation helpers aligned with WebUI `event-ui.ts`. */
fun eventCategoryIcon(category: String): ImageVector {
    val normalized = category.lowercase()
    return when {
        "thermal" in normalized || "temperature" in normalized -> Icons.Rounded.Thermostat
        "battery" in normalized -> Icons.Rounded.BatteryFull
        "power" in normalized -> Icons.Rounded.Bolt
        "pressure" in normalized -> Icons.Rounded.Timeline
        "safety" in normalized || "security" in normalized -> Icons.Rounded.Shield
        "config" in normalized -> Icons.Rounded.Tune
        "enforcement" in normalized -> Icons.Rounded.AutoGraph
        "lifecycle" in normalized || "service" in normalized || "daemon" in normalized ->
            Icons.Rounded.PowerSettingsNew
        else -> Icons.Rounded.Timeline
    }
}

/**
 * Module-level glyph for the overview cards. Derived from the event's own module id, so a module
 * that starts emitting events gets a card without any registry edit.
 */
/**
 * Icon container/tint for a module row. Severity still wins on individual event rows, so this is
 * only the module identity colour (teal CPU, blue memory/display/domain, amber system).
 */
@Composable
fun moduleColors(tone: ModuleTone): Pair<Color, Color> {
    val scheme = MaterialTheme.colorScheme
    val extended = MaterialTheme.sysFlowColors
    return when (tone) {
        ModuleTone.CPU ->
            scheme.secondaryContainer.copy(alpha = 0.72f) to scheme.onSecondaryContainer
        ModuleTone.MEMORY -> extended.infoContainer to extended.onInfoContainer
        ModuleTone.POWER -> extended.warningContainer to extended.onWarningContainer
        ModuleTone.NEUTRAL -> scheme.surfaceContainerHighest to scheme.onSurfaceVariant
    }
}

fun moduleIcon(module: String): ImageVector = when (module.lowercase()) {
    "cpu" -> Icons.Rounded.Memory
    "system" -> Icons.Rounded.Shield
    "display" -> Icons.Rounded.Bolt
    "battery" -> Icons.Rounded.BatteryFull
    "memory", "mem" -> Icons.Rounded.Dns
    "storage", "io" -> Icons.Rounded.Storage
    "process", "proc" -> Icons.Rounded.Timeline
    "wakelock" -> Icons.Rounded.Lock
    else -> Icons.Rounded.Timeline
}

fun eventSeverityLabel(severity: String): String = when (severity) {
    "notice" -> "提醒"
    "warning" -> "警告"
    "error" -> "错误"
    "critical" -> "严重"
    else -> "信息"
}

@Composable
fun eventSeverityColors(severity: String, neutralForIconBox: Boolean = false): Pair<Color, Color> {
    val scheme = MaterialTheme.colorScheme
    val extended = MaterialTheme.sysFlowColors
    return when (severity) {
        "critical", "error" -> scheme.errorContainer to scheme.onErrorContainer
        "warning" -> extended.warningContainer to extended.onWarningContainer
        "notice" -> extended.infoContainer to extended.onInfoContainer
        else -> if (neutralForIconBox) {
            scheme.secondaryContainer.copy(alpha = 0.72f) to scheme.onSecondaryContainer
        } else {
            scheme.surfaceContainerHighest to scheme.onSurfaceVariant
        }
    }
}

private val metadataJsonParser = Json { ignoreUnknownKeys = true; isLenient = true }

fun formatEventMetadata(metadataJson: String): String? {
    val trimmed = metadataJson.trim()
    if (trimmed.isEmpty() || trimmed == "{}" || trimmed == "null") return null
    val element = runCatching { metadataJsonParser.parseToJsonElement(trimmed) }.getOrNull()
        ?: return trimmed
    val obj = element as? JsonObject ?: return trimmed.takeIf { it != "[]" }
    if (obj.isEmpty()) return null

    val mismatchList = obj["mismatch"].stringList()
    val hasMismatch = mismatchList.isNotEmpty()
    val parts = mutableListOf<String>()
    val handled = mutableSetOf<String>()

    fun mark(key: String) {
        handled += key
    }

    obj["persist"]?.let { value ->
        mark("persist")
        (value as? JsonPrimitive)?.booleanOrNull?.let { persist ->
            parts += if (persist) "持久化: 是" else "持久化: 否（仅本次生效）"
        }
    }
    obj["keys"]?.let { value ->
        mark("keys")
        val items = value.stringList()
        if (items.isNotEmpty()) parts += "涉及项: ${items.joinToString(", ")}"
    }
    obj["drift"]?.let { value ->
        mark("drift")
        val items = value.stringList()
        if (items.isNotEmpty()) parts += "漂移项: ${items.joinToString(", ")}"
    }
    obj["mismatch"]?.let {
        mark("mismatch")
        if (mismatchList.isNotEmpty()) parts += "未对齐: ${mismatchList.joinToString(", ")}"
    }
    obj["dropped_keys"]?.let { value ->
        mark("dropped_keys")
        val items = value.stringList()
        if (items.isNotEmpty()) parts += "已丢弃: ${items.joinToString(", ")}"
    }
    obj["isolated_modules"]?.let { value ->
        mark("isolated_modules")
        val items = value.stringList()
        if (items.isNotEmpty()) parts += "已隔离模块: ${items.joinToString(", ")}"
    }
    obj["modules"]?.let { value ->
        mark("modules")
        val items = value.stringList()
        if (items.isNotEmpty()) parts += "模块: ${items.joinToString(", ")}"
    }
    obj["temperature_c"]?.let { value ->
        mark("temperature_c")
        (value as? JsonPrimitive)?.longOrNull?.takeIf { it >= 0 }?.let { temp ->
            parts += "温度: ${temp}°C"
        }
    }
    obj["sensor"]?.let { value ->
        mark("sensor")
        (value as? JsonPrimitive)?.contentOrNull?.takeIf { it.isNotBlank() }?.let { sensor ->
            parts += "传感器: $sensor"
        }
    }
    obj["battery_percent"]?.let { value ->
        mark("battery_percent")
        (value as? JsonPrimitive)?.longOrNull?.takeIf { it >= 0 }?.let { cap ->
            parts += "电量: ${cap}%"
        }
    }
    obj["cap_percent"]?.let { value ->
        mark("cap_percent")
        (value as? JsonPrimitive)?.longOrNull?.let { cut ->
            parts += "收紧幅度: ${cut}%"
        }
    }
    obj["psi_avg10"]?.let { value ->
        mark("psi_avg10")
        (value as? JsonPrimitive)?.doubleOrNull?.let { avg ->
            parts += String.format(Locale.US, "PSI avg10: %.2f", avg)
        }
    }
    obj["threshold"]?.let { value ->
        mark("threshold")
        (value as? JsonPrimitive)?.longOrNull?.let { threshold ->
            parts += "阈值: $threshold"
        }
    }
    obj["psi_floor_percent"]?.let { value ->
        mark("psi_floor_percent")
        (value as? JsonPrimitive)?.longOrNull?.takeIf { it > 0 }?.let { floor ->
            parts += "护栏上限: ${floor}%"
        }
    }
    obj["previous_floor_percent"]?.let { value ->
        mark("previous_floor_percent")
        (value as? JsonPrimitive)?.longOrNull?.takeIf { it > 0 }?.let { prev ->
            parts += "原护栏上限: ${prev}%"
        }
    }
    obj["level"]?.let { value ->
        mark("level")
        (value as? JsonPrimitive)?.longOrNull?.let { level ->
            parts += "档位: $level"
        }
    }
    obj["previous_level"]?.let { value ->
        mark("previous_level")
        (value as? JsonPrimitive)?.longOrNull?.let { prev ->
            parts += "原档位: $prev"
        }
    }
    obj["reason"]?.let { value ->
        mark("reason")
        (value as? JsonPrimitive)?.contentOrNull?.takeIf { it.isNotBlank() }?.let { reason ->
            parts += "原因: ${reasonLabel(reason)}"
        }
    }
    obj["error_code"]?.let { value ->
        mark("error_code")
        (value as? JsonPrimitive)?.contentOrNull?.takeIf { it.isNotBlank() }?.let { code ->
            parts += "错误码: $code"
        }
    }
    obj["interval_s"]?.let { value ->
        mark("interval_s")
        (value as? JsonPrimitive)?.longOrNull?.let { sec -> parts += "间隔: ${sec}s" }
    }
    obj["delay_s"]?.let { value ->
        mark("delay_s")
        (value as? JsonPrimitive)?.longOrNull?.let { sec -> parts += "延迟: ${sec}s" }
    }
    obj["boot_count"]?.let { value ->
        mark("boot_count")
        (value as? JsonPrimitive)?.longOrNull?.let { count -> parts += "开机计数: $count" }
    }
    obj["pid"]?.let { value ->
        mark("pid")
        (value as? JsonPrimitive)?.longOrNull?.let { pid -> parts += "PID: $pid" }
    }
    obj["action"]?.let { value ->
        mark("action")
        (value as? JsonPrimitive)?.contentOrNull?.takeIf { it.isNotBlank() }?.let { action ->
            parts += "动作: $action"
        }
    }
    obj["target"]?.let { value ->
        mark("target")
        (value as? JsonPrimitive)?.contentOrNull?.takeIf { it.isNotBlank() }?.let { target ->
            parts += "目标: $target"
        }
    }
    obj["mechanism"]?.let { value ->
        mark("mechanism")
        (value as? JsonPrimitive)?.contentOrNull?.takeIf { it.isNotBlank() }?.let { mechanism ->
            parts += "机制: $mechanism"
        }
    }
    obj["verified"]?.let { value ->
        mark("verified")
        (value as? JsonPrimitive)?.booleanOrNull?.let { verified ->
            parts += if (verified) "读回校验: 已生效" else "读回校验: 未完全生效"
        }
    }

    mark("requested")
    mark("applied")
    if (hasMismatch) {
        obj["requested"]?.takeIf { it !is JsonNull }?.let { parts += "请求: ${compactJsonValue(it)}" }
        obj["applied"]?.takeIf { it !is JsonNull }?.let { parts += "读回: ${compactJsonValue(it)}" }
    }

    obj.forEach { (key, value) ->
        if (key in handled || value is JsonNull) return@forEach
        val formatted = compactJsonValue(value)
        if (formatted.isNotEmpty() && formatted != "{}" && formatted != "[]") {
            parts += "$key: $formatted"
        }
    }

    return parts.takeIf { it.isNotEmpty() }?.joinToString(" · ")
}

private fun reasonLabel(reason: String): String = when (reason) {
    "adaptive_disabled" -> "关闭自适应引擎"
    "module_reset" -> "恢复模块默认设置"
    "screen_off_disabled" -> "关闭熄屏限制"
    "stale_profile_cleanup" -> "清理残留熄屏状态"
    else -> reason
}

private fun JsonElement?.stringList(): List<String> = (this as? JsonArray)
    ?.mapNotNull { (it as? JsonPrimitive)?.contentOrNull?.takeIf(String::isNotBlank) }
    .orEmpty()

private fun compactJsonValue(value: JsonElement): String = when (value) {
    is JsonPrimitive -> if (value.isString) value.content else value.toString()
    is JsonArray -> value.joinToString(", ", prefix = "[", postfix = "]") { compactJsonValue(it) }
    is JsonObject -> value.entries.joinToString(", ", prefix = "{", postfix = "}") { (k, v) ->
        "$k=${compactJsonValue(v)}"
    }
}
