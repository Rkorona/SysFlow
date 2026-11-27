package io.sysflow.feature.cpu

import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import io.sysflow.core.model.ApplyResult
import io.sysflow.core.model.CpuDraft
import io.sysflow.core.model.CpuLiveMetrics
import io.sysflow.core.model.CpuLiveTracker
import io.sysflow.core.model.CpuSnapshot
import io.sysflow.core.model.ITEM_GOV
import io.sysflow.core.model.ITEM_MAX
import io.sysflow.core.model.ITEM_MIN
import io.sysflow.core.model.ITEM_ONLINE
import io.sysflow.core.model.SafeStatus
import io.sysflow.core.model.Schema
import io.sysflow.core.model.describeMismatch
import io.sysflow.core.model.formatKhz
import io.sysflow.core.root.SysFlowClient
import io.sysflow.core.root.SysFlowException
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.longOrNull
import kotlinx.serialization.json.buildJsonArray
import kotlinx.serialization.json.buildJsonObject

data class CpuUiState(
    val loading: Boolean = true,
    val schema: Schema? = null,
    val snapshot: CpuSnapshot? = null,
    val live: CpuLiveMetrics = CpuLiveMetrics(),
    val draft: CpuDraft? = null,
    val dirty: Boolean = false,
    val pending: Boolean = false,
    val safeMode: Boolean = false,
    val rescueMarker: Boolean = false,
    val mismatch: List<String> = emptyList(),
    val message: String? = null,
)

class CpuViewModel(private val client: SysFlowClient) : ViewModel() {

    private val _state = MutableStateFlow(CpuUiState())
    val state: StateFlow<CpuUiState> = _state.asStateFlow()
    private val sampleMutex = Mutex()

    /** Serializes core-mask writes so two fast taps cannot interleave their payloads. */
    private val onlineMutex = Mutex()
    private var previousSample: CpuSnapshot? = null

    init {
        load()
    }

    fun load() {
        viewModelScope.launch {
            _state.update { it.copy(loading = true, message = null) }
            try {
                val schema = client.schema("cpu")
                val safe = client.safeStatus()
                val snapshot = client.snapshot()
                val cpu = CpuSnapshot.from(snapshot.modules["cpu"], snapshot.settings, snapshot.defaults)
                previousSample = cpu
                _state.update {
                    it.copy(
                        loading = false,
                        schema = schema,
                        snapshot = cpu,
                        live = cpu?.let { CpuLiveTracker.update(null, it) } ?: CpuLiveMetrics(),
                        draft = if (it.dirty) it.draft else cpu?.let(CpuDraft::from),
                        safeMode = safe.safe_mode,
                        rescueMarker = safe.rescue_marker,
                    )
                }
            } catch (e: Exception) {
                _state.update { it.copy(loading = false, message = e.message ?: "读取 CPU 状态失败") }
            }
        }
    }

    /** Called by the screen on its visible interval; never clobbers an unsaved draft. */
    fun refreshLive() {
        viewModelScope.launch {
            sampleMutex.withLock {
                try {
                    val safe = client.safeStatus()
                    val snapshot = client.snapshot()
                    val cpu = CpuSnapshot.from(snapshot.modules["cpu"], snapshot.settings, snapshot.defaults) ?: return@withLock
                    val previous = previousSample ?: _state.value.snapshot
                    val live = CpuLiveTracker.update(previous, cpu)
                    previousSample = cpu
                    _state.update {
                        it.copy(
                            snapshot = cpu,
                            live = live,
                            draft = if (it.dirty) it.draft else CpuDraft.from(cpu),
                            safeMode = safe.safe_mode,
                            rescueMarker = safe.rescue_marker,
                        )
                    }
                } catch (_: Exception) {
                    // A single failed poll is transient; keep the last good state.
                }
            }
        }
    }

