@file:OptIn(ExperimentalMaterial3ExpressiveApi::class)

package io.sysflow.feature.home

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.rounded.ArrowForward
import androidx.compose.material.icons.rounded.AutoGraph
import androidx.compose.material3.ExperimentalMaterial3ExpressiveApi
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Shape
import androidx.compose.ui.unit.dp
import io.sysflow.core.ui.components.AppIdentity
import io.sysflow.core.ui.components.segmentedShape

/**
 * Battery-page entry to the ranking: the top three apps as connected tiles, closed off by a
 * "查看完整排行" tile that opens the dedicated page.
 *
 * System UIDs are left out here (Android System, screen, radio...) because the question this card
 * answers is "which app is draining me"; the full page still lists them. If nothing but system UIDs
 * exists the preview falls back to them rather than showing an empty card.
 */
@Composable
internal fun RankingPreview(
    state: BatteryAppsState,
    infos: Map<String, AppIdentity>,
    onOpenAll: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val page = state.page
    val apps = remember(state.entries) {
        state.entries.filterNot { it.isSystemUid }.ifEmpty { state.entries }.take(3)
    }

    when {
        state.loading -> MessageTile("正在读取 BatteryStats…", modifier, loading = true)
        state.failed -> MessageTile(
            "读取失败：无法获取 BatteryStats 数据",
            modifier,
            error = true,
            action = { TextButton(onClick = state.refresh) { Text("重试") } },
        )
        page?.available == false -> MessageTile(
            page.reason ?: "本机没有暴露 BatteryStats 的功耗估算",
            modifier,
        )
        apps.isEmpty() -> MessageTile("暂无耗电记录", modifier)
        else -> {
            val maxShare = apps.maxOf { it.share }
            Column(
                modifier = modifier.fillMaxWidth(),
                verticalArrangement = Arrangement.spacedBy(4.dp),
            ) {
                apps.forEachIndexed { index, entry ->
                    AppPowerItem(
                        entry = entry,
                        rank = index + 1,
                        info = infos[entry.uid] ?: AppIdentity(entry.name ?: entry.uid, null),
                        shape = segmentedShape(index, apps.size + 1),
                        relative = if (maxShare > 0.0) (entry.share / maxShare).toFloat() else 0f,
                    )
                }
                OpenAllTile(
                    count = page?.count ?: state.entries.size,
                    shape = segmentedShape(apps.size, apps.size + 1),
                    onClick = onOpenAll,
                )
            }
        }
    }
}

@Composable
private fun OpenAllTile(count: Int, shape: Shape, onClick: () -> Unit) {
    val scheme = MaterialTheme.colorScheme
    Surface(
        onClick = onClick,
        modifier = Modifier.fillMaxWidth(),
        shape = shape,
        color = scheme.secondaryContainer,
        contentColor = scheme.onSecondaryContainer,
    ) {
        Row(
            modifier = Modifier.padding(horizontal = 20.dp, vertical = 18.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            Icon(Icons.Rounded.AutoGraph, contentDescription = null, modifier = Modifier.size(22.dp))
            Text(
                "查看完整排行",
                modifier = Modifier.weight(1f),
                style = MaterialTheme.typography.titleMediumEmphasized,
            )
            Text(
                "共 $count 项",
                style = MaterialTheme.typography.labelMedium,
            )
            Icon(
                Icons.AutoMirrored.Rounded.ArrowForward,
                contentDescription = null,
                modifier = Modifier.size(20.dp),
            )
        }
    }
}
