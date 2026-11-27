package io.sysflow.core.event

import android.app.Application
import android.content.Context
import androidx.room.Room
import androidx.room.withTransaction
import androidx.test.core.app.ApplicationProvider
import io.sysflow.core.model.ModuleEventPage
import io.sysflow.core.model.ModuleSystemEvent
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.test.runTest
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonObject
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config

/**
 * Room sync behaviour of [EventRepository] against a fake journal: first-install backfill (multi-page
 * and resumable), incremental catch-up, orphan repair, throttling, "load older" paging, and the
 * retention / row-cap policy.
 *
 * The fake source implements the real `event list` cursor semantics — (timestamp, id) tuple
 * comparisons with `ascending` ordering — so the assertions are about behaviour a device would see,
 * not about the repository's internal calls. A `SysFlowClientEventSource` smoke test would need root
 * and is out of scope here; the adapter itself is a one-line delegation.
 */
@RunWith(RobolectricTestRunner::class)
// A blank Application: the manifest points at SysFlowApplication, whose onCreate() calls startKoin().
// Robolectric instantiates the application once per test method, so Koin would already be started and
// every test after the first would fail with KoinApplicationAlreadyStartedException. These tests own
// their in-memory Room database and need no DI, so the app class is deliberately not instantiated.
@Config(sdk = [37], application = Application::class)
class EventRepositoryTest {

    private lateinit var db: EventDatabase
    private lateinit var dao: SystemEventDao

    // Timestamps must stay inside the 30-day retention window, otherwise every sync deletes the
    // rows it just imported and the assertions would pass for the wrong reason.
    private val base = System.currentTimeMillis() - 60L * 60L * 1_000L

    @Before
    fun setUp() {
        val context = ApplicationProvider.getApplicationContext<Context>()
        db = Room.inMemoryDatabaseBuilder(context, EventDatabase::class.java)
            .allowMainThreadQueries()
            .build()
        dao = db.systemEventDao()
    }

    @After
    fun tearDown() {
        db.close()
    }

    // ---- first install: multi-page backfill ---------------------------------

    @Test
    fun firstBackfillImportsEveryPageOnce() = runTest {
        val source = FakeEventSource((0 until 1_200).map { event(it, base + it) })
        val repository = EventRepository(db, source)

        val inserted = repository.syncLatest(force = true)

        assertEquals(1_200, inserted)
        assertEquals(1_200, count())
        assertEquals(1_200, allIds().distinct().size)
        // Three journal pages (500 + 500 + 200), so the before-cursor was used twice.
        assertTrue(
            "expected multi-page paging, got ${source.calls}",
            source.calls.count { it.beforeTimestampMs != null } >= 2,
        )
        assertTrue(dao.syncState()?.initialBackfillComplete == true)

        // A second run must not duplicate rows and must not re-read the journal.
        val callsBefore = source.calls.size
        assertEquals(0, repository.syncLatest(force = true))
        assertEquals(1_200, count())
        assertEquals(1_200, allIds().distinct().size)
        assertTrue(source.calls.size > callsBefore)
    }

    @Test
    fun interruptedBackfillResumesFromTheOldestMirroredRow() = runTest {
        val journal = (0 until 900).map { event(it, base + it) }
        val source = FakeEventSource(journal)
        // Process death after the first (newest) page: Room only holds 400..899 and the durable
        // marker still says the backfill is incomplete.
        dao.insertIgnore(journal.filter { it.timestampMs >= base + 400 }.map { it.toEntity() })
        dao.setSyncState(EventSyncStateEntity(initialBackfillComplete = false))
        val repository = EventRepository(db, source)

        val inserted = repository.syncLatest(force = true)

        assertEquals(400, inserted)
        assertEquals(900, count())
        assertEquals(900, allIds().distinct().size)
        assertTrue(allIds().containsAll(journal.map { it.id }))
        // Resumed strictly below the oldest mirrored row instead of re-reading from the top.
        assertTrue(
            "expected a before-cursor read at the oldest mirrored row, got ${source.calls}",
            source.calls.any {
                it.limit == EventRepository.SYNC_PAGE_SIZE && it.beforeTimestampMs == base + 400
            },
        )
        assertTrue(dao.syncState()?.initialBackfillComplete == true)
    }

