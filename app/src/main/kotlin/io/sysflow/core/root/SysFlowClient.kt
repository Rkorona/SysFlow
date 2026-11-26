package io.sysflow.core.root

import io.sysflow.core.model.ApplyResult
import io.sysflow.core.model.CapsData
import io.sysflow.core.model.CheckData
import io.sysflow.core.model.Envelope
import io.sysflow.core.model.ErrorCode
import io.sysflow.core.model.ProtocolError
import io.sysflow.core.model.ModuleEventPage
import io.sysflow.core.model.SafeStatus
import io.sysflow.core.model.Schema
import io.sysflow.core.model.SnapshotData
import io.sysflow.core.model.VersionData
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.serialization.decodeFromString
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.jsonObject

sealed class SysFlowException(message: String) : Exception(message) {
    class Unsupported(message: String) : SysFlowException(message)
    class Permission(message: String) : SysFlowException(message)
    class InvalidArg(message: String) : SysFlowException(message)
    class Busy(message: String) : SysFlowException(message)
    class Io(message: String) : SysFlowException(message)
    class SafeMode(message: String) : SysFlowException(message)
    class ProtocolMismatch(message: String) : SysFlowException(message)
    class NoBinary(message: String) : SysFlowException(message)
}

/**
 * Thin, typed facade over the `sysflow` CLI. The APK never touches sysfs directly: every read and
 * write goes through this class and the binary's JSON envelope.
 */
class SysFlowClient(private val root: RootExecutor) {

    private val json = Json {
        ignoreUnknownKeys = true
        explicitNulls = false
        isLenient = true
    }
    private val writeMutex = Mutex()
    @Volatile
    private var binary: String? = null

    suspend fun hasRoot(): Boolean = root.hasAccess()

    suspend fun version(): VersionData = parseEnvelope(run("version"))

    suspend fun caps(): CapsData = parseEnvelope(run("caps"))

    suspend fun snapshot(): SnapshotData = parseEnvelope(run("snapshot"))

    /** Reads the module-owned event journal. Module/category values intentionally remain open strings. */
    suspend fun events(
        limit: Int = 200,
        afterTimestampMs: Long? = null,
        afterId: String? = null,
        beforeTimestampMs: Long? = null,
        beforeId: String? = null,
        ascending: Boolean = false,
    ): ModuleEventPage {
        val args = buildList {
            add("event"); add("list"); add("--limit"); add(limit.coerceIn(1, 500).toString())
            afterTimestampMs?.let { add("--after-ms"); add(it.toString()) }
            afterId?.let { add("--after-id"); add(it) }
            beforeTimestampMs?.let { add("--before-ms"); add(it.toString()) }
            beforeId?.let { add("--before-id"); add(it) }
            if (ascending) add("--ascending")
        }
        return parseEnvelope(run(*args.toTypedArray()))
    }

    /**
     * Delete every event in the module-owned journal.
     *
     * This only clears the Root journal; the APK's Room mirror keeps its own copy, so callers must
     * also clear the local cache or the list would appear unchanged.
     */
    suspend fun clearEvents(): JsonObject = writeMutex.withLock {
        // Every mutation shares this mutex so a clear cannot interleave with a concurrent apply.
        parseEnvelope<JsonObject>(run("event", "clear", "--all")).jsonObject
    }

    /**
     * Run a command and return the *raw* stdout without envelope parsing.
     *
     * Used by the diagnostics sheet on the events screen: when the app and a terminal disagree about
     * the journal, the only way to settle it is to see exactly what this process executes and what
     * comes back.
     */
    suspend fun raw(command: List<String>): String = try {
        val bin = resolveBinary()
        val line = (listOf(bin) + command).joinToString(" ") { quote(it) }
        val result = root.exec(line)
        "\$ $line\nexit=${result.exitCode}\n${result.stdout}"
            .let { if (result.stderr.isBlank()) it else "$it\nstderr: ${result.stderr}" }
    } catch (cause: Exception) {
        "failed: ${cause::class.simpleName}: ${cause.message}"
    }

