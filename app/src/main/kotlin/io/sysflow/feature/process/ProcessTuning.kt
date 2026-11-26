package io.sysflow.feature.process

import android.content.Context

/**
 * Tuning of the process page's polling loop: how often the list refreshes, and how long a sampling
 * window each silent poll passes to `sysflow process list --sample-ms`.
 *
 * Both values are pure APK-side preferences — the module has no interest in them and never sees
 * them — so they are saved locally (SharedPreferences), not in `config.json`. The choices match the
 * module's own clamp: `--sample-ms` is clamped to 200..2000 (`docs/protocol.md` §3.25), and a poll
 * shorter than 1 s would spend the whole interval inside `dumpsys`, so it is not offered.
 */
object ProcessTuning {
    /** Default poll interval; the cadence the page shipped with. */
    const val DEFAULT_POLL_MS = 2000

    /** Default silent-poll sample window. */
    const val DEFAULT_SAMPLE_MS = 300

    /** Poll intervals offered in the tune dialog, ms. */
    val POLL_CHOICES = listOf(1000, 2000, 3000, 5000, 10000)

    /** Sample windows offered in the tune dialog, ms; bounds equal the module's clamp. */
    val SAMPLE_CHOICES = listOf(200, 300, 500, 1000, 2000)

    fun clampPoll(ms: Int): Int = ms.coerceIn(POLL_CHOICES.first(), POLL_CHOICES.last())

    fun clampSample(ms: Int): Int = ms.coerceIn(SAMPLE_CHOICES.first(), SAMPLE_CHOICES.last())
}

/** One loaded pair of tuning values, so the store can hand both over at once. */
data class ProcessTuningState(val pollMs: Int, val sampleMs: Int)

/** Local persistence for [ProcessTuning]. Thin on purpose: all logic lives in the pure object. */
class ProcessTuningStore(context: Context) {

    private val prefs = context.getSharedPreferences("process_tuning", Context.MODE_PRIVATE)

    fun load(): ProcessTuningState = ProcessTuningState(
        pollMs = ProcessTuning.clampPoll(prefs.getInt(KEY_POLL, ProcessTuning.DEFAULT_POLL_MS)),
        sampleMs = ProcessTuning.clampSample(
            prefs.getInt(KEY_SAMPLE, ProcessTuning.DEFAULT_SAMPLE_MS),
        ),
    )

    fun save(pollMs: Int, sampleMs: Int) {
        prefs.edit()
            .putInt(KEY_POLL, ProcessTuning.clampPoll(pollMs))
            .putInt(KEY_SAMPLE, ProcessTuning.clampSample(sampleMs))
            .apply()
    }

    private companion object {
        const val KEY_POLL = "poll_ms"
        const val KEY_SAMPLE = "sample_ms"
    }
}
