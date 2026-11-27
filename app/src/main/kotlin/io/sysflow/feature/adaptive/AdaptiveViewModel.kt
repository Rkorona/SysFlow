package io.sysflow.feature.adaptive

import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import io.sysflow.core.model.CpuLiveMetrics
import io.sysflow.core.model.CpuLiveTracker
import io.sysflow.core.model.CpuSnapshot
import io.sysflow.core.root.SysFlowClient
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.booleanOrNull
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.intOrNull
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.put

private const val CPU_ADAPTIVE = "cpu.adaptive"

data class AdaptiveSensor(
    val name: String,
    val temperatureDeciC: Int,
)

data class AdaptivePolicy(
    val id: String,
    val tier: Int,
    val load: Int?,
    val downRounds: Int,
)

data class AdaptiveRuntime(
    val enabled: Boolean = false,
    val guardEnabled: Boolean = false,
    val guardRunning: Boolean = false,
    val guardIntervalSeconds: Int = 10,
    val cpuAdjustment: Int = 100,
    val gpuAdjustment: Int = 100,
    val psiFloor: Int = 0,
    val maxTemperature: Int? = null,
    val temperatureSensor: String? = null,
    val batteryPercent: Int? = null,
    val temperatureCut: Int = 0,
    val batteryCut: Int = 0,
    val policies: List<AdaptivePolicy> = emptyList(),
)

data class AdaptiveUiState(
    val loading: Boolean = true,
    val pending: Boolean = false,
    val config: Map<String, Int> = emptyMap(),
    val drafts: Map<String, String> = emptyMap(),
    val dirtyGroups: Set<String> = emptySet(),
    val live: CpuLiveMetrics = CpuLiveMetrics(),
    val gpuBusy: Int? = null,
    val batteryTempDeciC: Int? = null,
    val sensors: List<AdaptiveSensor> = emptyList(),
    val runtime: AdaptiveRuntime = AdaptiveRuntime(),
    val message: String? = null,
)

class AdaptiveViewModel(private val client: SysFlowClient) : ViewModel() {
    private val _state = MutableStateFlow(AdaptiveUiState())
    val state: StateFlow<AdaptiveUiState> = _state.asStateFlow()
    private var previousSnapshot: CpuSnapshot? = null

    fun load() = refresh(showLoading = true)

    fun refresh(showLoading: Boolean = false) {
        viewModelScope.launch {
            if (showLoading) _state.update { it.copy(loading = true, message = null) }
            try {
                val envelope = client.snapshot()
                val daemon = client.daemonStatus()
                val cpuJson = envelope.modules["cpu"] as? JsonObject
                    ?: throw IllegalStateException("CPU 模块没有返回数据")
                val cpu = CpuSnapshot.from(cpuJson)
                val previous = previousSnapshot
                val live = if (cpu != null) CpuLiveTracker.update(previous, cpu) else CpuLiveMetrics()
                if (cpu != null) previousSnapshot = cpu
                val values = readConfig(cpuJson)
                val sensors = readSensors(cpuJson["cpu.thermal_sensors"] as? JsonArray)
                val runtime = readRuntime(cpuJson["cpu.adaptive_state"] as? JsonObject).copy(
                    guardEnabled = daemon["enabled"]?.jsonPrimitive?.booleanOrNull ?: false,
                    guardRunning = daemon["running"]?.jsonPrimitive?.booleanOrNull ?: false,
                    guardIntervalSeconds = daemon["interval_s"]?.jsonPrimitive?.intOrNull ?: 10,
                )
                val gpuBusy = (cpuJson["cpu.gpu_busy"] as? JsonPrimitive)?.intOrNull
                val batteryTemp = (cpuJson["cpu.battery_temp"] as? JsonPrimitive)?.intOrNull
                _state.update { old ->
                    val nextDrafts = old.drafts.toMutableMap()
                    val nextDirty = old.dirtyGroups.toMutableSet()
                    for (key in ADAPTIVE_VALUE_KEYS) {
                        val group = groupForKey(key)
                        if (group == null || group !in nextDirty) {
                            values[key]?.let { nextDrafts[key] = it.toString() }
                        }
                    }
                    old.copy(
                        loading = false,
                        config = values,
                        drafts = nextDrafts,
                        dirtyGroups = nextDirty,
                        live = live,
                        gpuBusy = gpuBusy,
                        batteryTempDeciC = batteryTemp,
                        sensors = sensors,
                        runtime = runtime,
                    )
                }
            } catch (e: Exception) {
                _state.update { it.copy(loading = false, message = e.message ?: "读取自适应配置失败") }
            }
        }
    }

