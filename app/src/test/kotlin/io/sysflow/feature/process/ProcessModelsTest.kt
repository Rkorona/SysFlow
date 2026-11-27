package io.sysflow.feature.process

import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.jsonObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/** Parsing rules for `docs/protocol.md` §3.25, shaped like the S25 probe output. */
class ProcessModelsTest {

    private fun json(text: String): JsonObject = Json.parseToJsonElement(text).jsonObject

    private val listPayload = json(
        """
        {
          "available": true, "generated_at": 1795605540, "sample_ms": 300, "clock_ticks": 100,
          "total": 1198, "returned": 2, "truncated": true, "skipped": 1,
          "cpuset_groups": ["background", "top-app"],
          "foreground": {"available": true, "source": "dumpsys",
                         "package": "com.server.auditor.ssh.client", "user": 0,
                         "pid": 8123, "cached": false, "disabled": false},
          "processes": [
            {"pid": 8123, "ppid": 701, "uid": 10123, "user": 0, "app_id": 10123,
             "package": "com.server.auditor.ssh.client", "name": "com.server.auditor.ssh.client",
             "cmdline": "com.server.auditor.ssh.client", "state": "S", "threads": 42,
             "rss_kb": 182344, "cpu_pct": 12.5, "cpu_ticks": 63, "oom_score_adj": 0,
             "cgroup": "/apps/uid_10123/pid_8123", "cpuset": "top-app",
             "foreground": true, "system": false, "protected": false},
            {"pid": 2, "ppid": 0, "uid": 0, "user": 0, "app_id": 0, "package": null,
             "name": "kthreadd", "cmdline": "", "state": "S", "threads": 1,
             "cpu_pct": null, "cpu_ticks": null, "cgroup": "/",
             "foreground": false, "system": true, "protected": true,
             "protected_reason": "kernel thread"}
          ]
        }
        """,
    )

    @Test
    fun list_parses_rows_foreground_and_cpuset_groups() {
        val list = parseProcessList(listPayload)
        assertTrue(list.available)
        assertEquals(1198, list.total)
        assertTrue(list.truncated)
        assertEquals(1, list.skipped)
        assertEquals(300L, list.sampleMs)
        assertEquals(100L, list.clockTicks)
        assertEquals(listOf("background", "top-app"), list.cpusetGroups)
        assertEquals("com.server.auditor.ssh.client", list.foreground.packageName)
        assertEquals(8123, list.foreground.pid)
        assertFalse(list.foreground.disabled)
        assertEquals(2, list.entries.size)

        val app = list.entries[0]
        assertEquals("com.server.auditor.ssh.client", app.packageName)
        assertEquals(12.5, app.cpuPct!!, 0.001)
        assertEquals(182_344L, app.rssKb)
        assertTrue(app.foreground)
        assertFalse(app.system)
        assertFalse(app.protected)
        assertEquals("top-app", app.cpuset)

        val kernel = list.entries[1]
        assertNull(kernel.packageName)
        assertNull(kernel.rssKb)
        assertTrue(kernel.system)
        assertEquals("kernel thread", kernel.protectedReason)
        assertEquals("kthreadd", kernel.displayName)
    }

    @Test
    fun detail_parses_pss_cgroup_and_scheduler_fields() {
        val detail = parseProcessDetail(
            json(
                """
                {"available": true, "pid": 8123, "ppid": 701, "uid": 10123, "user": 0,
                 "app_id": 10123, "package": "com.example.app", "packages": ["com.example.app"],
                 "name": "com.example.app", "cmdline": ["com.example.app"], "state": "S",
                 "threads": 42, "rss_kb": 182344, "pss_kb": 96400, "swap_pss_kb": 0,
                 "cpu_pct": 12.5, "cpu_ticks": 63, "sample_ms": 200, "oom_score_adj": 0,
                 "nice": 0, "rt_priority": 0, "policy": 0,
                 "cgroup": "/apps/uid_10123/pid_8123", "cpuset": "top-app",
                 "start_time_ticks": 2588536, "foreground": true,
                 "system": false, "protected": false, "clock_ticks": 100}
                """,
            ),
        )
        assertEquals(96_400L, detail.pssKb)
        assertEquals(0L, detail.swapPssKb)
        assertEquals("top-app", detail.cpuset)
        assertEquals(2588536L, detail.startTimeTicks)
        assertEquals(listOf("com.example.app"), detail.cmdline)
        assertEquals(42, detail.threads)
    }

