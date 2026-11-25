package io.sysflow.core.event

import androidx.room.Database
import androidx.room.RoomDatabase
import androidx.room.migration.Migration
import androidx.sqlite.db.SupportSQLiteDatabase

@Database(
    entities = [SystemEventEntity::class, EventSyncStateEntity::class],
    version = 2,
    exportSchema = true,
)
abstract class EventDatabase : RoomDatabase() {
    abstract fun systemEventDao(): SystemEventDao

    companion object {
        /** Preserve the v1 event cache while adding lossless dedupe-key storage and backfill state. */
        val MIGRATION_1_2 = object : Migration(1, 2) {
            override fun migrate(db: SupportSQLiteDatabase) {
                db.execSQL("ALTER TABLE system_events ADD COLUMN dedupeKey TEXT")
                db.execSQL(
                    "CREATE TABLE IF NOT EXISTS event_sync_state (" +
                        "id INTEGER NOT NULL, " +
                        "initialBackfillComplete INTEGER NOT NULL, " +
                        "PRIMARY KEY(id))",
                )
                // v1 may have only partially imported the Root journal; resume a complete backfill.
                db.execSQL("INSERT OR REPLACE INTO event_sync_state (id, initialBackfillComplete) VALUES (1, 0)")
            }
        }
    }
}