    @Test
    fun incrementalSyncCrossesPagesAndBreaksTimestampTiesById() = runTest {
        // Ten events share each timestamp, so a page boundary lands inside a tie group.
        val journal = (0 until 1_200).map { event(it, base + it / 10) }
        val alreadyMirrored = journal.take(505)
        dao.insertIgnore(alreadyMirrored.map { it.toEntity() })
        dao.setSyncState(EventSyncStateEntity(initialBackfillComplete = true))
        val source = FakeEventSource(journal)
        source.calls.clear()
        val repository = EventRepository(db, source)

        val inserted = repository.syncLatest(force = true)

        assertEquals(695, inserted)
        assertEquals(1_200, count())
        val ids = allIds()
        assertEquals(1_200, ids.distinct().size)
        assertEquals(journal.map { it.id }.toSet(), ids.toSet())
        // The cursor started at the local newest row, ascending, so ties continue by id.
        assertTrue(
            "expected an ascending read after the local cursor, got ${source.calls}",
            source.calls.any {
                it.ascending && it.afterTimestampMs == alreadyMirrored.last().timestampMs
            },
        )
    }

    // ---- orphan repair -----------------------------------------------------

    @Test
    fun clearedJournalEmptiesTheMirror() = runTest {
        dao.insertIgnore((0 until 5).map { event(it, base + it).toEntity() })
        dao.setSyncState(EventSyncStateEntity(initialBackfillComplete = true))
        val source = FakeEventSource()
        val repository = EventRepository(db, source)

        repository.syncLatest(force = true)

        assertEquals(0, count())
        assertEquals(emptyList<String>(), allIds())
    }

    @Test
    fun journalNewerThanTheMirrorDropsTheStaleRows() = runTest {
        val stale = (0 until 5).map { event(it, base + it) }
        dao.insertIgnore(stale.map { it.toEntity() })
        dao.setSyncState(EventSyncStateEntity(initialBackfillComplete = true))
        // The journal was cleared and rewritten: its oldest surviving record is newer than anything
        // mirrored, so no mirrored row can still be backed by the journal.
        val fresh = (0 until 5).map { event(it, base + 10_000 + it) }
        val source = FakeEventSource(fresh)
        val repository = EventRepository(db, source)

        val inserted = repository.syncLatest(force = true)

        assertEquals(5, inserted)
        assertEquals(5, count())
        assertEquals(fresh.map { it.id }.toSet(), allIds().toSet())
    }

    @Test
    fun failedProbeKeepsTheMirror() = runTest {
        val mirrored = (0 until 5).map { event(it, base + it) }
        dao.insertIgnore(mirrored.map { it.toEntity() })
        dao.setSyncState(EventSyncStateEntity(initialBackfillComplete = true))
        val newer = (0 until 5).map { event(it, base + 10_000 + it) }
        // The orphan probe fails once (an older module without `event list`, a transient root
        // failure). A failed probe is not an empty journal: the mirror must survive.
        val source = FakeEventSource(newer).apply { failOnCallNumber = 1 }
        val repository = EventRepository(db, source)

        val inserted = repository.syncLatest(force = true)

        assertEquals(5, inserted)
        assertEquals(10, count())
        val ids = allIds()
        assertTrue(ids.containsAll(mirrored.map { it.id }))
        assertTrue(ids.containsAll(newer.map { it.id }))
    }

    // ---- throttle ---------------------------------------------------------