    /**
     * Switch one core on or off and save it immediately.
     *
     * Core online state used to live only in the draft: `applyCluster` sends a single cluster's
     * frequency/governor keys and never the core mask, so a toggled core was silently dropped on the
     * next poll and there was no button that would have saved it. A core switch is a direct hardware
     * action, not a profile value, so it is written as it happens — serialized, so two fast taps
     * cannot interleave, and always from the newest draft.
     */
    fun toggleCore(index: Int, enabled: Boolean) {
        val current = _state.value
        val draft = current.draft ?: return
        val snapshot = current.snapshot ?: return
        if (!enabled && draft.onlineCount <= 1) {
            _state.update { it.copy(message = "至少需要一个核心保持在线") }
            return
        }
        val next = draft.withCore(index, enabled)
        // Optimistic: the tile flips now and the write below confirms it. Because the draft differs
        // from the snapshot while the write is in flight, `refreshLive` keeps this position instead
        // of rebuilding the draft from the old core mask.
        _state.update { it.copy(draft = next, dirty = next.isDirty(snapshot)) }
        commitOnline()
    }

    private fun commitOnline() {
        viewModelScope.launch {
            onlineMutex.withLock {
                val current = _state.value
                val snapshot = current.snapshot ?: return@withLock
                val draft = current.draft ?: return@withLock
                val wanted = draft.online
                if (wanted == snapshot.online) return@withLock
                _state.update { it.copy(pending = true) }
                val payload = buildJsonObject {
                    put(ITEM_ONLINE, buildJsonArray { wanted.forEach { add(JsonPrimitive(it)) } })
                }
                try {
                    val result = client.set("cpu", payload, persist = true)
                    val refreshed = client.snapshot()
                    val cpu = CpuSnapshot.from(
                        refreshed.modules["cpu"],
                        refreshed.settings,
                        refreshed.defaults,
                    )
                    previousSample = cpu
                    val mismatch = mismatchLines(result)
                    // Adopt the read-back mask: a core the kernel refused to switch must not keep
                    // showing the optimistic position. Only if this write is still the newest intent
                    // though — a tap made while it was in flight owns the draft, and overwriting it
                    // here would both undo that tap on screen and make the queued write a no-op.
                    val confirmed = cpu?.online ?: wanted
                    _state.update { state ->
                        val latest = state.draft
                        val nextDraft = latest?.copy(
                            online = resolveOnlineAfterWrite(latest.online, wanted, confirmed),
                        )
                        state.copy(
                            pending = false,
                            snapshot = cpu,
                            live = cpu?.let { value -> CpuLiveTracker.update(null, value) }
                                ?: CpuLiveMetrics(),
                            draft = nextDraft,
                            dirty = cpu != null && nextDraft != null && nextDraft.isDirty(cpu),
                            mismatch = mismatch,
                            // Silence on success: the tile position and the "配置已同步" pill are the
                            // confirmation, and a snackbar per tap would be noise while switching
                            // several cores in a row.
                            message = if (result.mismatch.isEmpty()) {
                                null
                            } else {
                                "核心状态未完全生效：${mismatch.joinToString("；")}"
                            },
                        )
                    }
                } catch (e: Exception) {
                    // Restore the last confirmed mask so the tiles do not claim a switch that failed,
                    // unless a newer tap superseded this one.
                    _state.update { state ->
                        val latest = state.draft
                        state.copy(
                            pending = false,
                            draft = latest?.copy(
                                online = resolveOnlineAfterWrite(
                                    latestOnline = latest.online,
                                    sentOnline = wanted,
                                    confirmedOnline = snapshot.online,
                                ),
                            ),
                            message = e.message ?: "核心状态保存失败",
                        )
                    }
                }
            }
        }
    }

