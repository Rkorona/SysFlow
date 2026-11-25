package io.sysflow.core.event

import androidx.room.Entity
import androidx.room.Index
import androidx.room.PrimaryKey

/** Local Room mirror of the Root-owned event journal. The module is the source of truth. */
@Entity(
    tableName = "system_events",
    indices = [
        Index(value = ["timestampMs"]),
        Index(value = ["module", "timestampMs"]),
        Index(value = ["category", "timestampMs"]),
        Index(value = ["severity", "timestampMs"]),
    ],
)
data class SystemEventEntity(
    @PrimaryKey val id: String,
    val module: String,
    val category: String,
    val eventType: String,
    val severity: String,
    val title: String,
    val message: String,
    val timestampMs: Long,
    /** Raw JSON metadata keeps the schema forward-compatible with future module payloads. */
    val metadataJson: String,
    val dedupeKey: String?,
)
