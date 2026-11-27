package io.sysflow.feature.wakelock

import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import io.sysflow.core.model.WAKELOCK_BLOCKED_ITEM
import io.sysflow.core.model.WAKELOCK_EXEMPT_ITEM
import io.sysflow.core.model.WakelockApps
import io.sysflow.core.model.WakelockHeld
import io.sysflow.core.model.WakelockHistory
import io.sysflow.core.model.WakelockState
import io.sysflow.core.model.WakelockWakeup
import io.sysflow.core.model.appliedBlocked
import io.sysflow.core.root.SysFlowClient
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.booleanOrNull
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.put

class WakelockViewModel(private val client: SysFlowClient) : ViewModel() {
    data class UiState(
        val loading: Boolean = true,
        val state: WakelockState? = null,
        /** Packages whose `WAKE_LOCK` op is `ignore`, as the shell reports it right now. */
        val blocked: Set<String> = emptySet(),
        /** Packages on the user whitelist (`wakelock.exempt`): never blocked. */
        val exempt: Set<String> = emptySet(),
        /** A bulk operation (reset) is running. */
        val busy: Boolean = false,
        val held: WakelockHeld? = null,
        val heldLoading: Boolean = false,
        /** Last wakeup reason; loaded when the 唤醒源 tab opens (a `dumpsys power` call). */
        val wakeup: WakelockWakeup? = null,
        val wakeupLoading: Boolean = false,
        /** Recorded trend (the daemon samples every five minutes). */
        val history: WakelockHistory? = null,
        val historyLoading: Boolean = false,
        /** BatteryStats' per-UID wake-lock ranking; loaded on demand (expensive dump). */
        val apps: WakelockApps? = null,
        val appsLoading: Boolean = false,
        /** Package currently being written, so its switch can show progress. */
        val pending: String? = null,
        val error: String? = null,
        val message: String? = null,
    )

    private val _state = MutableStateFlow(UiState())
    val state: StateFlow<UiState> = _state.asStateFlow()
    private val ioMutex = Mutex()

    init {
        refresh()
        loadHeld()
    }

    /** Kernel telemetry plus the live AppOps state (the latter is opt-in, so it is a second call). */
    fun refresh() {
        viewModelScope.launch {
            ioMutex.withLock {
                runCatching {
                    val wakelock = client.get("wakelock")
                    val rules = client.get("wakelock", listOf(WAKELOCK_BLOCKED_ITEM))
                    // The whitelist is configuration, so the persisted intent is the only source.
                    // `snapshot.settings` exposes it without another binder call, and a failure
                    // here must not fail the whole read.
                    val settings = runCatching { client.snapshot().settings }.getOrNull()
                    val exempt = exemptFrom(
                        settings?.get("wakelock")?.jsonObject?.get(WAKELOCK_EXEMPT_ITEM),
                    )
                    Triple(WakelockState.from(wakelock), blockedFrom(rules), exempt)
                }.onSuccess { (state, blocked, exempt) ->
                    _state.update {
                        it.copy(
                            loading = false,
                            state = state ?: it.state,
                            blocked = blocked,
                            exempt = exempt,
                            error = null,
                        )
                    }
                }.onFailure { e ->
                    _state.update { it.copy(loading = false, error = e.message ?: "读取唤醒锁失败") }
                }
            }
        }
    }

    /** BatteryStats wake-lock time per app. On demand only: it spawns a `dumpsys batterystats`. */
    fun loadApps(refresh: Boolean = false) {
        _state.update { it.copy(appsLoading = true) }
        viewModelScope.launch {
            runCatching { WakelockApps.from(client.wakelockApps(limit = 20, refresh = refresh)) }
                .onSuccess { apps ->
                    _state.update {
                        it.copy(
                            appsLoading = false,
                            apps = apps ?: it.apps,
                            error = if (apps == null) "无法读取应用唤醒统计" else null,
                        )
                    }
                }
                .onFailure { e ->
                    _state.update { it.copy(appsLoading = false, error = e.message ?: "读取应用唤醒统计失败") }
                }
        }
    }

    /** Last wakeup reason. On demand only: it spawns a `dumpsys power`, like the held list. */
    fun loadWakeup(refresh: Boolean = false) {
        if (_state.value.wakeupLoading) return
        _state.update { it.copy(wakeupLoading = true) }
        viewModelScope.launch {
            runCatching { WakelockWakeup.from(client.wakelockWakeup(refresh)) }
                .onSuccess { wakeup ->
                    _state.update {
                        it.copy(wakeupLoading = false, wakeup = wakeup ?: it.wakeup)
                    }
                }
                .onFailure { e ->
                    _state.update {
                        it.copy(
                            wakeupLoading = false,
                            error = e.message ?: "读取上次唤醒失败",
                        )
                    }
                }
        }
    }

    /** The recorded 24 h trend; a cheap file read, but it still runs off the main thread. */
    fun loadHistory() {
        if (_state.value.historyLoading) return
        _state.update { it.copy(historyLoading = true) }
        viewModelScope.launch {
            runCatching {
                WakelockHistory.from(client.history("wakelock", since = "24h", step = "30m"))
            }
                .onSuccess { history ->
                    _state.update {
                        it.copy(historyLoading = false, history = history ?: it.history)
                    }
                }
                .onFailure { e ->
                    _state.update {
                        it.copy(historyLoading = false, error = e.message ?: "读取唤醒趋势失败")
                    }
                }
        }
    }