    @Test
    fun syncLatestIsThrottledForTenSecondsAndForceBypassesIt() = runTest {
        val source = FakeEventSource((0 until 3).map { event(it, base + it) })
        val repository = EventRepository(db, source)
        assertEquals(3, repository.syncLatest(force = true))

        source.add(event(3, base + 3))
        source.calls.clear()
        // Within the 10 s window a plain sync is a no-op: it does not even read the journal.
        assertEquals(0, repository.syncLatest())
        assertEquals(0, source.calls.size)
        assertEquals(3, count())

        // `force` bypasses the throttle and pulls the new record.
        assertEquals(1, repository.syncLatest(force = true))
        assertEquals(4, count())
        assertTrue(source.calls.isNotEmpty())
    }

    // ---- history paging and local clear ------------------------------------

    @Test
    fun syncOlderPageAppendsOlderRowsAndClearCacheOnlyTouchesRoom() = runTest {
        val journal = (0 until 600).map { event(it, base + it) }
        // Room already mirrors the newest 100 rows; the journal still holds the older ones.
        dao.insertIgnore(journal.filter { it.timestampMs >= base + 500 }.map { it.toEntity() })
        dao.setSyncState(EventSyncStateEntity(initialBackfillComplete = true))
        val source = FakeEventSource(journal)
        val repository = EventRepository(db, source)

        assertEquals(500, repository.syncOlderPage())
        assertEquals(600, count())
        assertEquals(600, allIds().distinct().size)

        // Nothing older remains to import.
        assertEquals(0, repository.syncOlderPage())
        assertEquals(600, count())

        // Clearing the cache must never touch the journal (the module is the source of truth).
        val callsBefore = source.calls.size
        assertEquals(600, repository.clearLocalCache())
        assertEquals(0, count())
        assertEquals(callsBefore, source.calls.size)
        assertEquals(600, source.journalSize)
    }

    // ---- retention and row cap --------------------------------------------

    @Test
    fun syncDeletesEventsOlderThanThirtyDays() = runTest {
        val now = System.currentTimeMillis()
        val recent = (0 until 3).map { event(it, now - (it + 1) * 60L * 60L * 1_000L) }
        val expired = event(99, now - EventRepository.RETENTION_MS - 24L * 60L * 60L * 1_000L)
        val source = FakeEventSource(recent + expired)
        val repository = EventRepository(db, source)

        repository.syncLatest(force = true)

        assertEquals(3, count())
        assertFalse(allIds().contains(expired.id))
        assertTrue(allIds().containsAll(recent.map { it.id }))
    }

    @Test
    fun localMirrorIsCappedAtFiveThousandRows() = runTest {
        // One bulk DAO insert is far cheaper than replaying 5 005 events through the sync path.
        val seededBase = System.currentTimeMillis() - 60L * 60L * 1_000L
        val seeded = (0 until EventRepository.LOCAL_MAX_EVENTS + 5).map {
            event(it, seededBase + it).toEntity()
        }
        // One transaction: 5 005 individual inserts would each be an implicit transaction, which is
        // the only part of this test that could be slow on a 1-core Robolectric run.
        db.withTransaction { dao.insertIgnore(seeded) }
        assertEquals(EventRepository.LOCAL_MAX_EVENTS + 5, count())

        // The journal still overlaps the mirror, so orphan repair leaves the rows alone and the
        // row cap is what trims the five oldest.
        val overlapping = (5_000 until 5_005).map { event(it, seededBase + it) }
        dao.setSyncState(EventSyncStateEntity(initialBackfillComplete = true))
        val source = FakeEventSource(overlapping)
        val repository = EventRepository(db, source)

        repository.syncLatest(force = true)

        assertEquals(EventRepository.LOCAL_MAX_EVENTS, count())
        val ids = allIds()
        assertTrue(ids.contains(event(5_004, seededBase + 5_004).id))
        assertFalse(ids.contains(event(0, seededBase).id))
        assertFalse(ids.contains(event(4, seededBase + 4).id))
    }

