package io.sysflow.feature.process

import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import io.sysflow.core.root.SysFlowClient
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.Job
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
        val limit: Int = DEFAULT_LIMIT,
        /** A user-initiated refresh uses a longer sampling window than the 2 s poll. */
        val refreshing: Boolean = false,
        /** Human label of the action in flight, or `null`. */
        val actionRunning: String? = null,
        val message: String? = null,
        val error: String? = null,
        /** Experimental actions (OOM / sched / cpuset / freeze) stay hidden until explicitly enabled. */
        val experimental: Boolean = false,
        val selectionMode: Boolean = false,
        val selectedPids: Set<Int> = emptySet(),
    ) {
        val hasExperimental: Boolean
            get() = "oom_score_adj" in features || "sched" in features ||
                "cpuset" in features || "freeze" in features
    }

    private val _state = MutableStateFlow(UiState())
    private var detailJob: Job? = null
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
                    limit = current.limit.coerceIn(DEFAULT_LIMIT, MAX_LIMIT),
                    sort = current.sort,
                    filter = current.filter,
                    query = current.query.ifBlank { null },
                    sampleMs = if (silent) POLL_SAMPLE_MS else REFRESH_SAMPLE_MS,
                    foreground = true,
                )
            }.onSuccess { json ->
                val parsed = parseProcessList(json)
                _state.update {
                    val validPids = parsed.entries.map { entry -> entry.pid }.toSet()
                    it.copy(
                        loading = false,
                        refreshing = false,
                        list = parsed,
                        error = null,
                        selectedPids = if (it.selectionMode) {
                            it.selectedPids.intersect(validPids)
                        } else {
                            emptySet()
                        },
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
        val cleared = query.isEmpty() && _state.value.query.isNotEmpty()
        _state.update { it.copy(query = query) }
        if (cleared) refresh()
    }

    fun submitQuery() {
        refresh()
    }

    fun toggleExperimental() {
        _state.update { it.copy(experimental = !it.experimental) }
    }

    fun loadMore() {
        val nextLimit = (_state.value.limit + LIMIT_STEP).coerceAtMost(MAX_LIMIT)
        if (nextLimit == _state.value.limit) return
        _state.update { it.copy(limit = nextLimit) }
        refresh()
    }

    fun toggleSelectionMode() {
        _state.update {
            val next = !it.selectionMode
            it.copy(selectionMode = next, selectedPids = emptySet())
        }
    }

    fun enterSelectionWith(pid: Int) {
        val entry = _state.value.list?.entries?.firstOrNull { it.pid == pid } ?: return
        if (entry.protected) return
        _state.update { it.copy(selectionMode = true, selectedPids = setOf(pid)) }
    }

    fun toggleSelected(pid: Int) {
        val entry = _state.value.list?.entries?.firstOrNull { it.pid == pid } ?: return
        if (entry.protected) return
        _state.update { current ->
            val next = current.selectedPids.toMutableSet().apply {
                if (!add(pid)) remove(pid)
            }.toSet()
            current.copy(selectedPids = next)
        }
    }

    fun clearSelection() {
        _state.update { it.copy(selectionMode = false, selectedPids = emptySet()) }
    }

    /**
     * Load one process into the detail sheet.
     *
     * [reload] refreshes the open sheet in place after an action, instead of blanking it to a
     * spinner. If the process is gone by then (a successful stop), the sheet closes quietly and the
     * action's own message stays — "读取进程失败" would wrongly read as if the stop had failed.
     */
    fun loadDetail(pid: Int, reload: Boolean = false) {
        detailJob?.cancel()
        if (!reload) _state.update { it.copy(detailLoading = true, detail = null) }
        detailJob = viewModelScope.launch {
            runCatching { client.processGet(pid) }
                .onSuccess { json ->
                    _state.update {
                        it.copy(detailLoading = false, detail = parseProcessDetail(json))
                    }
                }
                .onFailure { error ->
                    if (error is CancellationException) throw error
                    _state.update {
                        if (reload) {
                            it.copy(detailLoading = false, detail = null)
                        } else {
                            it.copy(
                                detailLoading = false,
                                error = error.message ?: "读取进程 $pid 失败",
                            )
                        }
                    }
                }
        }
    }

    /** Closing the sheet also drops a load still in flight, so it cannot pop the sheet back open. */
    fun dismissDetail() {
        detailJob?.cancel()
        _state.update { it.copy(detail = null, detailLoading = false) }
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

    fun sched(target: String, policy: String, priority: Int?) =
        runAction("调整 $target 的调度策略") {
            val result = parseProcessAction(client.processSched(target, policy, priority))
            val failed = result.targets.count { it.verified == false }
            val label = result.policy ?: policy
            val prio = result.priority?.takeIf { it > 0 }?.let { "（优先级 $it）" }.orEmpty()
            if (failed == 0) "$target 的调度策略已设为 $label$prio"
            else "$failed 个进程未能应用调度策略"
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

    fun batchStop() {
        val stateNow = _state.value
        val targets = stateNow.list?.entries.orEmpty()
            .filter { it.pid in stateNow.selectedPids && !it.protected && it.packageName != null }
            .mapNotNull { entry -> entry.packageName?.let { it to entry.user } }
            .distinct()
        if (targets.isEmpty()) return
        viewModelScope.launch {
            _state.update {
                it.copy(actionRunning = "批量强停 ${targets.size} 个应用", message = null, error = null)
            }
            var stopped = 0
            var skipped = 0
            var failed = 0
            var lastError: String? = null
            for ((pkg, user) in targets) {
                runCatching { parseProcessAction(client.processStop(pkg, user)) }
                    .onSuccess { res ->
                        when {
                            res.alreadyStopped -> skipped += 1
                            res.verified == true -> stopped += 1
                            else -> failed += 1
                        }
                    }
                    .onFailure { err ->
                        failed += 1
                        lastError = err.message
                    }
            }
            val summary = buildString {
                append("批量强停完成：已停止 $stopped 个")
                if (skipped > 0) append("，已不在运行 $skipped 个")
                if (failed > 0) append("，未确认或失败 $failed 个")
                if (lastError != null && failed > 0) append("（$lastError）")
            }
            _state.update {
                it.copy(
                    actionRunning = null,
                    selectionMode = false,
                    selectedPids = emptySet(),
                    message = if (failed == 0) summary else null,
                    error = if (failed > 0) summary else null,
                )
            }
            refresh(silent = true)
        }
    }

    fun batchFreeze(frozen: Boolean) {
        val stateNow = _state.value
        val targets = stateNow.list?.entries.orEmpty()
            .filter { it.pid in stateNow.selectedPids && !it.protected }
            .map { entry -> entry.packageName ?: entry.pid.toString() }
            .distinct()
        if (targets.isEmpty()) return
        val verb = if (frozen) "冻结" else "解冻"
        viewModelScope.launch {
            _state.update {
                it.copy(actionRunning = "批量${verb} ${targets.size} 项", message = null, error = null)
            }
            var ok = 0
            var failed = 0
            var lastError: String? = null
            for (target in targets) {
                runCatching { client.processFreeze(target, frozen) }
                    .onSuccess { ok += 1 }
                    .onFailure { err ->
                        failed += 1
                        lastError = err.message
                    }
            }
            val summary = buildString {
                append("批量${verb}完成：成功 $ok 项")
                if (failed > 0) append("，失败 $failed 项")
                if (lastError != null && failed > 0) append("（$lastError）")
            }
            _state.update {
                it.copy(
                    actionRunning = null,
                    selectionMode = false,
                    selectedPids = emptySet(),
                    message = if (failed == 0) summary else null,
                    error = if (failed > 0) summary else null,
                )
            }
            refresh(silent = true)
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
            _state.value.detail?.let { loadDetail(it.pid, reload = true) }
        }
    }

    companion object {
        const val DEFAULT_LIMIT = 80
        const val LIMIT_STEP = 60
        const val MAX_LIMIT = 200
        const val POLL_SAMPLE_MS = 300
        const val REFRESH_SAMPLE_MS = 800
    }
}
