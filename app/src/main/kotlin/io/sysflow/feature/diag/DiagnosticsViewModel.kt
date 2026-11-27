package io.sysflow.feature.diag

import android.os.Build

import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import io.sysflow.core.root.SysFlowClient
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import kotlinx.serialization.encodeToString
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.jsonPrimitive

 data class DiagnosticsUiState(
    val loading: Boolean = false,
    val logsLoading: Boolean = false,
    val report: String = "",
    val logs: String = "",
    val error: String? = null,
)

class DiagnosticsViewModel(private val client: SysFlowClient) : ViewModel() {
    private val _state = MutableStateFlow(DiagnosticsUiState())
    val state: StateFlow<DiagnosticsUiState> = _state.asStateFlow()
    private val pretty = Json { prettyPrint = true }

    fun runDiagnostics() {
        viewModelScope.launch {
            _state.update { it.copy(loading = true, error = null) }
            try {
                val version = client.version()
                val info = client.info()
                val caps = client.caps()
                val boot = client.bootStatus()
                val daemon = client.daemonStatus()
                val safe = client.safeStatus()
                val snapshot = client.snapshot()
                val check = client.check()
                val cpu = snapshot.modules["cpu"] as? JsonObject
                val report = buildString {
                    appendLine("SysFlow Diagnostic Report")
                    appendLine("==========================")
                    appendLine("Module version : ${version.sysflow_version}")
                    appendLine("Protocol       : ${version.protocol}")
                    appendLine("Config version : ${version.config_version}")
                    appendLine("Manufacturer   : ${Build.MANUFACTURER}")
                    appendLine("Device         : ${Build.DEVICE}")
                    appendLine("Model          : ${Build.MODEL}")
                    appendLine("Product        : ${Build.PRODUCT}")
                    appendLine("Android        : ${Build.VERSION.RELEASE} (SDK ${Build.VERSION.SDK_INT})")
                    appendLine("Kernel         : ${info["kernel_release"]?.jsonPrimitive?.content ?: System.getProperty("os.version") ?: "unknown"}")
                    appendLine("SoC vendor     : ${info["soc_manufacturer"]?.jsonPrimitive?.content.orEmpty()}")
                    appendLine("SoC model      : ${info["soc_model"]?.jsonPrimitive?.content.orEmpty()}")
                    appendLine("Board platform : ${info["board_platform"]?.jsonPrimitive?.content.orEmpty()}")
                    appendLine("Hardware       : ${info["hardware"]?.jsonPrimitive?.content.orEmpty()}")
                    appendLine("Product device : ${info["product_device"]?.jsonPrimitive?.content.orEmpty()}")
                    appendLine("Primary ABI    : ${info["primary_abi"]?.jsonPrimitive?.content.orEmpty()}")
                    appendLine("ABI            : ${Build.SUPPORTED_ABIS.joinToString()}")
                    appendLine("Boot apply     : ${boot["enabled"]}")
                    appendLine("Boot delay     : ${boot["delay_s"]} s")
                    appendLine("Guard enabled  : ${daemon["enabled"]}")
                    appendLine("Guard running  : ${daemon["running"]}")
                    appendLine("Guard interval : ${daemon["interval_s"]} s")
                    appendLine("Safe mode      : ${safe.safe_mode}")
                    appendLine("Rescue marker  : ${safe.rescue_marker}")
                    appendLine("Boot count     : ${safe.boot_count}/${safe.threshold}")
                    appendLine()
                    appendLine("Module capabilities")
                    caps.modules.forEach { (id, capability) ->
                        appendLine("- $id: supported=${capability.supported}, features=${capability.features.joinToString()}")
                        capability.reason?.let { appendLine("  reason: $it") }
                    }
                    appendLine()
                    appendLine("Configuration drift")
                    appendLine("clean=${check.clean}")
                    check.drift.forEach { (id, keys) -> appendLine("- $id: ${keys.joinToString()}") }
                    check.errors.forEach { (id, error) -> appendLine("- $id error: ${error.code}: ${error.msg}") }
                    appendLine()
                    appendLine("CPU snapshot")
                    appendLine(cpu?.let { pretty.encodeToString(JsonObject.serializer(), it) } ?: "CPU module data unavailable")
                    if (snapshot.errors.isNotEmpty()) {
                        appendLine()
                        appendLine("Snapshot errors")
                        snapshot.errors.forEach { (id, error) -> appendLine("- $id: ${error.code}: ${error.msg}") }
                    }
                }
                _state.update { it.copy(loading = false, report = report) }
            } catch (e: Exception) {
                _state.update { it.copy(loading = false, error = e.message ?: "诊断失败") }
            }
        }
    }

    fun loadLogs(lines: Int = 200) {
        viewModelScope.launch {
            _state.update { it.copy(logsLoading = true, error = null) }
            try {
                val result = client.logTail(lines)
                val values = result["lines"] as? JsonArray
                val output = values?.joinToString("\n") { it.jsonPrimitive.content } ?: ""
                _state.update { it.copy(logsLoading = false, logs = output) }
            } catch (e: Exception) {
                _state.update { it.copy(logsLoading = false, error = e.message ?: "读取日志失败") }
            }
        }
    }
}