    @Test
    fun action_parses_verified_null_for_the_am_freeze_fallback() {
        val action = parseProcessAction(
            json(
                """
                {"frozen": true, "mechanism": "am",
                 "targets": [{"pid": 8123, "before": null, "applied": null, "verified": null}],
                 "note": "verified by the am command's exit status only"}
                """,
            ),
        )
        assertTrue(action.frozen!!)
        assertEquals("am", action.mechanism)
        assertNull(action.targets[0].verified)
        assertFalse(action.alreadyStopped)
    }

    @Test
    fun stop_action_parses_already_stopped() {
        val action = parseProcessAction(
            json("""{"package": "com.example.ghost", "pids_before": [], "pids_after": [], "verified": true, "already_stopped": true}"""),
        )
        assertTrue(action.alreadyStopped)
        assertTrue(action.verified!!)
        assertTrue(action.pidsBefore.isEmpty())
    }

    @Test
    fun formatting_degrades_with_a_dash_and_keeps_one_decimal() {
        assertEquals("—", formatProcessKb(null))
        assertEquals("—", formatProcessCpu(null))
        assertEquals("133.5 MB", formatProcessKb(136_684))
        assertEquals("1.0 GB", formatProcessKb(1_048_576))
        assertEquals("512 KB", formatProcessKb(512))
        assertEquals("12.5%", formatProcessCpu(12.5))
        assertEquals("100.0%", formatProcessCpu(100.0))
    }

    @Test
    fun subtitle_omits_missing_fields() {
        val entry = ProcessEntry(
            pid = 1, ppid = 0, uid = 0, user = 0, appId = 0, packageName = null,
            name = "init", cmdline = "", state = "S", threads = 3, rssKb = 1000,
            cpuPct = null, cpuTicks = null, oomScoreAdj = null, cgroup = null,
            cpuset = null, foreground = false, system = true, protected = true,
            protectedReason = "pid ≤ 2 is the kernel",
        )
        assertEquals("UID 0 · 3 线程", processSubtitle(entry))
    }

    @Test
    fun list_parses_the_frozen_field() {
        val list = parseProcessList(
            json(
                """
                {"available": true, "total": 1, "returned": 1, "truncated": false, "skipped": 0,
                 "sample_ms": 300, "clock_ticks": 100, "cpuset_groups": [],
                 "foreground": {"available": false, "package": null, "pid": null, "user": 0,
                                "cached": false, "disabled": true},
                 "processes": [
                   {"pid": 8123, "ppid": 701, "uid": 10123, "user": 0, "app_id": 10123,
                    "package": "com.example.app", "name": "com.example.app",
                    "cmdline": "com.example.app", "state": "S", "threads": 12,
                    "rss_kb": 136684, "cpu_pct": 12.5, "cpu_ticks": 38, "oom_score_adj": 0,
                    "cgroup": "/apps/uid_10123/pid_8123", "cpuset": "top-app", "frozen": true,
                    "foreground": true, "system": false, "protected": false}
                 ]}
                """,
            ),
        )
        assertEquals(true, list.entries.single().frozen)
        assertEquals("133.5 MB", formatProcessKb(list.entries.single().rssKb))
    }

    @Test
    fun list_parses_the_stoppable_budget() {
        val list = parseProcessList(
            json(
                """
                {"available": true, "total": 5, "returned": 1, "truncated": false, "skipped": 0,
                 "stoppable": 3, "sample_ms": 300, "clock_ticks": 100, "cpuset_groups": [],
                 "foreground": {"available": false, "package": null, "pid": null, "user": 0,
                                "cached": false, "disabled": true},
                 "processes": []}
                """,
            ),
        )
        assertEquals(3, list.stoppable)
    }

