@file:OptIn(ExperimentalMaterial3ExpressiveApi::class)

package io.sysflow.feature.storage

import androidx.compose.animation.AnimatedVisibility
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.asPaddingValues
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.navigationBars
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.DeleteForever
import androidx.compose.material.icons.rounded.ErrorOutline
import androidx.compose.material.icons.rounded.Info
import androidx.compose.material.icons.rounded.Storage
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.ExperimentalMaterial3ExpressiveApi
import androidx.compose.material3.Icon
import androidx.compose.material3.LoadingIndicator
import androidx.compose.material3.MaterialShapes
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.toShape
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.unit.dp
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.compose.LocalLifecycleOwner
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.repeatOnLifecycle
import kotlinx.coroutines.delay
import kotlinx.coroutines.isActive
import org.koin.androidx.compose.koinViewModel

/**
 * Storage page: how full the partition is, how fast it is moving data, which queue policy is active,
 * and the two maintenance actions (TRIM and a bounded directory scan).
 *
 * This file only hosts the page — polling, the loading / empty states, the feedback banner and the
 * confirmation dialog. The sections live in `StorageSections.kt`.
 *
 * `SysFlowApp` wraps this route in `FlexibleSubPage`, so no page heading is drawn here. The page
 * polls only while STARTED: a snapshot + one `storage io` reading every two seconds. Nothing runs in
 * the background.
 */
@Composable
fun StorageScreen(viewModel: StorageViewModel = koinViewModel()) {
    val state by viewModel.state.collectAsStateWithLifecycle()
    val lifecycleOwner = LocalLifecycleOwner.current
    val bottomInset = WindowInsets.navigationBars.asPaddingValues().calculateBottomPadding()
    var confirm by remember { mutableStateOf<StorageConfirm?>(null) }

    LaunchedEffect(Unit) {
        lifecycleOwner.repeatOnLifecycle(Lifecycle.State.STARTED) {
            while (isActive) {
                viewModel.refresh()
                viewModel.pollIo()
                delay(2_000)
            }
        }
    }

    val snapshot = state.snapshot

    Column(Modifier.fillMaxSize()) {
        // Pinned above the list, so the result of TRIM / a setting stays visible wherever you scrolled.
        val feedback = state.error ?: state.message
        AnimatedVisibility(visible = feedback != null && snapshot != null) {
            val danger = state.error != null
            val scheme = MaterialTheme.colorScheme
            NoteTile(
                text = feedback.orEmpty(),
                icon = if (danger) Icons.Rounded.ErrorOutline else Icons.Rounded.Info,
                container = if (danger) scheme.errorContainer else scheme.secondaryContainer,
                content = if (danger) scheme.onErrorContainer else scheme.onSecondaryContainer,
                modifier = Modifier.padding(horizontal = 16.dp, vertical = 8.dp),
                onDismiss = viewModel::dismissMessage,
            )
        }
        Box(Modifier.weight(1f)) {
            when {
                snapshot == null && state.loading -> Box(Modifier.fillMaxSize(), Alignment.Center) {
                    LoadingIndicator()
                }
                snapshot == null -> Box(Modifier.fillMaxSize(), Alignment.Center) {
                    EmptyActionTile(
                        icon = Icons.Rounded.Storage,
                        text = state.error ?: "设备没有提供 /data 容量信息，或模块尚未加载。",
                        actionLabel = "重试",
                        onAction = viewModel::refresh,
                        modifier = Modifier.padding(16.dp),
                    )
                }
                else -> LazyColumn(
                    modifier = Modifier.fillMaxSize(),
                    contentPadding = PaddingValues(
                        start = 16.dp,
                        end = 16.dp,
                        top = 8.dp,
                        bottom = 24.dp + bottomInset,
                    ),
                    verticalArrangement = Arrangement.spacedBy(16.dp),
                ) {
                    item(key = "hero") { CapacityHero(snapshot) }
                    snapshot.sdcard?.let { sdcard ->
                        item(key = "sdcard") { SdcardSection(sdcard) }
                    }
                    item(key = "io") {
                        IoSection(
                            io = state.io,
                            loading = state.ioLoading,
                            onRefresh = viewModel::pollIo,
                        )
                    }
                    item(key = "history") {
                        HistorySection(
                            history = state.history,
                            loading = state.historyLoading,
                            onRefresh = viewModel::loadHistory,
                        )
                    }
                    item(key = "device") {
                        DeviceSection(
                            state = state,
                            onScheduler = viewModel::setScheduler,
                            onReadAhead = viewModel::setReadAhead,
                        )
                    }
                    item(key = "maintenance") {
                        MaintenanceSection(
                            state = state,
                            onTrim = { confirm = trimConfirm { viewModel.trim() } },
                            onAnalyze = viewModel::analyzeUsage,
                        )
                    }
                }
            }
        }
    }

    confirm?.let { request -> ConfirmDialog(request = request, onDismiss = { confirm = null }) }
}

/** One confirmation: the icon sits in a cookie badge, and a risky action gets the error colors. */
@Composable
private fun ConfirmDialog(request: StorageConfirm, onDismiss: () -> Unit) {
    val scheme = MaterialTheme.colorScheme
    AlertDialog(
        onDismissRequest = onDismiss,
        shape = MaterialTheme.shapes.extraLarge,
        containerColor = scheme.surfaceContainerHigh,
        icon = {
            Box(
                modifier = Modifier
                    .size(64.dp)
                    .clip(MaterialShapes.Cookie9Sided.toShape())
                    .background(if (request.danger) scheme.errorContainer else scheme.primaryContainer),
                contentAlignment = Alignment.Center,
            ) {
                Icon(
                    request.icon,
                    contentDescription = null,
                    tint = if (request.danger) scheme.onErrorContainer else scheme.onPrimaryContainer,
                    modifier = Modifier.size(30.dp),
                )
            }
        },
        title = { Text(request.title, style = MaterialTheme.typography.headlineSmallEmphasized) },
        text = { Text(request.text, style = MaterialTheme.typography.bodyMedium) },
        confirmButton = {
            Button(
                onClick = {
                    onDismiss()
                    request.onConfirm()
                },
                shapes = ButtonDefaults.shapes(),
                colors = if (request.danger) {
                    ButtonDefaults.buttonColors(
                        containerColor = scheme.error,
                        contentColor = scheme.onError,
                    )
                } else {
                    ButtonDefaults.buttonColors()
                },
            ) { Text(request.confirmLabel) }
        },
        dismissButton = {
            TextButton(onClick = onDismiss, shapes = ButtonDefaults.shapes()) { Text("取消") }
        },
    )
}

private data class StorageConfirm(
    val title: String,
    val text: String,
    val confirmLabel: String,
    val icon: ImageVector,
    val danger: Boolean = false,
    val onConfirm: () -> Unit,
)

private fun trimConfirm(onConfirm: () -> Unit) = StorageConfirm(
    title = "执行 TRIM？",
    text = "向 /data 的存储控制器发送一次丢弃指令。不会删除文件，但可能让设备短暂卡顿；不会在开机时重放。",
    confirmLabel = "执行",
    icon = Icons.Rounded.DeleteForever,
    danger = true,
    onConfirm = onConfirm,
)
