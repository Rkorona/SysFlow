@file:OptIn(ExperimentalMaterial3ExpressiveApi::class)

package io.sysflow.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.Bolt
import androidx.compose.material.icons.rounded.Lock
import androidx.compose.material.icons.rounded.Refresh
import androidx.compose.material.icons.rounded.Warning
import androidx.compose.material3.Button
import androidx.compose.material3.ExperimentalMaterial3ExpressiveApi
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialShapes
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.toShape
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import io.sysflow.core.ui.components.ExpressiveCard

enum class GuideKind { NoRoot, NoModule, Unsupported, Error }

@Composable
fun GuideScreen(
    kind: GuideKind,
    version: String?,
    reason: String?,
    onRetry: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val icon: ImageVector
    val title: String
    val body: String
    when (kind) {
        GuideKind.NoRoot -> {
            icon = Icons.Rounded.Lock
            title = "需要 root 授权"
            body = "SysFlow 通过 root 命令读写 CPU 状态。请在 KernelSU / APatch 中授权后重试。"
        }
        GuideKind.NoModule -> {
            icon = Icons.Rounded.Warning
            title = "没有找到 sysflow 二进制"
            body = "请先安装 SysFlow 模块，确认 /data/adb/modules/sysflow/bin/sysflow 存在，然后重启设备。"
        }
        GuideKind.Unsupported -> {
            icon = Icons.Rounded.Bolt
            title = "当前设备不支持 CPU 调节"
            body = "没有探测到可用的 cpufreq 节点。可能是内核限制、SELinux 或机型差异。"
        }
        GuideKind.Error -> {
            icon = Icons.Rounded.Warning
            title = "读取失败"
            body = "请检查 root、模块状态和日志后重试。"
        }
    }
    Box(modifier.fillMaxSize().padding(24.dp), contentAlignment = Alignment.Center) {
        ExpressiveCard {
            Column(
                modifier = Modifier.fillMaxWidth(),
                horizontalAlignment = Alignment.CenterHorizontally,
                verticalArrangement = Arrangement.spacedBy(14.dp),
            ) {
                Box(
                    modifier = Modifier
                        .size(96.dp)
                        .clip(MaterialShapes.Cookie9Sided.toShape())
                        .background(MaterialTheme.colorScheme.primaryContainer),
                    contentAlignment = Alignment.Center,
                ) {
                    Icon(icon, contentDescription = null, modifier = Modifier.size(44.dp), tint = MaterialTheme.colorScheme.onPrimaryContainer)
                }
                Text(title, style = MaterialTheme.typography.headlineSmallEmphasized, textAlign = TextAlign.Center)
                Text(
                    body,
                    style = MaterialTheme.typography.bodyMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    textAlign = TextAlign.Center,
                )
                if (version != null) {
                    Text("模块版本 $version", style = MaterialTheme.typography.labelMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
                }
                if (!reason.isNullOrBlank()) {
                    Text(reason, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.error, textAlign = TextAlign.Center)
                }
                Button(onClick = onRetry) {
                    Icon(Icons.Rounded.Refresh, contentDescription = null)
                    Text("重试", modifier = Modifier.padding(start = 8.dp))
                }
            }
        }
    }
}