    suspend fun info(): JsonObject = parseEnvelope<JsonObject>(run("info")).jsonObject

    /**
     * Cycle count and health with a vendor fallback (Samsung keeps its counter outside sysfs).
     * On demand because the fallback shells out to `dumpsys batteryproperties`.
     */
    suspend fun batteryHealth(refresh: Boolean = false): JsonObject {
        val args = buildList {
            add("battery"); add("health")
            if (refresh) add("--refresh")
        }
        return parseEnvelope<JsonObject>(run(*args.toTypedArray())).jsonObject
    }

    /** Recorded history series for a module (`step` accepts `60`, `5m`, `1h`). */
    suspend fun history(module: String, since: String = "24h", step: String = "1h"): JsonObject {
        val args = buildList {
            add("history"); add(module); add("--since"); add(since); add("--step"); add(step)
        }
        return parseEnvelope<JsonObject>(run(*args.toTypedArray())).jsonObject
    }

    /**
     * Per-UID power attribution from BatteryStats. Expensive (a `dumpsys batterystats` run behind a
     * 60 s module-side cache), so the battery page pulls it on demand instead of every snapshot.
     */
    suspend fun batteryApps(limit: Int = 30, refresh: Boolean = false): JsonObject {
        val args = buildList {
            add("battery"); add("apps"); add("--limit"); add(limit.coerceAtLeast(0).toString())
            if (refresh) add("--refresh")
        }
        return parseEnvelope<JsonObject>(run(*args.toTypedArray())).jsonObject
    }

    /**
     * Per-UID memory ranking (protocol §3.23). Expensive — it opens `/proc` per process — so the
     * module caches it for a minute and the page pulls it on demand.
     */
    suspend fun memApps(limit: Int = 10, refresh: Boolean = false): JsonObject {
        val args = buildList {
            add("mem"); add("apps"); add("--limit"); add(limit.coerceAtLeast(1).toString())
            if (refresh) add("--refresh")
        }
        return parseEnvelope<JsonObject>(run(*args.toTypedArray())).jsonObject
    }

    /**
     * System-vs-user memory split (protocol §3.24). Measures PSS for every process, so it is the
     * heaviest read-only call in the app and is always pulled on demand.
     */
    suspend fun memBreakdown(refresh: Boolean = false): JsonObject {
        val args = buildList {
            add("mem"); add("breakdown")
            if (refresh) add("--refresh")
        }
        return parseEnvelope<JsonObject>(run(*args.toTypedArray())).jsonObject
    }

    /**
     * Live process list (protocol §3.25). Expensive (a two-pass `/proc` scan), so it is a verb and
     * never part of `snapshot`; the page calls it only while visible.
     *
     * @param sampleMs CPU sampling window, 200–2000 ms. 300 ms keeps the 2 s page poll responsive;
     *   the pull-to-refresh path uses a longer window for a steadier number.
     */
    suspend fun processList(
        limit: Int = 80,
        sort: String = "cpu",
        filter: String = "all",
        query: String? = null,
        sampleMs: Int = 300,
        foreground: Boolean = true,
    ): JsonObject {
        val args = buildList {
            add("process"); add("list")
            add("--limit"); add(limit.coerceIn(1, 200).toString())
            add("--sort"); add(sort)
            add("--filter"); add(filter)
            add("--sample-ms"); add(sampleMs.coerceIn(200, 2000).toString())
            if (!foreground) add("--no-foreground")
            if (!query.isNullOrBlank()) {
                add("--query"); add(query.trim())
            }
        }
        return parseEnvelope<JsonObject>(run(*args.toTypedArray())).jsonObject
    }

    /** One process in detail (protocol §3.25); includes PSS and scheduler/cgroup fields. */
    suspend fun processGet(pid: Int): JsonObject =
        parseEnvelope<JsonObject>(run("process", "get", pid.toString())).jsonObject

