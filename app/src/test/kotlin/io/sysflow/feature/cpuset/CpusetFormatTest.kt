package io.sysflow.feature.cpuset

import io.sysflow.core.model.ItemType
import io.sysflow.core.model.Risk
import io.sysflow.core.model.Schema
import io.sysflow.core.model.SchemaItem
import io.sysflow.core.model.Scope
import io.sysflow.core.model.SnapshotData
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import org.junit.Assert.assertEquals
import org.junit.Test

class CpusetFormatTest {
    @Test
    fun formatsAdjacentCoresAsRanges() {
        assertEquals("0–3,6,8–9", formatCpuSet(listOf(9, 2, 3, 0, 1, 6, 8, 3)))
    }

    @Test
    fun formatsEmptySetAsDash() {
        assertEquals("—", formatCpuSet(emptyList()))
    }

    @Test
    fun processGroupsUseLegacyPriorityOrder() {
        val schema = Schema(
            module = "cpu",
            supported = true,
            items = listOf("background", "foreground", "top-app", "system-background").map { id ->
                SchemaItem(
                    id = "cpu.cpuset.$id",
                    type = ItemType.BoolList,
                    scope = Scope.Global,
                    risk = Risk.Medium,
                    persist = true,
                    count = 4,
                )
            },
        )
        val online = JsonArray(List(4) { JsonPrimitive(1) })
        val groupMask = JsonArray(List(4) { JsonPrimitive(1) })
        val snapshot = SnapshotData(
            ts = 1,
            safe_mode = false,
            modules = JsonObject(mapOf(
                "cpu" to JsonObject(mapOf(
                    "cpu.online" to online,
                    "cpu.clusters" to JsonObject(mapOf("0" to JsonArray(listOf(JsonPrimitive(0), JsonPrimitive(1))), "6" to JsonArray(listOf(JsonPrimitive(2), JsonPrimitive(3))))),
                    "cpu.cpuset.background" to groupMask,
                    "cpu.cpuset.foreground" to groupMask,
                    "cpu.cpuset.top-app" to groupMask,
                    "cpu.cpuset.system-background" to groupMask,
                )),
            )),
            settings = JsonObject(emptyMap()),
            defaults = JsonObject(emptyMap()),
            errors = emptyMap(),
        )
        val result = buildProcessGroupStates(schema, snapshot)
        assertEquals(listOf("top-app", "foreground", "system-background", "background"), result.groups.map { it.id })
    }
}