    fun loadHeld(refresh: Boolean = false) {
        _state.update { it.copy(heldLoading = true) }
        viewModelScope.launch {
            runCatching { WakelockHeld.from(client.wakelockHeld(refresh)) }
                .onSuccess { held ->
                    _state.update { it.copy(heldLoading = false, held = held, error = if (held == null) "无法读取当前持锁列表" else null) }
                }
                .onFailure { e ->
                    _state.update { it.copy(heldLoading = false, error = e.message ?: "读取持锁列表失败") }
                }
        }
    }

    /**
     * Block or release one app's wake locks, persisting the rule so `apply-boot` replays it.
     *
     * The module answers with the read-back state; whatever it reports wins over the local guess.
     */
    fun setBlocked(packageName: String, blocked: Boolean) {
        _state.update { it.copy(pending = packageName) }
        viewModelScope.launch {
            runCatching {
                val payload = buildJsonObject {
                    put(WAKELOCK_BLOCKED_ITEM, buildJsonObject { put(packageName, blocked) })
                }
                client.set("wakelock", payload, persist = true)
            }.onSuccess { result ->
                // `applied` is keyed by item id (protocol §3.18); `appliedBlocked` unpacks it.
                // `null` means the module did not report this package at all — that is *unknown*,
                // never success, otherwise a ROM that refuses the change looks applied.
                val actual = appliedBlocked(result)[packageName]
                _state.update { current ->
                    val next = current.blocked.toMutableSet()
                    if (actual != null) {
                        if (actual) next.add(packageName) else next.remove(packageName)
                    }
                    current.copy(
                        pending = null,
                        blocked = next,
                        message = when {
                            actual == null ->
                                if (result.mismatch.contains(WAKELOCK_BLOCKED_ITEM)) {
                                    "系统未接受 $packageName 的变更（ROM 可能拒绝 WAKE_LOCK 控制）"
                                } else {
                                    "系统未确认 $packageName 的变更，请稍后刷新"
                                }
                            actual == blocked ->
                                if (blocked) {
                                    "已拦截 $packageName 的唤醒锁"
                                } else {
                                    "已恢复 $packageName 的唤醒锁"
                                }
                            else ->
                                "系统未接受 $packageName 的变更" +
                                    "（读回为${if (actual) "已拦截" else "未拦截"}）"
                        },
                    )
                }
            }.onFailure { e ->
                _state.update { it.copy(pending = null, message = e.message ?: "写入失败") }
            }
        }
    }

    /**
     * Add or remove a whitelist entry.
     *
     * Exempting implies releasing an existing block, so both items travel in one request: leaving
     * an app both exempt and blocked would make the whitelist a lie.
     */
    fun setExempt(packageName: String, exempted: Boolean) {
        _state.update { it.copy(pending = packageName) }
        viewModelScope.launch {
            runCatching {
                val payload = buildJsonObject {
                    if (exempted) {
                        put(WAKELOCK_BLOCKED_ITEM, buildJsonObject { put(packageName, false) })
                    }
                    put(WAKELOCK_EXEMPT_ITEM, buildJsonObject { put(packageName, exempted) })
                }
                client.set("wakelock", payload, persist = true)
            }.onSuccess { result ->
                val echoed = (result.applied[WAKELOCK_EXEMPT_ITEM] as? JsonObject)
                    ?.get(packageName)
                    ?.let { (it as? JsonPrimitive)?.booleanOrNull }
                _state.update { current ->
                    val exempt = current.exempt.toMutableSet()
                    val blocked = current.blocked.toMutableSet()
                    if (exempted) {
                        exempt.add(packageName)
                        blocked.remove(packageName)
                    } else {
                        exempt.remove(packageName)
                    }
                    current.copy(
                        pending = null,
                        exempt = exempt,
                        blocked = blocked,
                        message = if (echoed == exempted) {
                            if (exempted) {
                                "已保护 $packageName：不再拦截其唤醒锁"
                            } else {
                                "已取消保护 $packageName"
                            }
                        } else {
                            "未能保存 $packageName 的保护设置"
                        },
                    )
                }
            }.onFailure { e ->
                _state.update { it.copy(pending = null, message = e.message ?: "写入失败") }
            }
        }
    }

    /**
     * Release every rule this module persisted.
     *
     * The module restores the AppOps state it found before SysFlow touched it; rules set by other
     * tools are left alone because `reset` only touches its own.
     */
    fun releaseAll() {
        _state.update { it.copy(busy = true) }
        viewModelScope.launch {
            runCatching { client.reset("wakelock") }
                .onSuccess {
                    _state.update {
                        it.copy(
                            busy = false,
                            blocked = emptySet(),
                            exempt = emptySet(),
                            message = "已恢复所有唤醒锁拦截",
                        )
                    }
                    refresh()
                }
                .onFailure { e ->
                    _state.update { it.copy(busy = false, message = e.message ?: "恢复失败") }
                }
        }
    }

    fun dismissMessage() = _state.update { it.copy(message = null) }

    private fun blockedFrom(value: JsonObject): Set<String> =
        readRules(value[WAKELOCK_BLOCKED_ITEM])
            ?.filterValues { it }
            ?.keys
            .orEmpty()

    /** The whitelist arrives as a raw settings map, not as a module item. */
    private fun exemptFrom(value: kotlinx.serialization.json.JsonElement?): Set<String> =
        readRules(value)
            ?.filterValues { it }
            ?.keys
            .orEmpty()

    private fun readRules(value: kotlinx.serialization.json.JsonElement?): Map<String, Boolean>? {
        val obj = value as? JsonObject ?: return null
        return obj.mapNotNull { (key, element) ->
            val flag = (element as? JsonPrimitive)?.booleanOrNull ?: return@mapNotNull null
            key to flag
        }.toMap()
    }
}
