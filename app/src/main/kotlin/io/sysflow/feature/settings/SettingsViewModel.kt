package io.sysflow.feature.settings

import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import io.sysflow.core.root.SysFlowClient
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import kotlinx.serialization.json.booleanOrNull
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.intOrNull
import kotlinx.serialization.json.jsonPrimitive

data class SettingsUiState(
    val loading: Boolean = true,
    val pending: Boolean = false,
    val bootApply: Boolean = true,
    val bootDelaySeconds: Int = 30,
    val guardEnabled: Boolean = true,
    val guardRunning: Boolean = false,
    val guardIntervalSeconds: Int = 10,
    val safeMode: Boolean = false,
    val rescueMarker: Boolean = false,
    val bootCount: Int = 0,
    val bootThreshold: Int = 3,
    val hasRecoveryBackup: Boolean = false,
    val recoveryModules: List<String> = emptyList(),
    val hasCorruptConfig: Boolean = false,
    val message: String? = null,
)

class SettingsViewModel(private val client: SysFlowClient) : ViewModel() {
    private val _state = MutableStateFlow(SettingsUiState())
    val state: StateFlow<SettingsUiState> = _state.asStateFlow()

    fun load() {
        viewModelScope.launch {
            _state.update { it.copy(loading = true, message = null) }
            refreshInternal()
        }
    }

    fun refresh() {
        viewModelScope.launch { refreshInternal() }
    }

    private suspend fun refreshInternal() {
        try {
            val boot = client.bootStatus()
            val daemon = client.daemonStatus()
            val safe = client.safeStatus()
            val recovery = client.recoveryStatus()
            _state.update {
                it.copy(
                    loading = false,
                    bootApply = boot["enabled"]?.jsonPrimitive?.booleanOrNull ?: true,
                    bootDelaySeconds = boot["delay_s"]?.jsonPrimitive?.intOrNull ?: 30,
                    guardEnabled = daemon["enabled"]?.jsonPrimitive?.booleanOrNull ?: true,
                    guardRunning = daemon["running"]?.jsonPrimitive?.booleanOrNull ?: false,
                    guardIntervalSeconds = daemon["interval_s"]?.jsonPrimitive?.intOrNull ?: 10,
                    safeMode = safe.safe_mode,
                    rescueMarker = safe.rescue_marker,
                    bootCount = safe.boot_count,
                    bootThreshold = safe.threshold,
                    hasRecoveryBackup = recovery["has_backup"]?.jsonPrimitive?.content?.toBooleanStrictOrNull() ?: false,
                    recoveryModules = (recovery["backup_modules"] as? kotlinx.serialization.json.JsonArray)
                        ?.mapNotNull { it.jsonPrimitive.contentOrNull } ?: emptyList(),
                    hasCorruptConfig = recovery["has_corrupt_config"]?.jsonPrimitive?.content?.toBooleanStrictOrNull() ?: false,
                )
            }
        } catch (e: Exception) {
            _state.update { it.copy(loading = false, message = e.message ?: "读取设置失败") }
        }
    }

    fun setBootApply(enabled: Boolean) {
        mutate("开机应用设置已更新") {
            client.bootConfigure(enabled = enabled)
        }
    }

    fun setGuardEnabled(enabled: Boolean) {
        if (enabled && !_state.value.bootApply) {
            _state.update { it.copy(message = "请先启用开机应用配置") }
            return
        }
        mutate(if (enabled) "持续守护已启用" else "持续守护已停止") {
            client.daemonConfigure(enabled = enabled)
        }
    }

    fun setGuardInterval(seconds: Int) {
        if (seconds !in 2..120) {
            _state.update { it.copy(message = "守护间隔必须在 2–120 秒之间") }
            return
        }
        mutate("守护间隔已保存") { client.daemonConfigure(intervalSeconds = seconds) }
    }

    fun setBootDelay(seconds: Int) {
        if (seconds !in 0..300) {
            _state.update { it.copy(message = "开机延迟必须在 0–300 秒之间") }
            return
        }
        mutate("开机延迟已保存") { client.bootConfigure(delaySeconds = seconds) }
    }

    fun clearSafeMode() {
        mutate("已退出安全模式") { client.safeClear() }
    }

    fun restoreRecovery() {
        mutate("已恢复隔离配置；请确认配置后退出安全模式") { client.recoveryRestore() }
    }

    fun discardRecovery() {
        mutate("已丢弃隔离配置备份") { client.recoveryClear() }
    }

    fun clearCorruptConfig() {
        mutate("已删除损坏的配置备份") { client.recoveryClearCorrupt() }
    }

    fun resetAll() {
        viewModelScope.launch {
            _state.update { it.copy(pending = true, message = null) }
            try {
                val result = client.resetAll()
                val errors = result["errors"]
                _state.update {
                    it.copy(
                        pending = false,
                        message = if (errors == null || errors.toString() == "{}") {
                            "已恢复所有模块默认配置"
                        } else {
                            "部分模块未能恢复，请查看诊断日志"
                        },
                    )
                }
                refreshInternal()
            } catch (e: Exception) {
                _state.update { it.copy(pending = false, message = e.message ?: "恢复配置失败") }
            }
        }
    }

    fun dismissMessage() = _state.update { it.copy(message = null) }

    private fun mutate(success: String, block: suspend () -> Any) {
        viewModelScope.launch {
            _state.update { it.copy(pending = true, message = null) }
            try {
                block()
                _state.update { it.copy(pending = false, message = success) }
                refreshInternal()
            } catch (e: Exception) {
                _state.update { it.copy(pending = false, message = e.message ?: "设置失败") }
            }
        }
    }
}
