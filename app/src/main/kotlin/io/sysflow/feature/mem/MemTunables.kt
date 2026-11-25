@file:OptIn(ExperimentalMaterial3ExpressiveApi::class)

package io.sysflow.feature.mem

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.Info
import androidx.compose.material.icons.rounded.RestartAlt
import androidx.compose.material.icons.rounded.Save
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.ExperimentalMaterial3ExpressiveApi
import androidx.compose.material3.FilledTonalButton
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Slider
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import io.sysflow.core.ui.components.InfoPill
import kotlin.math.roundToInt

/**
 * The writable half: two sysctls, sliders bounded by the module's own schema.
 *
 * Everything is opt-in — moving a slider changes nothing until 保存 — because these write to
 * `/proc/sys/vm` and a stray touch should not retune the device. A slider that differs from the
 * live value says so on its own pill ("60 → 100"), so an unsaved change is visible per control and
 * not only through the section badge.
 */
@Composable
internal fun TunablesSection(
    state: MemViewModel.UiState,
    onDraft: (String, Int) -> Unit,
    onSave: () -> Unit,
    onRestore: () -> Unit,
    onDismissMessage: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val snapshot = state.snapshot
    val scheme = MaterialTheme.colorScheme
    val dirty = state.draft.isNotEmpty()

    Column(modifier.fillMaxWidth(), verticalArrangement = Arrangement.spacedBy(12.dp)) {
        SectionHeader(
            "可调参数",
            supporting = "写入 /proc/sys/vm，点「保存」后才生效",
            action = {
                if (dirty) {
                    InfoPill(
                        text = "未保存",
                        container = scheme.tertiaryContainer,
                        content = scheme.onTertiaryContainer,
                    )
                }
            },
        )
        SectionSurface {
            Column(
                modifier = Modifier.padding(20.dp),
                verticalArrangement = Arrangement.spacedBy(24.dp),
            ) {
                state.ranges.forEach { (id, range) ->
                    val live = when (id) {
                        "mem.swappiness" -> snapshot?.swappiness
                        "mem.vfs_cache_pressure" -> snapshot?.vfsCachePressure
                        else -> null
                    }
                    TunableControl(
                        id = id,
                        range = range,
                        live = live,
                        value = state.draft[id] ?: live ?: range.min,
                        onValueChange = { onDraft(id, it) },
                    )
                }
                if (snapshot?.mglru == true) {
                    NoteTile(
                        "本机启用了 MGLRU（多代 LRU）：swappiness 的效果会被稀释，调整后不一定有可感知变化。",
                        icon = Icons.Rounded.Info,
                        container = scheme.primaryContainer,
                        content = scheme.onPrimaryContainer,
                    )
                }
                Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    FilledTonalButton(
                        onClick = onRestore,
                        enabled = !state.saving,
                        modifier = Modifier.weight(1f),
                        shapes = ButtonDefaults.shapes(),
                    ) {
                        Icon(
                            Icons.Rounded.RestartAlt,
                            contentDescription = null,
                            modifier = Modifier.size(ButtonDefaults.IconSize),
                        )
                        Spacer(Modifier.size(ButtonDefaults.IconSpacing))
                        Text("恢复默认", maxLines = 1)
                    }
                    Button(
                        onClick = onSave,
                        enabled = !state.saving && dirty,
                        modifier = Modifier.weight(1f),
                        shapes = ButtonDefaults.shapes(),
                    ) {
                        Icon(
                            Icons.Rounded.Save,
                            contentDescription = null,
                            modifier = Modifier.size(ButtonDefaults.IconSize),
                        )
                        Spacer(Modifier.size(ButtonDefaults.IconSpacing))
                        Text(if (state.saving) "保存中…" else "保存", maxLines = 1)
                    }
                }
                state.message?.let { message ->
                    NoteTile(
                        message,
                        container = scheme.secondaryContainer,
                        content = scheme.onSecondaryContainer,
                        onDismiss = onDismissMessage,
                    )
                }
                Text(
                    "保存后写入 /proc/sys/vm 并持久化；开机由模块回放，若被系统反复改回，模块会停手并在事件里说明。",
                    style = MaterialTheme.typography.bodySmall,
                    color = scheme.onSurfaceVariant,
                )
            }
        }
    }
}

@Composable
private fun TunableControl(
    id: String,
    range: MemViewModel.Range,
    live: Int?,
    value: Int,
    onValueChange: (Int) -> Unit,
) {
    val scheme = MaterialTheme.colorScheme
    val changed = live != null && value != live
    Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
        Row(
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            Column(modifier = Modifier.weight(1f)) {
                Text(tunableTitle(id), style = MaterialTheme.typography.titleMedium)
                Text(
                    tunableKey(id),
                    style = MaterialTheme.typography.labelMedium,
                    color = scheme.onSurfaceVariant,
                )
            }
            Surface(
                shape = CircleShape,
                color = if (changed) scheme.primaryContainer else scheme.secondaryContainer,
                contentColor = if (changed) scheme.onPrimaryContainer else scheme.onSecondaryContainer,
            ) {
                Text(
                    if (changed) "$live → $value" else "$value",
                    modifier = Modifier.padding(horizontal = 16.dp, vertical = 8.dp),
                    style = MaterialTheme.typography.titleMediumEmphasized,
                    maxLines = 1,
                )
            }
        }
        if (range.max > range.min) {
            // Continuous track, integer value: `onValueChange` rounds, so the thumb snaps without
            // a tick mark for each of the (up to 200) positions.
            Slider(
                value = value.toFloat(),
                onValueChange = { onValueChange(it.roundToInt()) },
                valueRange = range.min.toFloat()..range.max.toFloat(),
            )
            Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.SpaceBetween) {
                Text(
                    "${range.min}",
                    style = MaterialTheme.typography.labelSmall,
                    color = scheme.onSurfaceVariant,
                )
                Text(
                    "${range.max}",
                    style = MaterialTheme.typography.labelSmall,
                    color = scheme.onSurfaceVariant,
                )
            }
        }
        Text(
            tunableHint(id),
            style = MaterialTheme.typography.bodySmall,
            color = scheme.onSurfaceVariant,
        )
    }
}

private fun tunableTitle(id: String): String = when (id) {
    "mem.swappiness" -> "交换倾向"
    "mem.vfs_cache_pressure" -> "缓存回收压力"
    else -> id
}

private fun tunableKey(id: String): String = when (id) {
    "mem.swappiness" -> "swappiness"
    "mem.vfs_cache_pressure" -> "vfs_cache_pressure"
    else -> "sysctl"
}

private fun tunableHint(id: String): String = when (id) {
    "mem.swappiness" -> "越低越不愿换出匿名页；zRAM 设备上过低会让压缩内存闲置。"
    "mem.vfs_cache_pressure" -> "越高越积极回收 dentry/inode 缓存。"
    else -> ""
}
