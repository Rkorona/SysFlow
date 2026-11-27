package io.sysflow.feature.mem

import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import io.sysflow.core.root.SysFlowClient
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonObject

/**
 * State holder for the memory page.
 *
 * The snapshot comes from the same `sysflow snapshot` the rest of the app already polls; the two
 * expensive halves (the 24 h curve and the per-app ranking) are pulled on demand, exactly like the
 * battery page does, because both cross the root shell and the ranking also opens `/proc` per
 * process.
 */
class MemViewModel(private val client: SysFlowClient) : ViewModel() {

    /** Integer range of one writable item, as advertised by the module's schema. */
    data class Range(val min: Int, val max: Int)

    data class UiState(
        val loading: Boolean = true,
        val snapshot: MemSnapshot? = null,
        val history: MemHistory = MemHistory(emptyList(), emptyList(), emptyList()),
        val historyLoading: Boolean = false,
        val apps: MemApps? = null,
        val appsLoading: Boolean = false,
        val breakdown: MemBreakdown? = null,
        val breakdownLoading: Boolean = false,
        /** Ranges only exist for the items the device actually offers. */
        val ranges: Map<String, Range> = emptyMap(),
        /** Unsaved slider positions, keyed like the ranges. */
        val draft: Map<String, Int> = emptyMap(),
        val saving: Boolean = false,
        val message: String? = null,
        val error: String? = null,
    )

    private val _state = MutableStateFlow(UiState())
    val state = _state.asStateFlow()

    init {
        refresh()
        loadSchema()
    }

    fun refresh() {
        viewModelScope.launch {
            runCatching { client.snapshot() }
                .onSuccess { snapshot ->
                    _state.update {
                        it.copy(
                            loading = false,
                            snapshot = parseMemSnapshot(snapshot.modules["mem"]),
                            error = null,
                        )
                    }
                }
                .onFailure { error ->
                    _state.update { it.copy(loading = false, error = error.message ?: "读取失败") }
                }
        }
    }

    /**
     * Slider ranges come from the module, not from the UI: `swappiness` accepts 0..200 only from
     * kernel 5.8, and guessing would let the app offer a value the device rejects.
     */
    fun loadSchema() {
        viewModelScope.launch {
            runCatching { client.schema("mem") }.onSuccess { schema ->
                val ranges = schema.items
                    .mapNotNull { item ->
                        val min = item.min ?: return@mapNotNull null
                        val max = item.max ?: return@mapNotNull null
                        item.id to Range(min.toInt(), max.toInt())
                    }
                    .toMap()
                _state.update { it.copy(ranges = ranges) }
            }
        }
    }

    fun setDraft(id: String, value: Int) {
        _state.update { it.copy(draft = it.draft + (id to value), message = null) }
    }

    /** Persist the draft, then re-read: whatever the kernel kept is what the page shows. */
    fun save() {
        val draft = _state.value.draft
        if (draft.isEmpty() || _state.value.saving) return
        _state.update { it.copy(saving = true, message = null) }
        viewModelScope.launch {
            val payload = buildJsonObject {
                draft.forEach { (id, value) -> put(id, JsonPrimitive(value)) }
            }
            runCatching { client.set("mem", payload, persist = true) }
                .onSuccess { result ->
                    _state.update {
                        it.copy(
                            saving = false,
                            draft = emptyMap(),
                            message = if (result.mismatch.isEmpty()) {
                                "已保存"
                            } else {
                                "内核未接受：" + result.mismatch.joinToString()
                            },
                        )
                    }
                    refresh()
                }
                .onFailure { error ->
                    _state.update {
                        it.copy(saving = false, message = error.message ?: "保存失败")
                    }
                }
        }
    }

    /** Back to the values the device booted with (the module's own default snapshot). */
    fun restoreDefaults() {
        if (_state.value.saving) return
        _state.update { it.copy(saving = true, draft = emptyMap(), message = null) }
        viewModelScope.launch {
            runCatching { client.reset("mem") }
                .onSuccess {
                    _state.update { it.copy(saving = false, message = "已恢复默认") }
                    refresh()
                }
                .onFailure { error ->
                    _state.update {
                        it.copy(saving = false, message = error.message ?: "恢复失败")
                    }
                }
        }
    }

    fun dismissMessage() {
        _state.update { it.copy(message = null) }
    }

    /** 24 h of PSI / availability, downsampled server-side to one point per 30 minutes. */
    fun loadHistory() {
        if (_state.value.historyLoading) return
        _state.update { it.copy(historyLoading = true) }
        viewModelScope.launch {
            runCatching { client.history("mem", since = "24h", step = "30m") }
                .onSuccess { value ->
                    _state.update { it.copy(history = parseMemHistory(value), historyLoading = false) }
                }
                .onFailure {
                    _state.update { state -> state.copy(historyLoading = false) }
                }
        }
    }

    /** System vs user split: the heaviest call on this page, so it is never automatic. */
    fun loadBreakdown(refresh: Boolean = false) {
        if (_state.value.breakdownLoading) return
        _state.update { it.copy(breakdownLoading = true) }
        viewModelScope.launch {
            runCatching { client.memBreakdown(refresh = refresh) }
                .onSuccess { value ->
                    _state.update { it.copy(breakdown = parseMemBreakdown(value), breakdownLoading = false) }
                }
                .onFailure {
                    _state.update { state -> state.copy(breakdownLoading = false) }
                }
        }
    }

    fun loadApps(refresh: Boolean = false, limit: Int = 10) {
        if (_state.value.appsLoading) return
        _state.update { it.copy(appsLoading = true) }
        viewModelScope.launch {
            runCatching { client.memApps(limit = limit, refresh = refresh) }
                .onSuccess { value ->
                    _state.update { it.copy(apps = parseMemApps(value), appsLoading = false) }
                }
                .onFailure {
                    _state.update { state -> state.copy(appsLoading = false) }
                }
        }
    }

}
