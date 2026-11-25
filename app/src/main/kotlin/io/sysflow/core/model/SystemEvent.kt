package io.sysflow.core.model

import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonNull

/** Event payload authored and persisted by the Root module. The APK treats module/category/type
 * as open strings so new modules can add events without an APK release. */
@Serializable
data class ModuleSystemEvent(
    val id: String,
    val module: String,
    val category: String,
    @SerialName("event_type") val eventType: String,
    val severity: String,
    val title: String,
    val message: String = "",
    @SerialName("timestamp_ms") val timestampMs: Long,
    val metadata: JsonElement = JsonNull,
    @SerialName("dedupe_key") val dedupeKey: String? = null,
)

@Serializable
data class ModuleEventPage(
    val events: List<ModuleSystemEvent> = emptyList(),
    @SerialName("next_cursor_ms") val nextCursorMs: Long? = null,
    @SerialName("next_cursor_id") val nextCursorId: String? = null,
    /** Journal directory the CLI read, as it resolved it. Used by the events diagnostics panel. */
    val directory: String? = null,
)