    fun toggle(key: String, enabled: Boolean) {
        if (key !in ADAPTIVE_BOOL_KEYS) return
        update(buildJsonObject { put(key, enabled) }, "自适应策略已更新")
    }

    fun edit(key: String, value: String) {
        val group = groupForKey(key) ?: return
        _state.update { current ->
            val drafts = current.drafts + (key to value.filter(Char::isDigit).take(4))
            val fields = GROUP_FIELDS.getValue(group)
            val dirty = fields.any { drafts[it]?.toIntOrNull() != current.config[it] }
            current.copy(
                drafts = drafts,
                dirtyGroups = if (dirty) current.dirtyGroups + group else current.dirtyGroups - group,
            )
        }
    }

    fun saveGroup(group: String) {
        val fields = GROUP_FIELDS[group] ?: return
        val current = _state.value
        val numbers = fields.associateWith { current.drafts[it]?.toIntOrNull() }
        val invalid = fields.firstOrNull { key ->
            val value = numbers[key] ?: return@firstOrNull true
            val range = FIELD_RANGES.getValue(key)
            value !in range.first..range.last
        }
        if (invalid != null) {
            _state.update { it.copy(message = "$invalid 参数超出允许范围") }
            return
        }
        val values = numbers.mapValues { it.value!! }
        val crossError = when (group) {
            "smart" -> if (!(values.getValue("cpu.adaptive.load_t1") < values.getValue("cpu.adaptive.load_t2") &&
                    values.getValue("cpu.adaptive.load_t2") < values.getValue("cpu.adaptive.load_t3"))) {
                "负载阈值必须严格递增"
            } else if (!(values.getValue("cpu.adaptive.load_cap1") <= values.getValue("cpu.adaptive.load_cap2") &&
                    values.getValue("cpu.adaptive.load_cap2") <= values.getValue("cpu.adaptive.load_cap3") &&
                    values.getValue("cpu.adaptive.load_cap3") <= values.getValue("cpu.adaptive.load_cap4"))) {
                "四档负载上限必须保持不递减"
            } else null
            "thermal" -> if (values.getValue("cpu.adaptive.temp_low") >= values.getValue("cpu.adaptive.temp_high")) {
                "开始恢复温度必须低于开始收紧温度"
            } else null
            "battery" -> if (values.getValue("cpu.adaptive.batt_critical") >= values.getValue("cpu.adaptive.batt_low")) {
                "临界电量阈值必须低于低电量阈值"
            } else null
            else -> null
        }
        if (crossError != null) {
            _state.update { it.copy(message = crossError) }
            return
        }
        update(buildJsonObject { values.forEach { (key, value) -> put(key, value) } }, "参数已保存", cleanGroup = group)
    }

    fun resetDefaults() {
        update(buildJsonObject {
            put("cpu.adaptive", false)
            put("cpu.adaptive.smart", true)
            put("cpu.adaptive.thermal", false)
            put("cpu.adaptive.battery", false)
            put("cpu.adaptive.psi", false)
            DEFAULT_CONFIG.filterKeys { it != "cpu.adaptive" && it !in ADAPTIVE_BOOL_KEYS }
                .forEach { (key, value) -> put(key, value) }
        }, "自适应配置已恢复默认值", resetDrafts = true)
    }

    fun dismissMessage() = _state.update { it.copy(message = null) }