    /** Force-stop a package (protocol §3.25). `user` is only needed for multi-user ambiguity. */
    suspend fun processStop(target: String, user: Int? = null): JsonObject {
        val args = buildList {
            add("process"); add("stop"); add(target)
            user?.let { add("--user"); add(it.toString()) }
        }
        return parseEnvelope<JsonObject>(run(*args.toTypedArray())).jsonObject
    }

    /** Set `oom_score_adj` (`-1000..=1000`), high risk and never persisted. */
    suspend fun processOom(target: String, adjustment: Int): JsonObject =
        parseEnvelope<JsonObject>(
            run("process", "oom", target, adjustment.coerceIn(-1000, 1000).toString()),
        ).jsonObject

    /** Set nice (`-20..=19`); the module uses `setpriority(2)` first. */
    suspend fun processRenice(target: String, nice: Int): JsonObject =
        parseEnvelope<JsonObject>(
            run("process", "renice", target, nice.coerceIn(-20, 19).toString()),
        ).jsonObject

    /** Move every process of the target into an existing `/dev/cpuset/<group>` (experimental). */
    suspend fun processCpuset(target: String, group: String): JsonObject =
        parseEnvelope<JsonObject>(run("process", "cpuset", target, group)).jsonObject

    /** Freeze / unfreeze (experimental; cgroup v2 leaf first, `am freeze` as fallback). */
    suspend fun processFreeze(target: String, frozen: Boolean): JsonObject =
        parseEnvelope<JsonObject>(
            run("process", if (frozen) "freeze" else "unfreeze", target),
        ).jsonObject

    /** Live storage throughput (protocol §3.26); needs an interval, so it is a verb. */
    suspend fun storageIo(intervalMs: Int = 1_000): JsonObject {
        val args = buildList {
            add("storage"); add("io")
            add("--interval-ms"); add(intervalMs.coerceIn(300, 5_000).toString())
        }
        return parseEnvelope<JsonObject>(run(*args.toTypedArray())).jsonObject
    }

    /** Bounded directory-size ranking; never part of `snapshot` (it walks the tree). */
    suspend fun storageUsage(
        path: String = "/data",
        limit: Int = 20,
        budgetMs: Int = 3_000,
    ): JsonObject {
        val args = buildList {
            add("storage"); add("usage")
            add("--path"); add(path)
            add("--limit"); add(limit.coerceIn(1, 100).toString())
            add("--budget-ms"); add(budgetMs.coerceIn(100, 30_000).toString())
        }
        return parseEnvelope<JsonObject>(run(*args.toTypedArray())).jsonObject
    }

    /** Whole-device mount overview (protocol §3.26): one row per real filesystem. */
    suspend fun storagePartitions(): JsonObject =
        parseEnvelope<JsonObject>(run("storage", "partitions")).jsonObject

    /** One-shot `fstrim -v /data`. */
    suspend fun storageTrim(path: String = "/data"): JsonObject =
        parseEnvelope<JsonObject>(run("storage", "trim", "--path", path)).jsonObject

    /**
     * Userspace wake locks currently held (protocol §3.19).
     *
     * Expensive: it enters the Java system server through `dumpsys power`, so the module caches the
     * answer for a minute and the page pulls it on demand instead of every snapshot.
     */
    suspend fun wakelockHeld(refresh: Boolean = false): JsonObject {
        val args = buildList {
            add("wakelock"); add("held")
            if (refresh) add("--refresh")
        }
        return parseEnvelope<JsonObject>(run(*args.toTypedArray())).jsonObject
    }

    /**
     * The last wakeup reason (protocol §3.21).
     *
     * Same cost as [wakelockHeld]: one `dumpsys power` behind the module's 60 s cache.
     */
    suspend fun wakelockWakeup(refresh: Boolean = false): JsonObject {
        val args = buildList {
            add("wakelock"); add("wakeup")
            if (refresh) add("--refresh")
        }
        return parseEnvelope<JsonObject>(run(*args.toTypedArray())).jsonObject
    }

