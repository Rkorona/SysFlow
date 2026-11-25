package io.sysflow.feature.cpu

import org.junit.Assert.assertEquals
import org.junit.Test

/**
 * Core switches save immediately, so a write can still be in flight when the next tap arrives.
 * Overwriting the draft with the older read-back would undo that tap on screen — and because the
 * queued write compares the draft with the snapshot, it would then find nothing to send and drop the
 * switch entirely. These tests pin the rule that decides whose mask wins.
 */
class CoreOnlineWriteRaceTest {

    @Test
    fun readBackWinsWhenNoNewerTapArrived() {
        val sent = listOf(1, 1, 1, 0)
        // The kernel refused to take cpu3 offline; the tiles must show that.
        val confirmed = listOf(1, 1, 1, 1)
        assertEquals(confirmed, resolveOnlineAfterWrite(sent, sent, confirmed))
    }

    @Test
    fun newerTapSurvivesTheOlderWrite() {
        val sent = listOf(0, 1, 1, 1)
        val newer = listOf(0, 0, 1, 1)
        // The older write finished after the user tapped another core: keep the newer draft so the
        // queued write still has something to send.
        assertEquals(newer, resolveOnlineAfterWrite(newer, sent, listOf(0, 1, 1, 1)))
    }

    @Test
    fun failedWriteRevertsTheMaskItSent() {
        val sent = listOf(1, 1, 0, 0)
        val lastConfirmed = listOf(1, 1, 1, 1)
        assertEquals(lastConfirmed, resolveOnlineAfterWrite(sent, sent, lastConfirmed))
    }

    @Test
    fun failedWriteDoesNotRevertANewerTap() {
        val sent = listOf(1, 1, 0, 0)
        val newer = listOf(1, 0, 0, 0)
        assertEquals(newer, resolveOnlineAfterWrite(newer, sent, listOf(1, 1, 1, 1)))
    }
}