    private fun update(
        payload: JsonObject,
        success: String,
        cleanGroup: String? = null,
        resetDrafts: Boolean = false,
    ) {
        viewModelScope.launch {
            _state.update { it.copy(pending = true, message = null) }
            try {
                client.set("cpu", payload, persist = true)
                _state.update { old ->
                    val drafts = old.drafts.toMutableMap()
                    if (resetDrafts) {
                        DEFAULT_CONFIG.forEach { (key, value) -> drafts[key] = value.toString() }
                    } else if (cleanGroup != null) {
                        GROUP_FIELDS.getValue(cleanGroup).forEach { key ->
                            drafts[key] = old.drafts[key].orEmpty()
                        }
                    }
                    old.copy(
                        pending = false,
                        dirtyGroups = if (resetDrafts) emptySet() else cleanGroup?.let { old.dirtyGroups - it } ?: old.dirtyGroups,
                        drafts = drafts,
                        message = success,
                    )
                }
                previousSnapshot = null
                refresh()
            } catch (e: Exception) {
                _state.update { it.copy(pending = false, message = e.message ?: "操作失败") }
            }
        }
    }

    private fun readConfig(cpu: JsonObject): Map<String, Int> = buildMap {
        DEFAULT_CONFIG.forEach { (key, default) ->
            val value = cpu[key]?.jsonPrimitive
            val parsed = value?.booleanOrNull?.let { if (it) 1 else 0 } ?: value?.intOrNull
            put(key, parsed ?: default)
        }
    }

    private fun readRuntime(obj: JsonObject?): AdaptiveRuntime {
        if (obj == null) return AdaptiveRuntime()
        val policies = (obj["policies"] as? JsonObject)?.map { (id, value) ->
            val p = value.jsonObject
            AdaptivePolicy(
                id = id,
                tier = p["tier"]?.jsonPrimitive?.intOrNull ?: 0,
                load = p["load"]?.jsonPrimitive?.intOrNull?.takeIf { it >= 0 },
                downRounds = p["down_rounds"]?.jsonPrimitive?.intOrNull ?: 0,
            )
        }?.sortedBy { it.id.toIntOrNull() ?: Int.MAX_VALUE }.orEmpty()
        return AdaptiveRuntime(
            enabled = obj["enabled"]?.jsonPrimitive?.booleanOrNull ?: false,
            cpuAdjustment = obj["cpu_adj"]?.jsonPrimitive?.intOrNull ?: 100,
            gpuAdjustment = obj["gpu_adj"]?.jsonPrimitive?.intOrNull ?: 100,
            psiFloor = obj["psi_floor"]?.jsonPrimitive?.intOrNull ?: 0,
            maxTemperature = obj["max_temp"]?.jsonPrimitive?.intOrNull?.takeIf { it >= 0 },
            temperatureSensor = (obj["temp_sensor"] as? JsonPrimitive)?.content
                ?.takeIf { it.isNotBlank() && it != "null" },
            batteryPercent = obj["battery_cap"]?.jsonPrimitive?.intOrNull?.takeIf { it >= 0 },
            temperatureCut = obj["temp_cut"]?.jsonPrimitive?.intOrNull ?: 0,
            batteryCut = obj["battery_cut"]?.jsonPrimitive?.intOrNull ?: 0,
            policies = policies,
        )
    }

    private fun readSensors(array: JsonArray?): List<AdaptiveSensor> = array.orEmpty().mapNotNull { element ->
        val obj = element as? JsonObject ?: return@mapNotNull null
        val name = obj["sensor"]?.jsonPrimitive?.content ?: return@mapNotNull null
        val temp = obj["temp_deci_c"]?.jsonPrimitive?.intOrNull ?: return@mapNotNull null
        AdaptiveSensor(name, temp)
    }

    private fun groupForKey(key: String): String? = GROUP_FIELDS.entries.firstOrNull { key in it.value }?.key