    // ---- field fidelity, tie-splitting backfill and interrupted backfill --

    @Test
    fun syncPreservesEveryEventFieldAndEncodesMetadata() = runTest {
        val metadata = buildJsonObject {
            put("origin", JsonPrimitive("test"))
            put("attempt", JsonPrimitive(2))
            put("nested", buildJsonObject { put("safe", JsonPrimitive(true)) })
        }
        val full = ModuleSystemEvent(
            id = idFor(0, base + 1_234),
            module = "wakelock",
            category = "power",
            eventType = "wakeup_held",
            severity = "warning",
            title = "Held wake lock",
            message = "A wake lock was held for 3600 s",
            timestampMs = base + 1_234,
            metadata = metadata,
            dedupeKey = "wakeup.held:example",
        )
        val source = FakeEventSource(listOf(full))
        val repository = EventRepository(db, source)

        repository.syncLatest(force = true)

        val row = dao.observeRecent(10).first().single()
        assertEquals(full.id, row.id)
        assertEquals("wakelock", row.module)
        assertEquals("power", row.category)
        assertEquals("wakeup_held", row.eventType)
        assertEquals("warning", row.severity)
        assertEquals("Held wake lock", row.title)
        assertEquals("A wake lock was held for 3600 s", row.message)
        assertEquals(base + 1_234, row.timestampMs)
        assertEquals("wakeup.held:example", row.dedupeKey)
        // The JSON blob must be the encoded metadata itself, not a placeholder or another field.
        assertEquals(metadata, Json.parseToJsonElement(row.metadataJson))
    }

    @Test
    fun backfillPageBoundarySplitsATimestampTieWithoutSkipping() = runTest {
        // Three records share each timestamp. The newest page holds ids 0..700 of group 233, so the
        // page ends *inside* a tie group: the continuation must resume with 699/698 (lower ids, same
        // millisecond) instead of dropping them as "already seen".
        val journal = (0 until 1_200).map { event(it, base + it / 3) }
        val source = FakeEventSource(journal)
        val repository = EventRepository(db, source)

        val inserted = repository.syncLatest(force = true)

        assertEquals(1_200, inserted)
        assertEquals(1_200, count())
        val ids = allIds()
        assertEquals(1_200, ids.distinct().size)
        assertEquals(journal.map { it.id }.toSet(), ids.toSet())
        assertTrue(
            "expected the tie-aware before-cursor, got ${source.calls}",
            source.calls.any { it.beforeId == journal[700].id },
        )
    }

    @Test
    fun failedBackfillKeepsImportedRowsAndResumesFromTheBreakpoint() = runTest {
        val journal = (0 until 1_200).map { event(it, base + it) }
        // The first page succeeds, the second one fails: a process death mid-backfill looks the same.
        val source = FakeEventSource(journal).apply { failOnCallNumber = 2 }
        val repository = EventRepository(db, source)

        val failure = runCatching { repository.syncLatest(force = true) }

        assertTrue("the second journal page should fail, got $failure", failure.isFailure)
        assertEquals(500, count())
        assertTrue(dao.syncState()?.initialBackfillComplete != true)

        // The next run resumes below the oldest mirrored row and finishes, without re-reading the
        // page that already landed.
        val inserted = repository.syncLatest(force = true)

        assertEquals(700, inserted)
        assertEquals(1_200, count())
        assertEquals(journal.map { it.id }.toSet(), allIds().toSet())
        assertTrue(dao.syncState()?.initialBackfillComplete == true)
    }

    // ---- helpers ----------------------------------------------------------

    private suspend fun count(): Int = dao.observeFilteredCount(null, null, null).first()

    /** Newest-first mirror rows, matching the screen's order. */
    private suspend fun allIds(): List<String> = dao.observeRecent(10_000).first().map { it.id }

