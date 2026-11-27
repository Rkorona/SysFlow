package io.sysflow.feature.home

import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import io.sysflow.core.event.EventRepository
import io.sysflow.core.model.BatteryApps
import io.sysflow.core.model.BatteryHealth
import io.sysflow.core.model.BatteryHistory
import io.sysflow.core.event.SystemEventEntity
import io.sysflow.core.model.CapsData
import io.sysflow.core.model.SafeStatus
import io.sysflow.core.model.SnapshotData
import io.sysflow.core.root.SysFlowClient
import io.sysflow.core.root.SysFlowException
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.launch
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.intOrNull
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.jsonPrimitive

sealed interface ClientStatus {
    data object Loading : ClientStatus
    data object Ready : ClientStatus
    data object NoRoot : ClientStatus
    data object NoModule : ClientStatus
    data object Unsupported : ClientStatus
    data class Error(val message: String) : ClientStatus
}

data class DeviceInfoDisplay(
    val deviceModel: String?,
    val androidRelease: String?,
    val kernelRelease: String?,
)

data class AppUiState(
    val loading: Boolean = true,
    val status: ClientStatus = ClientStatus.Loading,
    val caps: CapsData? = null,
    val snapshot: SnapshotData? = null,
    val safe: SafeStatus? = null,
    val deviceInfo: DeviceInfoDisplay? = null,
    val activityEvents: List<SystemEventEntity> = emptyList(),
    val unsupportedReason: String? = null,
    val message: String? = null,
) {
    val cpuSupported: Boolean get() = caps?.modules?.get("cpu")?.supported == true
}

