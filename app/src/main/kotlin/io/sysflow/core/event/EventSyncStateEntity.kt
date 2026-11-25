package io.sysflow.core.event

import androidx.room.Entity
import androidx.room.PrimaryKey

/** Durable marker so a process death during first-install backfill can resume safely. */
@Entity(tableName = "event_sync_state")
data class EventSyncStateEntity(
    @PrimaryKey val id: Int = 1,
    val initialBackfillComplete: Boolean = true,
)