    companion object {
        val ADAPTIVE_BOOL_KEYS = setOf(
            "cpu.adaptive", "cpu.adaptive.smart", "cpu.adaptive.thermal",
            "cpu.adaptive.battery", "cpu.adaptive.psi",
        )
        val GROUP_FIELDS = mapOf(
            "smart" to listOf(
                "cpu.adaptive.load_cap1", "cpu.adaptive.load_cap2", "cpu.adaptive.load_cap3", "cpu.adaptive.load_cap4",
                "cpu.adaptive.load_t1", "cpu.adaptive.load_t2", "cpu.adaptive.load_t3", "cpu.adaptive.load_down_rounds",
            ),
            "thermal" to listOf(
                "cpu.adaptive.temp_high", "cpu.adaptive.temp_low", "cpu.adaptive.temp_step",
                "cpu.adaptive.temp_max", "cpu.adaptive.temp_release_rounds",
            ),
            "battery" to listOf(
                "cpu.adaptive.batt_low", "cpu.adaptive.batt_critical", "cpu.adaptive.batt_low_cut",
                "cpu.adaptive.batt_critical_cut", "cpu.adaptive.batt_hyst",
            ),
            "psi" to listOf("cpu.adaptive.psi_threshold", "cpu.adaptive.psi_cap"),
        )
        val ADAPTIVE_VALUE_KEYS = GROUP_FIELDS.values.flatten().toSet()
        private val FIELD_RANGES = mapOf(
            "cpu.adaptive.load_cap1" to 20..100, "cpu.adaptive.load_cap2" to 20..100,
            "cpu.adaptive.load_cap3" to 20..100, "cpu.adaptive.load_cap4" to 20..100,
            "cpu.adaptive.load_t1" to 1..98, "cpu.adaptive.load_t2" to 2..99,
            "cpu.adaptive.load_t3" to 3..100, "cpu.adaptive.load_down_rounds" to 1..10,
            "cpu.adaptive.temp_high" to 40..100, "cpu.adaptive.temp_low" to 30..99,
            "cpu.adaptive.temp_step" to 1..20, "cpu.adaptive.temp_max" to 0..40,
            "cpu.adaptive.temp_release_rounds" to 1..10,
            "cpu.adaptive.batt_low" to 1..50, "cpu.adaptive.batt_critical" to 1..49,
            "cpu.adaptive.batt_low_cut" to 0..30, "cpu.adaptive.batt_critical_cut" to 0..30,
            "cpu.adaptive.batt_hyst" to 0..20, "cpu.adaptive.psi_threshold" to 1..100,
            "cpu.adaptive.psi_cap" to 20..100,
        )
        val DEFAULT_CONFIG = mapOf(
            "cpu.adaptive" to 0, "cpu.adaptive.smart" to 1, "cpu.adaptive.thermal" to 0,
            "cpu.adaptive.battery" to 0, "cpu.adaptive.psi" to 0,
            "cpu.adaptive.load_t1" to 12, "cpu.adaptive.load_t2" to 35, "cpu.adaptive.load_t3" to 70,
            "cpu.adaptive.load_cap1" to 45, "cpu.adaptive.load_cap2" to 65,
            "cpu.adaptive.load_cap3" to 85, "cpu.adaptive.load_cap4" to 100,
            "cpu.adaptive.load_down_rounds" to 2,
            "cpu.adaptive.temp_high" to 75, "cpu.adaptive.temp_low" to 68,
            "cpu.adaptive.temp_step" to 5, "cpu.adaptive.temp_max" to 20,
            "cpu.adaptive.temp_release_rounds" to 2,
            "cpu.adaptive.batt_low" to 20, "cpu.adaptive.batt_critical" to 10,
            "cpu.adaptive.batt_low_cut" to 5, "cpu.adaptive.batt_critical_cut" to 5,
            "cpu.adaptive.batt_hyst" to 2,
            "cpu.adaptive.psi_threshold" to 8, "cpu.adaptive.psi_cap" to 85,
        )
    }
}