    /**
     * Re-seed the defaults from the kernel's hardware floor and refresh the screen.
     *
     * Only the defaults move; the live frequencies are untouched. A draft with unsaved edits is kept
     * as-is, because rebaselining is not a reason to throw away what the user was editing.
     */
    fun rebaselineDefaults() {
        viewModelScope.launch {
            _state.update { it.copy(pending = true, message = null) }
            try {
                val result = client.cpuRebaseline()
                val refreshed = client.snapshot()
                val cpu = CpuSnapshot.from(
                    refreshed.modules["cpu"],
                    refreshed.settings,
                    refreshed.defaults,
                )
                previousSample = cpu
                val floored = (result["defaults"] as? JsonObject)?.get(ITEM_MIN) as? JsonObject
                val summary = floored?.entries
                    ?.sortedBy { it.key }
                    ?.mapNotNull { (cluster, value) ->
                        val khz = value.jsonPrimitive.longOrNull ?: return@mapNotNull null
                        "簇 $cluster ${formatKhz(khz)}"
                    }
                    ?.joinToString("，")
                _state.update { state ->
                    val nextDraft = if (state.dirty) state.draft else cpu?.let(CpuDraft::from)
                    state.copy(
                        pending = false,
                        snapshot = cpu,
                        live = cpu?.let { value -> CpuLiveTracker.update(null, value) }
                            ?: CpuLiveMetrics(),
                        draft = nextDraft,
                        dirty = cpu != null && nextDraft != null && nextDraft.isDirty(cpu),
                        message = if (summary.isNullOrEmpty()) {
                            "已按硬件下限更新默认值"
                        } else {
                            "已按硬件下限更新默认值：$summary"
                        },
                    )
                }
            } catch (e: Exception) {
                _state.update { it.copy(pending = false, message = e.message ?: "更新默认值失败") }
            }
        }
    }

    fun setMax(id: String, value: Long) = updateDraft { it.withMax(id, value) }

    fun setMin(id: String, value: Long) = updateDraft { it.withMin(id, value) }

    fun setRange(id: String, minValue: Long, maxValue: Long) =
        updateDraft { it.withRange(id, minValue, maxValue) }

    /** Immediately restore only this policy to its first-run min/max/governor values. */
    /**
     * Revert one cluster's unsaved edits. Purely local: nothing is written, so this is the safe
     * "never mind" path that ordinary drafts need.
     */
    fun discardCluster(id: String) = updateDraft { draft -> draft.withoutCluster(id) }

    fun restoreCluster(id: String) {
        val current = _state.value
        val snapshot = current.snapshot ?: return
        val draft = current.draft ?: return
        val min = snapshot.resetMin(id)
        val max = snapshot.resetMax(id)
        if (min <= 0L || max <= 0L || min > max) return
        val governor = snapshot.defaultGovernor[id] ?: snapshot.governor[id]
        val restoredDraft = draft.withRange(id, min, max).let { next ->
            if (governor.isNullOrBlank()) next else next.withGovernor(id, governor)
        }
        val payload = buildJsonObject {
            put(ITEM_MIN, buildJsonObject { put(id, JsonPrimitive(min)) })
            put(ITEM_MAX, buildJsonObject { put(id, JsonPrimitive(max)) })
            if (!governor.isNullOrBlank()) {
                put(ITEM_GOV, buildJsonObject { put(id, JsonPrimitive(governor)) })
            }
        }
        viewModelScope.launch {
            _state.update { it.copy(pending = true, message = null) }
            try {
                val result = client.set("cpu", payload, persist = true)
                val nextSnapshot = client.snapshot()
                val cpu = CpuSnapshot.from(nextSnapshot.modules["cpu"], nextSnapshot.settings, nextSnapshot.defaults)
                previousSample = cpu
                val mismatch = mismatchLines(result)
                _state.update {
                    it.copy(
                        pending = false,
                        snapshot = cpu,
                        live = cpu?.let { value -> CpuLiveTracker.update(null, value) } ?: CpuLiveMetrics(),
                        draft = restoredDraft,
                        dirty = cpu?.let(restoredDraft::isDirty) ?: false,
                        mismatch = mismatch,
                        message = if (result.mismatch.isEmpty()) "Cluster $id 已恢复默认" else
                            "Cluster $id 默认值未完全生效：${mismatch.joinToString("；")}",
                    )
                }
            } catch (e: Exception) {
                _state.update { it.copy(pending = false, message = e.message ?: "恢复默认失败") }
            }
        }
    }

    fun setGovernor(id: String, value: String) = updateDraft { it.withGovernor(id, value) }

