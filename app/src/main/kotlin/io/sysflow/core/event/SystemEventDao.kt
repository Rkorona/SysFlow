package io.sysflow.core.event

import androidx.room.Dao
import androidx.room.Insert
import androidx.room.OnConflictStrategy
import androidx.room.Query
import kotlinx.coroutines.flow.Flow

@Dao
interface SystemEventDao {
    @Insert(onConflict = OnConflictStrategy.IGNORE)
    suspend fun insertIgnore(events: List<SystemEventEntity>): List<Long>

    @Query("SELECT * FROM system_events ORDER BY timestampMs DESC, id DESC LIMIT :limit")
    fun observeRecent(limit: Int): Flow<List<SystemEventEntity>>

    @Query("SELECT * FROM system_events WHERE (:module IS NULL OR module = :module) AND (:category IS NULL OR category = :category) AND (:severity IS NULL OR severity = :severity) ORDER BY timestampMs DESC, id DESC LIMIT :limit OFFSET :offset")
    fun observeFiltered(
        module: String?,
        category: String?,
        severity: String?,
        limit: Int,
        offset: Int,
    ): Flow<List<SystemEventEntity>>

    @Query("SELECT * FROM system_events ORDER BY timestampMs DESC, id DESC LIMIT 1")
    suspend fun newest(): SystemEventEntity?

    @Query("SELECT * FROM system_events ORDER BY timestampMs ASC, id ASC LIMIT 1")
    suspend fun oldest(): SystemEventEntity?

    @Query("SELECT COUNT(*) FROM system_events WHERE (:module IS NULL OR module = :module) AND (:category IS NULL OR category = :category) AND (:severity IS NULL OR severity = :severity)")
    fun observeFilteredCount(module: String?, category: String?, severity: String?): Flow<Int>

    @Query("SELECT DISTINCT module FROM system_events ORDER BY module COLLATE NOCASE")
    fun observeModules(): Flow<List<String>>

    @Query("SELECT DISTINCT category FROM system_events ORDER BY category COLLATE NOCASE")
    fun observeCategories(): Flow<List<String>>

    @Query("DELETE FROM system_events WHERE timestampMs < :cutoffMs")
    suspend fun deleteOlderThan(cutoffMs: Long): Int

    @Query("DELETE FROM system_events WHERE id NOT IN (SELECT id FROM system_events ORDER BY timestampMs DESC, id DESC LIMIT :keepCount)")
    suspend fun keepNewest(keepCount: Int): Int

    @Query("DELETE FROM system_events")
    suspend fun clearLocal(): Int

    @androidx.room.Query("SELECT * FROM event_sync_state WHERE id = 1 LIMIT 1")
    suspend fun syncState(): EventSyncStateEntity?

    @androidx.room.Insert(onConflict = androidx.room.OnConflictStrategy.REPLACE)
    suspend fun setSyncState(state: EventSyncStateEntity): Long
}