    private fun event(index: Int, timestampMs: Long, id: String = idFor(index, timestampMs)) =
        ModuleSystemEvent(
            id = id,
            module = "system",
            category = "safety",
            eventType = "test_event",
            severity = "info",
            title = "event $index",
            timestampMs = timestampMs,
        )

    private fun ModuleSystemEvent.toEntity() = SystemEventEntity(
        id = id,
        module = module,
        category = category,
        eventType = eventType,
        severity = severity,
        title = title,
        message = message,
        timestampMs = timestampMs,
        metadataJson = "null",
        dedupeKey = dedupeKey,
    )

    /**
     * Real journal ids are `<timestamp_ms>-<pid:010>-<sequence:020>`, so plain string comparison is
     * chronological and the CLI breaks same-millisecond ties with it. Reproducing that fixed-width,
     * timestamp-prefixed shape keeps the fake's tie-breaking identical to the CLI's.
     */
    private fun idFor(index: Int, timestampMs: Long): String =
        "%013d-%010d-%020d".format(timestampMs, 0L, index.toLong())
}

/**
 * In-memory journal with the CLI's cursor semantics: the default page is newest-first, `ascending`
 * is oldest-first, `after*` / `before*` are exclusive cursor bounds and a short page marks the end of
 * the journal.
 *
 * Ordering is `(timestampMs, id)` — the same order the CLI uses. That is faithful here because real
 * ids are `<timestamp_ms>-<pid:010>-<sequence:020>`: their plain string order is chronological, so a
 * lower id means "older within the same millisecond". [EventRepositoryTest.idFor] reproduces that
 * fixed-width shape, which is why tie-breaking by id string is equivalent to the device's order
 * rather than an artefact of the fake.
 */
private class FakeEventSource(events: List<ModuleSystemEvent> = emptyList()) : EventSource {

    data class Call(
        val limit: Int,
        val afterTimestampMs: Long?,
        val afterId: String?,
        val beforeTimestampMs: Long?,
        val beforeId: String?,
        val ascending: Boolean,
    )

    private val journal = events.toMutableList()

    /** Every read the repository made, so a test can assert cursor resumption. */
    val calls = mutableListOf<Call>()

    /**
     * 1-based number of the read that must fail instead of returning a page, simulating an
     * unreadable journal / an older module without `event list`. Cleared once it fires.
     */
    var failOnCallNumber: Int? = null

    val journalSize: Int get() = journal.size

    fun add(vararg events: ModuleSystemEvent) {
        journal += events
    }

    override suspend fun events(
        limit: Int,
        afterTimestampMs: Long?,
        afterId: String?,
        beforeTimestampMs: Long?,
        beforeId: String?,
        ascending: Boolean,
    ): ModuleEventPage {
        calls += Call(limit, afterTimestampMs, afterId, beforeTimestampMs, beforeId, ascending)
        if (failOnCallNumber == calls.size) {
            failOnCallNumber = null
            throw IllegalStateException("simulated journal read failure")
        }
        val ordered = journal.sortedWith(compareBy({ it.timestampMs }, { it.id }))
        val matching = ordered.filter {
            isAfter(it, afterTimestampMs, afterId) && isBefore(it, beforeTimestampMs, beforeId)
        }
        val page = if (ascending) matching.take(limit) else matching.takeLast(limit).asReversed()
        return ModuleEventPage(events = page)
    }

    private fun isAfter(event: ModuleSystemEvent, timestampMs: Long?, id: String?): Boolean {
        if (timestampMs == null) return true
        if (event.timestampMs != timestampMs) return event.timestampMs > timestampMs
        return id == null || event.id > id
    }

    private fun isBefore(event: ModuleSystemEvent, timestampMs: Long?, id: String?): Boolean {
        if (timestampMs == null) return true
        if (event.timestampMs != timestampMs) return event.timestampMs < timestampMs
        return id == null || event.id < id
    }
}
