package io.sysflow.core.event

import androidx.room.withTransaction
import io.sysflow.core.model.ModuleSystemEvent
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.serialization.encodeToString
import kotlinx.serialization.json.Json

/**
 * Synchronizes the module-owned durable journal into a Room read model.
 * The APK never creates or infers system events; it only imports and presents module records.
 *
 * The journal is read through [EventSource] so the sync logic can be exercised on the JVM with a
 * fake source instead of requiring root/libsu.
 */
class EventRepository(
    private val database: EventDatabase,
    private val source: EventSource,
) {
    private val dao = database.systemEventDao()
    private val syncMutex = Mutex()
    private val json = Json { ignoreUnknownKeys = true; explicitNulls = false }
    @Volatile private var lastSyncAttemptAtMs = 0L

    fun observeRecent(limit: Int = HOME_EVENT_LIMIT): Flow<List<SystemEventEntity>> =
        dao.observeRecent(limit.coerceIn(1, 100))

    fun observeFiltered(
        module: String? = null,
        category: String? = null,
        severity: String? = null,
        limit: Int = 50,
        offset: Int = 0,
    ): Flow<List<SystemEventEntity>> = dao.observeFiltered(
        module = module,
        category = category,
        severity = severity,
        limit = limit.coerceIn(1, 1000),
        offset = offset.coerceAtLeast(0),
    )

    fun observeFilteredCount(module: String? = null, category: String? = null, severity: String? = null): Flow<Int> =
        dao.observeFilteredCount(module, category, severity)

    fun observeModules(): Flow<List<String>> = dao.observeModules()

    fun observeCategories(): Flow<List<String>> = dao.observeCategories()

    /**
     * Foreground-only incremental pull. A 10-second throttle deliberately decouples event reads
     * from the dashboard's faster resource snapshot refresh.
     */
    suspend fun syncLatest(force: Boolean = false): Int = syncMutex.withLock {
        val now = System.currentTimeMillis()
        if (!force && now - lastSyncAttemptAtMs < SYNC_INTERVAL_MS) return@withLock 0
        // Throttle failed attempts too (for example, an older installed Root module without the
        // event command), otherwise the dashboard's 2-second refresh loop would retry continuously.
        lastSyncAttemptAtMs = now

        var insertedTotal = 0
        // Repair orphaned rows before pulling new ones. A clear from the CLI (or from a terminal)
        // empties the journal while Room keeps its own copy, so the screen would keep listing events
        // whose source no longer exists — which reads as "the delete did nothing".
        repairOrphanedMirror()
        val syncState = dao.syncState()
        if (syncState?.initialBackfillComplete != true || dao.newest() == null) {
            // The completion flag is written only after the full retained journal has been read.
            // If Android kills the APK halfway through first-install backfill, the next run resumes
            // from the oldest record already in Room instead of incorrectly treating it as complete.
            val oldestLocal = dao.oldest()
            var page = if (oldestLocal == null) {
                source.events(limit = SYNC_PAGE_SIZE)
            } else {
                source.events(
                    limit = SYNC_PAGE_SIZE,
                    beforeTimestampMs = oldestLocal.timestampMs,
                    beforeId = oldestLocal.id,
                )
            }
            while (page.events.isNotEmpty()) {
                insertedTotal += insertPage(page.events.map { it.toEntity() }, now)
                if (page.events.size < SYNC_PAGE_SIZE) break
                val oldest = page.events.last()
                page = source.events(
                    limit = SYNC_PAGE_SIZE,
                    beforeTimestampMs = oldest.timestampMs,
                    beforeId = oldest.id,
                )
            }
            dao.setSyncState(EventSyncStateEntity())
        }

        // Pull in chronological order until caught up. This avoids skipping records when more
        // than one page of module events accumulated while the APK was not open.
        val localCursor = dao.newest()
        if (localCursor != null) {
            var afterTimestamp = localCursor.timestampMs
            var afterId = localCursor.id
            while (true) {
                val page = source.events(
                    limit = SYNC_PAGE_SIZE,
                    afterTimestampMs = afterTimestamp,
                    afterId = afterId,
                    ascending = true,
                )
                if (page.events.isEmpty()) break
                insertedTotal += insertPage(page.events.map { it.toEntity() }, now)
                val last = page.events.last()
                val advanced = last.timestampMs > afterTimestamp ||
                    (last.timestampMs == afterTimestamp && last.id > afterId)
                if (!advanced || page.events.size < SYNC_PAGE_SIZE) break
                afterTimestamp = last.timestampMs
                afterId = last.id
            }
        }
        database.withTransaction {
            dao.deleteOlderThan(now - RETENTION_MS)
            dao.keepNewest(LOCAL_MAX_EVENTS)
        }
        insertedTotal
    }

    /** Import one older page from the Root journal, suitable for an event-history screen. */
    suspend fun syncOlderPage(): Int = syncMutex.withLock {
        val oldest = dao.oldest() ?: return@withLock 0
        val page = source.events(
            limit = SYNC_PAGE_SIZE,
            beforeTimestampMs = oldest.timestampMs,
            beforeId = oldest.id,
        )
        if (page.events.isEmpty()) return@withLock 0
        database.withTransaction {
            val inserted = dao.insertIgnore(page.events.map { it.toEntity() }).count { it != -1L }
            dao.deleteOlderThan(System.currentTimeMillis() - RETENTION_MS)
            dao.keepNewest(LOCAL_MAX_EVENTS)
            inserted
        }
    }

    /** Clears only the APK's cache; module-side source-of-truth records are never deleted here. */
    suspend fun clearLocalCache(): Int = dao.clearLocal()

    /**
     * Drop mirrored rows when the journal no longer backs them.
     *
     * The journal is the source of truth and Room is only a read model of it, so a record the journal
     * no longer holds must not survive locally. Row-by-row reconciliation is unnecessary: the journal
     * is ordered, so if its oldest surviving record is newer than the newest mirrored row, no mirrored
     * row can still be confirmed and the whole mirror goes. An empty journal empties the mirror.
     *
     * A failed probe is *not* treated as an empty journal. An older module binary without
     * `event list` would otherwise wipe the cache on every sync.
     */
    private suspend fun repairOrphanedMirror() {
        val newest = dao.newest() ?: return
        val page = runCatching { source.events(limit = 1, ascending = true) }.getOrNull() ?: return
        val journalOldest = page.events.firstOrNull()
        if (mirrorIsOrphaned(journalOldest?.timestampMs, newest.timestampMs)) {
            dao.clearLocal()
        }
    }

    private suspend fun insertPage(events: List<SystemEventEntity>, now: Long): Int {
        if (events.isEmpty()) return 0
        return database.withTransaction {
            val inserted = dao.insertIgnore(events).count { it != -1L }
            dao.deleteOlderThan(now - RETENTION_MS)
            dao.keepNewest(LOCAL_MAX_EVENTS)
            inserted
        }
    }

    private fun ModuleSystemEvent.toEntity() = SystemEventEntity(
        id = id,
        module = module,
        category = category,
        eventType = eventType,
        severity = severity,
        title = title,
        message = message,
        timestampMs = timestampMs,
        metadataJson = json.encodeToString(metadata),
        dedupeKey = dedupeKey,
    )

    companion object {
        const val HOME_EVENT_LIMIT = 100
        const val LOCAL_MAX_EVENTS = 5_000
        const val SYNC_PAGE_SIZE = 500
        const val SYNC_INTERVAL_MS = 10_000L
        const val RETENTION_MS = 30L * 24L * 60L * 60L * 1_000L
    }
}

/**
 * Whether the mirrored rows can no longer be backed by the journal.
 *
 * [journalOldestMs] is the timestamp of the journal's oldest surviving record, or null when the
 * journal is empty. The mirror is orphaned when the journal holds nothing, or when its oldest record
 * is already newer than the newest mirrored row: at that point the journal has been cleared (or has
 * rolled past the mirror), so no mirrored record has a source any more.
 *
 * Kept as a pure function because the decision is the whole bug: it used to be "sync more records",
 * which cannot express "the source forgot these".
 */
internal fun mirrorIsOrphaned(journalOldestMs: Long?, mirrorNewestMs: Long): Boolean =
    journalOldestMs == null || journalOldestMs > mirrorNewestMs