    @Test
    fun detail_parses_the_policy_name_and_frozen_state() {
        val detail = parseProcessDetail(
            json(
                """
                {"pid": 8123, "ppid": 701, "uid": 10123, "user": 0, "app_id": 10123,
                 "package": "com.example.app", "packages": ["com.example.app"],
                 "name": "com.example.app", "cmdline": ["com.example.app", "--worker"],
                 "state": "S", "threads": 8, "rss_kb": 51200, "pss_kb": 40960, "swap_pss_kb": 0,
                 "cpu_pct": 1.2, "sample_ms": 200, "oom_score_adj": 100,
                 "nice": -5, "rt_priority": 10, "policy": 1, "policy_name": "SCHED_FIFO",
                 "cgroup": "/apps/uid_10123/pid_8123", "cpuset": "top-app",
                 "start_time_ticks": 999, "frozen": false,
                 "foreground": false, "system": false, "protected": false}
                """,
            ),
        )
        assertEquals(false, detail.frozen)
        assertEquals("SCHED_FIFO", schedPolicyLabel(detail.policy, detail.policyName))
        assertEquals("fifo", schedPolicyArg(detail.policy))
        assertTrue(isRealtimeSchedPolicy(detail.policy))
        assertFalse(isRealtimeSchedPolicy(0))
        assertEquals(listOf("com.example.app", "--worker"), detail.cmdline)
    }

    @Test
    fun action_parses_sched_and_stop_results() {
        val sched = parseProcessAction(
            json(
                """
                {"target": "com.example.app", "policy": "SCHED_RR", "policy_id": 2, "priority": 25,
                 "targets": [
                   {"pid": 8123, "before": "SCHED_OTHER (0)", "applied": "SCHED_RR (25)",
                    "verified": true, "mechanism": "sched_setscheduler"}
                 ]}
                """,
            ),
        )
        assertEquals("SCHED_RR", sched.policy)
        assertEquals(2, sched.policyId)
        assertEquals(25, sched.priority)
        assertEquals("SCHED_OTHER (0)", sched.targets.first().before)
        assertEquals("SCHED_RR (25)", sched.targets.first().applied)

        val stop = parseProcessAction(
            json("""{"package": "com.example.app", "user": 0, "pids_before": [1, 2], "pids_after": [], "verified": true, "already_stopped": false}"""),
        )
        assertEquals("com.example.app", stop.pkg)
        assertNull(stop.target)
        assertEquals(true, stop.verified)
    }

    @Test
    fun stop_all_result_parses_counts_and_skips() {
        val result = parseProcessStopAll(
            json(
                """
                {"stopped": ["com.example.game", "com.example.notes"], "packages": 2,
                 "skipped_protected": 3, "skipped_unreadable": 1,
                 "skipped_foreground": "io.sysflow",
                 "failed": [{"package": "com.example.bad", "error": "timed out"}],
                 "pids_after": [23123], "verified": false}
                """,
            ),
        )
        assertEquals(listOf("com.example.game", "com.example.notes"), result.stopped)
        assertEquals(2, result.packages)
        assertEquals(3, result.skippedProtected)
        assertEquals("io.sysflow", result.skippedForeground)
        assertEquals(listOf("com.example.bad"), result.failed)
        assertEquals(listOf(23123), result.pidsAfter)
        assertFalse(result.verified)
    }

    @Test
    fun custom_numeric_helpers_handle_signs_and_bounds() {
        assertEquals("-15", filterSignedDigits(" -1a5b ", 3))
        assertEquals("123", filterSignedDigits("12-34", 3))
        assertEquals("-", toggleNumberSign("0"))
        assertEquals("", toggleNumberSign("-"))
        assertEquals("-10", toggleNumberSign("10"))
        assertEquals("10", toggleNumberSign("-10"))
    }
}
