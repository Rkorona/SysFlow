package io.sysflow.feature.storage

import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import io.sysflow.core.root.SysFlowClient
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.put

/**
 * State holder for the storage page.
 *
 * The snapshot gives capacity/device/current values; the live I/O card polls `storage io` while the
 * page is visible (the screen owns the loop); history, TRIM and the directory analysis are pulled on
 * demand. The two tunables are the only persisted writes, and each one is validated by the module
 * against the kernel's own option list before it is applied.
 */
class StorageViewModel(private val client: SysFlowClient) : ViewModel() {

    data class UiState(
        val loading: Boolean = true,
        val snapshot: StorageSnapshot? = null,
        val io: StorageIo? = null,
        val ioLoading: Boolean = false,
        val history: StorageHistory = StorageHistory(emptyList(), emptyList()),
        val historyLoading: Boolean = false,
        val options: StorageOptions = StorageOptions(emptyList(), emptyList()),
        val usage: StorageUsage? = null,
        val usageLoading: Boolean = false,
        /** What the directory analysis scans: user-visible storage or the whole /data partition. */
        val usagePath: String = USER_STORAGE,
        val saving: Boolean = false,
        val actionRunning: String? = null,
        val message: String? = null,
        val error: String? = null,
    )

    private val _state = MutableStateFlow(UiState())
    val state = _state.asStateFlow()

    init {
        refresh()
        loadSchema()
        loadHistory()
    }

    fun refresh() {
        viewModelScope.launch {
            runCatching { client.snapshot() }
                .onSuccess { snapshot ->
                    _state.update {
                        it.copy(
                            loading = false,
                            snapshot = parseStorageSnapshot(snapshot.modules["storage"]),
                            error = null,
                        )
                    }
                }
                .onFailure { error ->
                    _state.update {
                        it.copy(loading = false, error = error.message ?: "读取储存信息失败")
                    }
                }
        }
    }

    /** One live I/O reading; the screen calls this every two seconds while visible. */
    fun pollIo() {
        viewModelScope.launch {
            runCatching { client.storageIo() }
                .onSuccess { json ->
                    _state.update { it.copy(io = parseStorageIo(json), ioLoading = false) }
                }
                .onFailure {
                    // Keep the last reading: a single failed interval should not blank the card.
                    _state.update { it.copy(ioLoading = false) }
                }
        }
    }

    fun loadHistory() {
        viewModelScope.launch {
            _state.update { it.copy(historyLoading = true) }
            runCatching { client.history("storage", since = "24h", step = "1h") }
                .onSuccess { json ->
                    _state.update {
                        it.copy(history = parseStorageHistory(json), historyLoading = false)
                    }
                }
                .onFailure {
                    _state.update { it.copy(historyLoading = false) }
                }
        }
    }

    fun loadSchema() {
        viewModelScope.launch {
            runCatching { client.schema("storage") }.onSuccess { schema ->
                _state.update { it.copy(options = parseStorageOptions(schema)) }
            }
        }
    }

    fun setScheduler(value: String) {
        save("storage.scheduler", value)
    }

    fun setReadAhead(value: Int) {
        save("storage.read_ahead_kb", value)
    }

    private fun save(id: String, value: Any) {
        viewModelScope.launch {
            _state.update { it.copy(saving = true, message = null, error = null) }
            runCatching {
                val payload = buildJsonObject {
                    when (value) {
                        is String -> put(id, value)
                        is Int -> put(id, value)
                        else -> put(id, value.toString())
                    }
                }
                client.set("storage", payload, persist = true)
            }.onSuccess { result ->
                val applied = result.applied[id]?.toString()?.trim('"') ?: value.toString()
                val mismatch = result.mismatch.contains(id)
                _state.update {
                    it.copy(
                        saving = false,
                        message = if (mismatch) {
                            "已请求 $applied，但内核没有接受该值"
                        } else {
                            "已设置为 $applied"
                        },
                    )
                }
                refresh()
            }.onFailure { error ->
                _state.update {
                    it.copy(saving = false, error = error.message ?: "设置失败")
                }
            }
        }
    }

    fun trim() {
        viewModelScope.launch {
            _state.update { it.copy(actionRunning = "正在 TRIM", message = null, error = null) }
            runCatching { client.storageTrim() }
                .onSuccess { json ->
                    val bytes = (json["trimmed_bytes"] as? JsonPrimitive)
                        ?.contentOrNull?.toLongOrNull() ?: 0L
                    val kb = bytes / 1024
                    _state.update {
                        it.copy(
                            actionRunning = null,
                            message = if (bytes > 0) {
                                "TRIM 完成，回收 ${formatStorageKb(kb)}"
                            } else {
                                "TRIM 完成，没有可回收的空间"
                            },
                        )
                    }
                }
                .onFailure { error ->
                    _state.update {
                        it.copy(actionRunning = null, error = error.message ?: "TRIM 失败")
                    }
                }
        }
    }

    /** Switch the analysis target; the previous result no longer describes the selected view. */
    fun setUsagePath(path: String) {
        _state.update { it.copy(usagePath = path, usage = null) }
    }

    fun analyzeUsage() {
        viewModelScope.launch {
            val path = _state.value.usagePath
            _state.update { it.copy(usageLoading = true, usage = null, error = null) }
            runCatching { client.storageUsage(path = path, limit = 20, budgetMs = 3000) }
                .onSuccess { json ->
                    _state.update {
                        it.copy(usage = parseStorageUsage(json), usageLoading = false)
                    }
                }
                .onFailure { error ->
                    _state.update {
                        it.copy(
                            usageLoading = false,
                            error = error.message ?: "分析目录占用失败",
                        )
                    }
                }
        }
    }

    fun dismissMessage() {
        _state.update { it.copy(message = null, error = null) }
    }

    companion object {
        /** User-visible internal storage: photos, downloads, app folders. */
        const val USER_STORAGE = "/storage/emulated/0"

        /** The whole data partition, including `/data/media` and every app's private data. */
        const val SYSTEM_DATA = "/data"

        /** The phone's root: mounts reported as partitions, everything else walked. */
        const val WHOLE_DEVICE = "/"
    }
}
