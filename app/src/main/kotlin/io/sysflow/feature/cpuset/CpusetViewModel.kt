package io.sysflow.feature.cpuset

import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import io.sysflow.core.model.SnapshotData
import io.sysflow.core.root.SysFlowClient
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.put

class CpusetViewModel(private val client: SysFlowClient) : ViewModel() {
    private val _state = MutableStateFlow(ProcessGroupsUiState())
    val state: StateFlow<ProcessGroupsUiState> = _state.asStateFlow()
    private val ioMutex = Mutex()

    init { refresh() }

    fun refresh() {
        viewModelScope.launch {
            ioMutex.withLock {
                runCatching { readState() }
                    .onFailure { e ->
                        _state.update { it.copy(loading = false, error = e.message ?: "读取进程组失败") }
                    }
            }
        }
    }

    fun toggle(groupId: String, cpu: Int) {
        _state.update { current ->
            current.copy(groups = current.groups.map { group ->
                if (group.id != groupId || cpu in group.locked) group
                else {
                    val next = group.selected.toMutableSet().apply {
                        if (!add(cpu)) remove(cpu)
                    }.toSet()
                    group.copy(selected = next, dirty = next != group.configured)
                }
            }) }
    }

    fun select(groupId: String, cpus: List<Int>) {
        _state.update { current ->
            current.copy(groups = current.groups.map { group ->
                if (group.id != groupId) group
                else group.copy(
                    selected = cpus.filter { it in 0 until group.count }.toSet(),
                    dirty = cpus.filter { it in 0 until group.count }.toSet() != group.configured,
                )
            }) }
    }

    fun apply(groupId: String) {
        val group = _state.value.groups.firstOrNull { it.id == groupId } ?: return
        submit(groupId, group.selected)
    }

    fun reset(groupId: String) {
        val group = _state.value.groups.firstOrNull { it.id == groupId } ?: return
        submit(groupId, group.default)
    }

    fun resetAll() {
        val state = _state.value
        if (state.groups.isEmpty() || state.busy) return
        viewModelScope.launch {
            ioMutex.withLock {
                _state.update { current ->
                    current.copy(
                        resettingAll = true,
                        message = null,
                        error = null,
                        groups = current.groups.map { group -> group.copy(selected = group.default, dirty = group.default != group.configured) },
                    )
                }
                try {
                    val payload = buildJsonObject {
                        state.groups.forEach { group -> put("$CPUSET_ITEM_PREFIX${group.id}", encodeCpuSet(group, group.default)) }
                    }
                    val result = client.set("cpu", payload, persist = true)
                    readState(preserveDrafts = true)
                    _state.update { it.copy(resettingAll = false, message = if (result.mismatch.isEmpty()) "所有进程组已恢复默认" else "部分进程组未能完全恢复") }
                } catch (e: Exception) {
                    _state.update { it.copy(resettingAll = false, error = e.message ?: "恢复进程组失败") }
                }
            }
        }
    }

    fun dismissMessage() { _state.update { it.copy(message = null, error = null) } }

    private fun submit(groupId: String, selected: Set<Int>) {
        val group = _state.value.groups.firstOrNull { it.id == groupId } ?: return
        if (groupId.isBlank() || selected.isEmpty() || selected.none { it in group.online }) {
            _state.update { it.copy(error = if (selected.isEmpty()) "至少选择一个 CPU 核心" else "至少选择一个当前在线的核心") }
            return
        }
        viewModelScope.launch {
            ioMutex.withLock {
                _state.update { current ->
                    current.copy(
                        pendingGroups = current.pendingGroups + groupId,
                        error = null,
                        message = null,
                        groups = current.groups.map { item ->
                            if (item.id == groupId) item.copy(selected = selected, dirty = selected != item.configured) else item
                        },
                    )
                }
                try {
                    val payload = buildJsonObject {
                        put("$CPUSET_ITEM_PREFIX$groupId", encodeCpuSet(group, selected))
                    }
                    val result = client.set("cpu", payload, persist = true)
                    val partial = result.mismatch.isNotEmpty()
                    readState(preserveDrafts = partial)
                    _state.update { current ->
                        current.copy(
                            pendingGroups = current.pendingGroups - groupId,
                            message = if (partial) "设置已提交，但部分离线核心暂未生效" else if (selected == group.default) "已恢复默认" else "进程组已应用",
                        )
                    }
                } catch (e: Exception) {
                    _state.update { current -> current.copy(pendingGroups = current.pendingGroups - groupId, error = e.message ?: "应用进程组失败") }
                }
            }
        }
    }

    private suspend fun readState(preserveDrafts: Boolean = true) {
        val nextSchema = client.schema("cpu")
        val nextSnapshot = client.snapshot()
        val previous = if (preserveDrafts) _state.value.groups else emptyList()
        _state.update {
            val built = buildProcessGroupStates(nextSchema, nextSnapshot, previous)
            built.copy(
                pendingGroups = it.pendingGroups,
                resettingAll = it.resettingAll,
                message = it.message,
                error = it.error,
            )
        }
    }

}
