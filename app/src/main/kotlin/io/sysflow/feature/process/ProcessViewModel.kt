package io.sysflow.feature.process

import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import io.sysflow.core.root.SysFlowClient
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import java.util.Locale

/**
 * State holder for the process page.
 *
 * The list is pulled from `sysflow process list` only while the page is visible (the screen owns
 * the polling loop, as on every other page — the APK has no background work). `get` and the actions
 * are pull-on-demand. Nothing here persists: the module treats every action as one-shot.
 */
class ProcessViewModel(private val client: SysFlowClient) : ViewModel() {

    data class UiState(
        val loading: Boolean = true,
        val list: ProcessList? = null,
        val detail: ProcessDetail? = null,
        val detailLoading: Boolean = false,
        /** `caps.process.features`; an absent feature hides its control instead of failing later. */
        val features: Set<String> = emptySet(),
        val sort: String = "cpu",
        val filter: String = "all",
        val query: String = "",
        /** A user-initiated refresh uses a longer sampling window than the 2 s poll. */
        val refreshing: Boolean = false,
        /** Human label of the action in flight, or `null`. */
        val actionRunning: String? = null,
        val message: String? = null,
        val error: String? = null,
        /** Experimental actions (OOM / cpuset / freeze) stay hidden until explicitly enabled. */
        val experimental: Boolean = false,
    ) {
        val hasExperimental: Boolean
            get() = "oom_score_adj" in features || "cpuset" in features || "freeze" in features
    }

    private val _state = MutableStateFlow(UiState())
    val state = _state.asStateFlow()

    init {
        refresh()
        loadFeatures()
    }

    /** Silent polls use the short sample window; pull-to-refresh uses 800 ms for a steadier value. */
    fun refresh(silent: Boolean = false) {
        viewModelScope.launch {
            val current = _state.value
            if (!silent) _state.update { it.copy(refreshing = true) }
            runCatching {
                client.processList(
                    limit = LIST_LIMIT,
                    sort = current.sort,
                    filter = current.filter,
                    query = current.query.ifBlank { null },
                    sampleMs = if (silent) POLL_SAMPLE_MS else REFRESH_SAMPLE_MS,
                    foreground = true,
                )
            }.onSuccess { json ->
                _state.update {
                    it.copy(
                        loading = false,
                        refreshing = false,
                        list = parseProcessList(json),
                        error = null,
                    )
                }
            }.onFailure { error ->
                _state.update {
                    it.copy(
                        loading = false,
                        refreshing = false,
                        error = error.message ?: "读取进程列表失败",
                    )
                }
            }
        }
    }

    /** Capability detection: never show an action the module did not advertise. */
    fun loadFeatures() {
        viewModelScope.launch {
            runCatching { client.caps() }.onSuccess { caps ->
                _state.update { it.copy(features = processFeatures(caps)) }
            }
        }
    }

    fun setSort(sort: String) {
        _state.update { it.copy(sort = sort) }
        refresh()
    }

    fun setFilter(filter: String) {
        _state.update { it.copy(filter = filter) }
        refresh()
    }

    fun setQuery(query: String) {
        _state.update { it.copy(query = query) }
    }

    fun submitQuery() {
        refresh()
    }

    fun toggleExperimental() {
        _state.update { it.copy(experimental = !it.experimental) }
    }

    fun loadDetail(pid: Int) {
        _state.update { it.copy(detailLoading = true, detail = null) }
        viewModelScope.launch {
            runCatching { client.processGet(pid) }
                .onSuccess { json ->
                    _state.update {
                        it.copy(detailLoading = false, detail = parseProcessDetail(json))
                    }
                }
                .onFailure { error ->
                    _state.update {
                        it.copy(
                            detailLoading = false,
                            error = error.message ?: "读取进程 $pid 失败",
                        )
                    }
                }
        }
    }

    fun dismissDetail() {
        _state.update { it.copy(detail = null) }
    }

    fun stop(target: String, user: Int? = null) = runAction("停止 $target") {
        val json = client.processStop(target, user)
        val result = parseProcessAction(json)
        when {
            result.alreadyStopped -> "$target 没有运行中的进程"
            result.verified == true -> "已停止 $target"
            else -> "已请求停止 $target，但仍有进程存活"
        }
    }

    fun oom(target: String, adjustment: Int) = runAction("调整 $target 的 OOA") {
        val result = parseProcessAction(client.processOom(target, adjustment))
        val failed = result.targets.count { it.verified == false }
        if (failed == 0) {
            "$target 的 oom_score_adj 已设为 $adjustment"
        } else {
            "$failed 个进程未能应用 oom_score_adj"
        }
    }

    fun renice(target: String, nice: Int) = runAction("调整 $target 的优先级") {
        val result = parseProcessAction(client.processRenice(target, nice))
        val failed = result.targets.count { it.verified == false }
        if (failed == 0) "$target 的 nice 已设为 $nice" else "$failed 个进程未能调整 nice"
    }

    fun cpuset(target: String, group: String) = runAction("把 $target 移入 $group") {
        val result = parseProcessAction(client.processCpuset(target, group))
        val moved = result.targets.count { it.verified == true }
        "已把 $target 的 $moved 个进程移入 $group（ActivityManager 可能随后改回）"
    }

    fun freeze(target: String, frozen: Boolean) = runAction(
        if (frozen) "冻结 $target" else "解冻 $target",
    ) {
        val result = parseProcessAction(client.processFreeze(target, frozen))
        val verb = if (frozen) "冻结" else "解冻"
        val unverified = result.targets.count { it.verified == null }
        if (unverified > 0) {
            "$verb $target 已提交（am 路径只能看退出码，未逐进程读回）"
        } else {
            "$verb $target 完成"
        }
    }

    fun dismissMessage() {
        _state.update { it.copy(message = null, error = null) }
    }

    private fun runAction(label: String, block: suspend () -> String) {
        viewModelScope.launch {
            _state.update { it.copy(actionRunning = label, message = null, error = null) }
            runCatching { block() }
                .onSuccess { message ->
                    _state.update { it.copy(actionRunning = null, message = message) }
                }
                .onFailure { error ->
                    _state.update {
                        it.copy(
                            actionRunning = null,
                            error = error.message ?: String.format(Locale.ROOT, "%s 失败", label),
                        )
                    }
                }
            // The action changed live process state; refresh the list and the open detail if any.
            refresh(silent = true)
            _state.value.detail?.let { loadDetail(it.pid) }
        }
    }

    companion object {
        const val LIST_LIMIT = 80
        const val POLL_SAMPLE_MS = 300
        const val REFRESH_SAMPLE_MS = 800
    }
}