/** Dashboard state is display-only. Event records are imported from the Root module into Room. */
class AppViewModel(
    private val client: SysFlowClient,
    private val eventRepository: EventRepository,
) : ViewModel() {

    private val _state = MutableStateFlow(AppUiState())
    val state: StateFlow<AppUiState> = _state.asStateFlow()

    init {
        viewModelScope.launch {
            eventRepository.observeRecent(EventRepository.HOME_EVENT_LIMIT).collect { events ->
                _state.update { it.copy(activityEvents = events) }
            }
        }
        boot()
    }

    fun boot() {
        viewModelScope.launch {
            _state.update { it.copy(loading = true, status = ClientStatus.Loading, message = null) }
            if (!client.hasRoot()) {
                _state.update { it.copy(loading = false, status = ClientStatus.NoRoot) }
                return@launch
            }
            try {
                client.verifyProtocol()
                val caps = client.caps()
                val safe = client.safeStatus()
                val snapshot = client.snapshot()
                // Event sync is optional during upgrades: an older module binary can still serve
                // resource pages while the APK reports no imported journal records.
                val deviceInfo = runCatching { client.info().toDeviceInfoDisplay() }.getOrNull()
                val availableModules = caps.modules.filterValues { it.supported }
                val unsupportedReason = caps.modules.values.firstOrNull { !it.supported }?.reason
                _state.update {
                    it.copy(
                        loading = false,
                        status = if (availableModules.isNotEmpty()) ClientStatus.Ready else ClientStatus.Unsupported,
                        caps = caps,
                        snapshot = snapshot,
                        safe = safe,
                        deviceInfo = deviceInfo,
                        unsupportedReason = if (availableModules.isEmpty()) unsupportedReason else null,
                    )
                }
                // Backfill Room after the dashboard is already ready; do not hold first paint.
                viewModelScope.launch { runCatching { eventRepository.syncLatest(force = true) } }
            } catch (e: SysFlowException) {
                _state.update { it.copy(loading = false, status = e.toStatus(), message = e.message) }
            } catch (e: Exception) {
                _state.update {
                    it.copy(loading = false, status = ClientStatus.Error(e.message ?: "未知错误"), message = e.message)
                }
            }
        }
    }

    fun refresh() {
        viewModelScope.launch {
            try {
                val snapshot = client.snapshot()
                val safe = client.safeStatus()
                _state.update { it.copy(snapshot = snapshot, safe = safe) }
                // Module-side capture continues independently of the APK. The APK only performs
                // a throttled foreground pull to refresh its local Room read model.
                runCatching { eventRepository.syncLatest() }
            } catch (_: Exception) {
                // Keep the last good dashboard and Room data when the module is temporarily busy.
            }
        }
    }

    /** Recorded history (24 h, hourly buckets) for the battery curve. */
    suspend fun loadBatteryHistory(): BatteryHistory? = runCatching {
        BatteryHistory.from(client.history(module = "battery", since = "24h", step = "1h"))
    }.getOrNull()

    /** Cycle count/health with the vendor fallback; cached module-side for a minute. */
    suspend fun loadBatteryHealth(refresh: Boolean = false): BatteryHealth? = runCatching {
        BatteryHealth.from(client.batteryHealth(refresh))
    }.getOrNull()

    /**
     * BatteryStats per-UID attribution. Deliberately not part of [refresh]: it runs an expensive
     * dumpsys behind a module-side cache, so only the battery page asks for it.
     */
    suspend fun loadBatteryApps(refresh: Boolean = false): BatteryApps? = runCatching {
        BatteryApps.from(client.batteryApps(limit = 30, refresh = refresh))
    }.getOrNull()

    /**
     * Delete all events: the Root journal first, then the local Room mirror.
     *
     * Both halves are required. The module owns the journal, but the APK renders its Room cache, so
     * clearing only one of them leaves the list showing records that no longer have a source.
     */
    suspend fun clearEvents(): ClearEventsResult {
        // Three failure modes must not collapse into one message: the CLI can fail, the Room cache can
        // fail, and the read-back can fail. Blaming "无法连接模块" for a local cache problem sends the
        // user looking at the wrong layer.
        val payload = try {
            client.clearEvents()
        } catch (cause: Exception) {
            return ClearEventsResult(
                cleared = null,
                journalRemaining = null,
                localDeleted = 0,
                localRemaining = null,
                probeError = null,
                error = cause.message ?: cause::class.simpleName ?: "模块返回错误",
            )
        }
        val cleared = (payload["cleared"] ?: payload["removed"])?.jsonPrimitive?.intOrNull ?: 0
        val directory = payload["directory"]?.jsonPrimitive?.contentOrNull
        val localDeleted = try {
            eventRepository.clearLocalCache()
        } catch (cause: Exception) {
            return ClearEventsResult(
                cleared = cleared,
                journalRemaining = null,
                localDeleted = 0,
                localRemaining = null,
                probeError = null,
                error = "期刊已删除 $cleared 条，但本地缓存清理失败：${cause.message ?: cause::class.simpleName}",
                directory = directory,
            )
        }
        // Ask the journal directly how many records survive, through the same CLI that deleted them.
        // A non-zero answer is normal — the daemon keeps writing — so the message explains it rather
        // than implying failure.
        val journalRemaining = runCatching { client.events(limit = 200).events.size }.getOrNull()
        // Push the empty state immediately instead of waiting for the Room flow; a collector that died
        // would otherwise leave the deleted rows on screen.
        _state.update { it.copy(activityEvents = emptyList()) }
        // Read the mirror back so the result is self-verifying: rows surviving the DELETE mean the
        // delete is at fault, while an empty table next to a populated screen means the observation is.
        var probeError: String? = null
        val remaining = runCatching {
            eventRepository.observeRecent(100).first().size
        }.onFailure { probeError = it.message ?: it::class.simpleName }.getOrNull()
        // No immediate syncLatest() here: it would re-import whatever the daemon wrote a second ago and
        // make the list look untouched. The periodic sync picks genuinely new records up on its own.
        return ClearEventsResult(
            cleared = cleared,
            journalRemaining = journalRemaining,
            localDeleted = localDeleted,
            localRemaining = remaining,
            probeError = probeError,
            error = null,
            directory = directory,
        )
    }

    /**
     * Read-only CLI transcript for the events diagnostics panel.
     *
     * Deliberately non-destructive: it used to clear the journal as part of the transcript, which
     * made a debugging tap delete the user's data. It answers the one question that matters when the
     * app and a terminal disagree — which journal does this process read, and what does it see — by
     * printing the directory the CLI itself resolved, the newest records, and the oldest record.
     */
    suspend fun eventsDiagnostics(): String = buildString {
        appendLine(client.raw(listOf("version")))
        appendLine()
        appendLine("— 最新 3 条 —")
        appendLine(client.raw(listOf("event", "list", "--limit", "3")))
        appendLine()
        appendLine("— 最早 1 条 —")
        appendLine(client.raw(listOf("event", "list", "--limit", "1", "--ascending")))
        appendLine()
        val local = runCatching { eventRepository.observeRecent(100).first().size }
        append("本地 Room 行数：")
        append(local.getOrElse { "读取失败（${it.message ?: it::class.simpleName}）" })
    }

    fun syncEvents(force: Boolean = false) {
        viewModelScope.launch {
            runCatching { eventRepository.syncLatest(force = force) }
        }
    }

    fun clearSafeMode() {
        viewModelScope.launch {
            try {
                val safe = client.safeClear()
                _state.update { it.copy(safe = safe, message = "已退出安全模式") }
                runCatching { eventRepository.syncLatest(force = true) }
                refresh()
            } catch (e: SysFlowException) {
                _state.update { it.copy(message = e.message) }
            }
        }
    }

    fun dismissMessage() {
        _state.update { it.copy(message = null) }
    }

    private fun JsonObject.toDeviceInfoDisplay() = DeviceInfoDisplay(
        deviceModel = this["device_model"]?.jsonPrimitive?.contentOrNull,
        androidRelease = this["android_release"]?.jsonPrimitive?.contentOrNull,
        kernelRelease = this["kernel_release"]?.jsonPrimitive?.contentOrNull,
    )

    private fun SysFlowException.toStatus(): ClientStatus = when (this) {
        is SysFlowException.NoBinary -> ClientStatus.NoModule
        is SysFlowException.Permission -> ClientStatus.NoRoot
        is SysFlowException.Unsupported -> ClientStatus.Unsupported
        else -> ClientStatus.Error(message ?: "读取失败")
    }
}

/** Outcome of clearing all events; [error] is shown verbatim so the failing layer is obvious. */
data class ClearEventsResult(
    val cleared: Int?,
    /** Records the journal still reports right after the delete, read through the same CLI. */
    val journalRemaining: Int?,
    /** Rows removed from the APK's Room mirror; 0 means the cache was already empty. */
    val localDeleted: Int,
    /** Rows still in the mirror right after the delete; null when the read-back failed. */
    val localRemaining: Int?,
    /** Why the read-back failed, when it did. */
    val probeError: String?,
    val error: String?,
    /** Journal directory the CLI itself reported clearing; proves app and terminal agree on a path. */
    val directory: String? = null,
)
