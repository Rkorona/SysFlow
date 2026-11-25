package io.sysflow.feature.screenoff

import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import io.sysflow.core.model.describeMismatch
import io.sysflow.core.root.SysFlowClient
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonArray
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.put

class ScreenOffViewModel(private val client: SysFlowClient) : ViewModel() {
    private val _state = MutableStateFlow(ScreenOffUiState())
    val state: StateFlow<ScreenOffUiState> = _state.asStateFlow()
    private val ioMutex = Mutex()

    init { refresh() }

    fun refresh() {
        viewModelScope.launch {
            ioMutex.withLock {
                runCatching { readState(preserveDraft = true) }
                    .onFailure { error ->
                        _state.update { it.copy(loading = false, error = error.message ?: "读取熄屏配置失败") }
                    }
            }
        }
    }

    fun setCap(policyId: String, index: Int) {
        _state.update { current ->
            val policy = current.policies.firstOrNull { it.id == policyId } ?: return@update current
            current.copy(capIndices = current.capIndices + (policyId to index.coerceIn(0, policy.steps.lastIndex.coerceAtLeast(0))))
        }
    }

    fun setCoreClosed(cpu: Int, closed: Boolean) {
        _state.update { current ->
            if (cpu !in 0 until current.coreCount || cpu in current.locked) return@update current
            // A core that was already offline in the normal profile cannot be newly selected,
            // but an existing stale selection must remain removable while the screen is off.
            if (closed && cpu !in current.baseOnline) return@update current
            val next = if (closed) current.offCores + cpu else current.offCores - cpu
            if (closed && !current.canClose(cpu, current.offCores)) return@update current
            current.copy(offCores = next)
        }
    }

    fun setEnabled(enabled: Boolean) {
        if (_state.value.pending || _state.value.safeMode || _state.value.enabled == enabled) return
        viewModelScope.launch {
            ioMutex.withLock {
                _state.update { it.copy(pending = true, error = null, message = null) }
                try {
                    val result = client.set(
                        "cpu",
                        buildJsonObject { put(ITEM_SCREEN_OFF_ENABLED, JsonPrimitive(enabled)) },
                        persist = true,
                    )
                    readState(preserveDraft = true)
                    _state.update {
                        it.copy(
                            pending = false,
                            message = if (result.mismatch.isEmpty()) {
                                if (enabled) "熄屏限制已启用" else "熄屏限制已关闭，正常配置已恢复"
                            } else "开关已保存，但状态未完全同步：${result.describeMismatch().joinToString("；")}",
                        )
                    }
                } catch (e: Exception) {
                    _state.update { it.copy(pending = false, error = e.message ?: "切换熄屏限制失败") }
                }
            }
        }
    }

    fun save() {
        val current = _state.value
        if (!current.canSave) {
            if (current.coreCount > 0 && current.baseOnline.all { it in current.offCores }) {
                _state.update { it.copy(error = "熄屏时至少需要保留一个正常配置为在线的 CPU 核心") }
            }
            return
        }
        submit(clear = false)
    }

    fun clear() {
        if (_state.value.pending || _state.value.safeMode) return
        submit(clear = true)
    }

    fun dismissMessage() { _state.update { it.copy(message = null, error = null) } }

    private fun submit(clear: Boolean) {
        val current = _state.value
        if (current.pending || current.safeMode) return
        if (!clear && current.coreCount > 0 && current.baseOnline.all { it in current.offCores }) {
            _state.update { it.copy(error = "熄屏时至少需要保留一个正常配置为在线的 CPU 核心") }
            return
        }
        viewModelScope.launch {
            ioMutex.withLock {
                _state.update { it.copy(pending = true, error = null, message = null) }
                try {
                    val payload = buildJsonObject {
                        put(
                            ITEM_SCREEN_OFF_MAX,
                            buildJsonObject {
                                current.policies.forEach { policy ->
                                    val value = when {
                                        clear -> 0L
                                        !policy.usable -> current.savedCaps[policy.id] ?: 0L
                                        else -> ScreenOffUiState.capValue(
                                            policy,
                                            current.capIndices[policy.id] ?: policy.steps.lastIndex.coerceAtLeast(0),
                                        )
                                    }
                                    put(policy.id, JsonPrimitive(value))
                                }
                            },
                        )
                        if (current.coreCount > 0) {
                            put(
                                ITEM_SCREEN_OFF_CORES,
                                buildJsonArray {
                                    repeat(current.coreCount) { cpu ->
                                        add(JsonPrimitive(if (!clear && cpu in current.offCores) 1 else 0))
                                    }
                                },
                            )
                        }
                    }
                    val result = client.set("cpu", payload, persist = true)
                    readState(preserveDraft = false)
                    _state.update {
                        it.copy(
                            pending = false,
                            message = when {
                                result.mismatch.isNotEmpty() -> "配置已保存，但部分项目未完全生效：${result.describeMismatch().joinToString("；")}"
                                clear -> "熄屏频率与核心限制已清除"
                                else -> "熄屏配置已保存"
                            },
                        )
                    }
                } catch (e: Exception) {
                    _state.update { it.copy(pending = false, error = e.message ?: if (clear) "清除熄屏配置失败" else "保存熄屏配置失败") }
                }
            }
        }
    }

    private suspend fun readState(preserveDraft: Boolean) {
        val schema = client.schema("cpu")
        val snapshot = client.snapshot()
        val safe = client.safeStatus()
        val parsed = parseScreenOffState(schema, snapshot).copy(safeMode = safe.safe_mode)
        _state.update { previous ->
            val sameTopology = previous.policies.map { it.id to it.cpus } == parsed.policies.map { it.id to it.cpus } &&
                previous.coreCount == parsed.coreCount
            if (preserveDraft && previous.dirty && sameTopology) {
                parsed.copy(
                    capIndices = previous.capIndices,
                    offCores = previous.offCores,
                    pending = previous.pending,
                    message = previous.message,
                    error = previous.error,
                )
            } else {
                parsed.copy(message = previous.message, error = previous.error, pending = previous.pending)
            }
        }
    }
}
