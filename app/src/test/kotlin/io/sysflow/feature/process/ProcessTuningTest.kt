package io.sysflow.feature.process

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

/** Clamp rules of [ProcessTuning]; the store itself is thin SharedPreferences glue. */
class ProcessTuningTest {

    @Test
    fun poll_clamps_to_the_choice_range() {
        assertEquals(1000, ProcessTuning.clampPoll(0))
        assertEquals(1000, ProcessTuning.clampPoll(-500))
        assertEquals(2000, ProcessTuning.clampPoll(2000))
        assertEquals(10000, ProcessTuning.clampPoll(99999))
    }

    @Test
    fun sample_clamps_to_the_module_range() {
        assertEquals(200, ProcessTuning.clampSample(0))
        assertEquals(200, ProcessTuning.clampSample(150))
        assertEquals(300, ProcessTuning.clampSample(300))
        assertEquals(2000, ProcessTuning.clampSample(5000))
    }

    @Test
    fun choices_cover_the_defaults_and_the_module_bounds() {
        assertTrue(ProcessTuning.POLL_CHOICES.contains(ProcessTuning.DEFAULT_POLL_MS))
        assertTrue(ProcessTuning.SAMPLE_CHOICES.contains(ProcessTuning.DEFAULT_SAMPLE_MS))
        assertEquals(200, ProcessTuning.SAMPLE_CHOICES.first())
        assertEquals(2000, ProcessTuning.SAMPLE_CHOICES.last())
        assertTrue(ProcessTuning.POLL_CHOICES.first() >= 1000)
    }
}