    /** Apply only the selected policy, retaining unsaved edits in other policy cards. */
    fun applyCluster(id: String, persist: Boolean = true) {
        val current = _state.value
        val snapshot = current.snapshot ?: return
        val draft = current.draft ?: return
        val payload = draft.diff(snapshot, id) ?: return
        viewModelScope.launch {
            _state.update { it.copy(pending = true, message = null) }
            try {
                val result = client.set("cpu", payload, persist)
                val nextSnapshot = client.snapshot()
                val cpu = CpuSnapshot.from(nextSnapshot.modules["cpu"], nextSnapshot.settings, nextSnapshot.defaults)
                previousSample = cpu
                val mismatch = mismatchLines(result)
                _state.update {
                    val retainedDraft = draft
                    it.copy(
                        pending = false,
                        snapshot = cpu,
                        live = cpu?.let { value -> CpuLiveTracker.update(null, value) } ?: CpuLiveMetrics(),
                        draft = retainedDraft,
                        dirty = cpu?.let(retainedDraft::isDirty) ?: false,
                        mismatch = mismatch,
                        message = if (result.mismatch.isEmpty()) {
                            if (persist) "Cluster $id 已应用并保存" else "Cluster $id 已应用"
                        } else {
                            "Cluster $id 未完全生效：${mismatch.joinToString("；")}"
                        },
                    )
                }
            } catch (e: Exception) {
                _state.update { it.copy(pending = false, message = e.message ?: "应用失败") }
            }
        }
    }

    /** Discard local edits without sending any write command to the device. */
    fun reset() {
        viewModelScope.launch {
            _state.update { it.copy(pending = true, message = null) }
            try {
                client.reset("cpu")
                val snapshot = client.snapshot()
                val cpu = CpuSnapshot.from(snapshot.modules["cpu"], snapshot.settings, snapshot.defaults)
                previousSample = cpu
                _state.update {
                    it.copy(
                        pending = false,
                        snapshot = cpu,
                        live = cpu?.let { CpuLiveTracker.update(null, it) } ?: CpuLiveMetrics(),
                        draft = cpu?.let(CpuDraft::from),
                        dirty = false,
                        mismatch = emptyList(),
                        message = "已恢复 CPU 默认设置",
                    )
                }
            } catch (e: SysFlowException) {
                _state.update { it.copy(pending = false, message = e.message ?: "恢复默认失败") }
            }
        }
    }

    fun clearSafeMode() {
        viewModelScope.launch {
            try {
                val safe: SafeStatus = client.safeClear()
                _state.update { it.copy(safeMode = safe.safe_mode, message = "已退出安全模式") }
                load()
            } catch (e: Exception) {
                _state.update { it.copy(message = e.message ?: "退出安全模式失败") }
            }
        }
    }

    fun dismissMessage() {
        _state.update { it.copy(message = null) }
    }

    /** Turn the protocol's key-only mismatch list into requested/read-back lines for the UI. */
    private fun mismatchLines(result: ApplyResult): List<String> = result.describeMismatch()

    private fun updateDraft(transform: (CpuDraft) -> CpuDraft) {
        _state.update { current ->
            val draft = current.draft ?: return@update current
            val next = transform(draft)
            val snapshot = current.snapshot ?: return@update current
            current.copy(draft = next, dirty = next.isDirty(snapshot))
        }
    }
}

/**
 * The core mask the tiles must show once a core write finishes.
 *
 * [sentOnline] is the mask that write carried and [confirmedOnline] what the device says now (the
 * read-back on success, the last confirmed mask on failure). A newer tap that arrived while the write
 * was in flight owns the draft: overwriting it would undo that tap on screen, and the queued write
 * would then find "nothing changed" and drop it — two switches, one lost.
 *
 * Pure so the rule can be tested without a device or a root shell.
 */
internal fun resolveOnlineAfterWrite(
    latestOnline: List<Int>,
    sentOnline: List<Int>,
    confirmedOnline: List<Int>,
): List<Int> = if (latestOnline == sentOnline) confirmedOnline else latestOnline
