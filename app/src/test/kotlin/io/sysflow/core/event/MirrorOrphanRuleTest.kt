package io.sysflow.core.event

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * The APK mirrors the module-owned journal into Room. Clearing the journal (from the CLI, a terminal,
 * or the app) leaves the mirror holding records whose source is gone, and the screen then looks as if
 * the clear did nothing. These tests pin the rule that decides when the mirror must be dropped.
 */
class MirrorOrphanRuleTest {

    @Test
    fun emptyJournalOrphansTheMirror() {
        // The journal was cleared: nothing local can still be confirmed.
        assertTrue(mirrorIsOrphaned(journalOldestMs = null, mirrorNewestMs = 1_700_000_000_000))
    }

    @Test
    fun journalResetPastTheMirrorOrphansIt() {
        // Cleared and then written again: every surviving record is newer than the newest mirrored row.
        assertTrue(
            mirrorIsOrphaned(
                journalOldestMs = 1_700_000_500_000,
                mirrorNewestMs = 1_700_000_000_000,
            ),
        )
    }

    @Test
    fun overlappingJournalKeepsTheMirror() {
        // Normal case: the journal still holds the newest mirrored record.
        assertFalse(
            mirrorIsOrphaned(
                journalOldestMs = 1_699_999_000_000,
                mirrorNewestMs = 1_700_000_000_000,
            ),
        )
        // Boundary: the oldest journal record *is* the newest mirrored row.
        assertFalse(mirrorIsOrphaned(journalOldestMs = 1_700_000_000_000, mirrorNewestMs = 1_700_000_000_000))
    }
}