    /**
     * Per-UID wake-lock time since the last charge (protocol §3.20).
     *
     * Expensive: a `dumpsys batterystats --charged` run, cached for a minute by the module.
     */
    suspend fun wakelockApps(limit: Int = 20, refresh: Boolean = false): JsonObject {
        val args = buildList {
            add("wakelock"); add("apps"); add("--limit"); add(limit.coerceAtLeast(0).toString())
            if (refresh) add("--refresh")
        }
        return parseEnvelope<JsonObject>(run(*args.toTypedArray())).jsonObject
    }

    suspend fun schema(module: String): Schema = parseEnvelope(run(module, "schema"))

    suspend fun get(module: String, keys: List<String> = emptyList()): JsonObject {
        val args = buildList {
            add(module)
            add("get")
            if (keys.isNotEmpty()) {
                add("--keys")
                add(keys.joinToString(","))
            }
        }
        return parseEnvelope<JsonObject>(run(*args.toTypedArray())).jsonObject
    }

    suspend fun set(module: String, payload: JsonObject, persist: Boolean): ApplyResult =
        writeMutex.withLock {
            val args = buildList {
                add(module)
                add("set")
                add(payload.toString())
                if (persist) add("--persist")
            }
            parseEnvelope(run(*args.toTypedArray()))
        }

    suspend fun reset(module: String): JsonObject = writeMutex.withLock {
        parseEnvelope<JsonObject>(run(module, "reset")).jsonObject
    }

    /**
     * Re-seed the stored defaults for CPU from the kernel's own hardware floor.
     *
     * The defaults are what 恢复默认 restores, and they are recorded once and never refreshed, so a
     * baseline captured before a kernel/ROM update can leave the app restoring a minimum frequency
     * the hardware no longer honours. Only defaults change here — the live values are untouched.
     */
    suspend fun cpuRebaseline(): JsonObject = writeMutex.withLock {
        parseEnvelope<JsonObject>(run("cpu", "rebaseline")).jsonObject
    }

    suspend fun check(): CheckData = parseEnvelope(run("check"))

    suspend fun daemonStatus(): JsonObject = parseEnvelope<JsonObject>(run("daemon", "status")).jsonObject

    suspend fun daemonConfigure(enabled: Boolean? = null, intervalSeconds: Int? = null): JsonObject = writeMutex.withLock {
        val args = buildList {
            add("daemon")
            add("configure")
            enabled?.let { add("--enabled"); add(it.toString()) }
            intervalSeconds?.let { add("--interval"); add(it.toString()) }
        }
        parseEnvelope<JsonObject>(run(*args.toTypedArray())).jsonObject
    }

    suspend fun bootStatus(): JsonObject = parseEnvelope<JsonObject>(run("boot", "status")).jsonObject

    suspend fun bootConfigure(enabled: Boolean? = null, delaySeconds: Int? = null): JsonObject = writeMutex.withLock {
        val args = buildList {
            add("boot")
            add("configure")
            enabled?.let { add("--enabled"); add(it.toString()) }
            delaySeconds?.let { add("--delay"); add(it.toString()) }
        }
        parseEnvelope<JsonObject>(run(*args.toTypedArray())).jsonObject
    }

    suspend fun logTail(lines: Int = 200): JsonObject =
        parseEnvelope<JsonObject>(run("log", "--tail", lines.coerceIn(1, 1000).toString())).jsonObject

    suspend fun resetAll(): JsonObject = writeMutex.withLock {
        parseEnvelope<JsonObject>(run("reset", "--all")).jsonObject
    }

    suspend fun safeStatus(): SafeStatus = parseEnvelope(run("safe-mode", "status"))

    suspend fun safeClear(): SafeStatus = writeMutex.withLock {
        parseEnvelope(run("safe-mode", "clear"))
    }

    suspend fun recoveryStatus(): JsonObject = parseEnvelope<JsonObject>(run("recovery", "status")).jsonObject

    suspend fun recoveryRestore(): JsonObject = writeMutex.withLock {
        parseEnvelope<JsonObject>(run("recovery", "restore")).jsonObject
    }

    suspend fun recoveryClear(): JsonObject = writeMutex.withLock {
        parseEnvelope<JsonObject>(run("recovery", "clear")).jsonObject
    }

    suspend fun recoveryClearCorrupt(): JsonObject = writeMutex.withLock {
        parseEnvelope<JsonObject>(run("recovery", "clear-corrupt")).jsonObject
    }

    /** Check the protocol before any screen trusts the payload. */
    suspend fun verifyProtocol(): VersionData {
        val v = version()
        if (v.protocol != io.sysflow.core.model.PROTOCOL_VERSION) {
            throw SysFlowException.ProtocolMismatch(
                "协议版本不匹配：模块 ${v.protocol}，应用 ${io.sysflow.core.model.PROTOCOL_VERSION}。请更新模块或应用。",
            )
        }
        return v
    }

    private suspend fun run(vararg args: String): String {
        val bin = resolveBinary()
        val command = (listOf(bin) + args).joinToString(" ") { quote(it) }
        val result = root.exec(command)
        val stdout = result.stdout.trim()
        if (stdout.isEmpty()) {
            val detail = result.stderr.trim()
            if (detail.contains("not found", ignoreCase = true) || detail.contains("No such file")) {
                throw SysFlowException.NoBinary("找不到 sysflow 二进制，请重新安装模块。")
            }
            if (detail.contains("permission", ignoreCase = true) ||
                detail.contains("Permission denied", ignoreCase = true) ||
                detail.contains("root", ignoreCase = true)
            ) {
                throw SysFlowException.Permission("没有 root 权限，请在 KernelSU / APatch 中授权。")
            }
            throw SysFlowException.Io(detail.ifEmpty { "sysflow 退出码 ${result.exitCode}" })
        }
        return stdout
    }

    private suspend fun resolveBinary(): String {
        binary?.let { return it }
        for (candidate in BIN_CANDIDATES) {
            val probe = if (candidate.startsWith('/')) {
                "[ -x ${quote(candidate)} ]"
            } else {
                "command -v ${quote(candidate)}"
            }
            val result = runCatching { root.exec(probe) }.getOrNull() ?: continue
            if (result.exitCode == 0) {
                binary = candidate
                return candidate
            }
        }
        throw SysFlowException.NoBinary(
            "找不到 sysflow 二进制。请确认模块已安装（/data/adb/modules/sysflow/bin/sysflow）。",
        )
    }

    private inline fun <reified T> parseEnvelope(text: String): T {
        val envelope: Envelope<T> = try {
            json.decodeFromString(text)
        } catch (e: Exception) {
            throw SysFlowException.Io("sysflow 返回了无法解析的输出：${e.message}")
        }
        if (!envelope.ok) {
            throw envelope.error.toException()
        }
        return envelope.data ?: throw SysFlowException.Io("sysflow 返回了空数据。")
    }

    private fun ProtocolError?.toException(): SysFlowException {
        val error = this ?: return SysFlowException.Io("未知错误")
        return when (error.code) {
            ErrorCode.Unsupported -> SysFlowException.Unsupported(error.msg)
            ErrorCode.Permission -> SysFlowException.Permission(error.msg)
            ErrorCode.InvalidArg -> SysFlowException.InvalidArg(error.msg)
            ErrorCode.Busy -> SysFlowException.Busy(error.msg)
            ErrorCode.Io -> SysFlowException.Io(error.msg)
            ErrorCode.SafeMode -> SysFlowException.SafeMode(error.msg)
        }
    }

    private fun quote(value: String): String = "'" + value.replace("'", "'\\''") + "'"

    companion object {
        private val BIN_CANDIDATES = listOf(
            "/data/adb/modules/sysflow/bin/sysflow",
            "sysflow",
        )
    }
}
