@file:OptIn(ExperimentalMaterial3ExpressiveApi::class)

package io.sysflow.feature.mem

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.asPaddingValues
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.navigationBars
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.Memory
import androidx.compose.material3.ExperimentalMaterial3ExpressiveApi
import androidx.compose.material3.LoadingIndicator
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.compose.LocalLifecycleOwner
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.repeatOnLifecycle
import kotlinx.coroutines.delay
import kotlinx.coroutines.isActive
import org.koin.androidx.compose.koinViewModel

/**
 * Memory page: what the device is holding, what is left, and who is holding it.
 *
 * `SysFlowApp` already wraps this route in a `FlexibleSubPage` titled 「内存管理」, so the page draws
 * no heading of its own — it starts with the number people come for. From there it reads top to
 * bottom as a question chain: how much is left → where did the rest go → is the system struggling
 * → who is responsible → what can I change.
 *
 * Every number comes from `snapshot`; the 24 h curve and the two per-process scans are pulled on
 * demand. The only writes are the two sysctls in [TunablesSection], and those wait for 保存.
 */
@Composable
fun MemScreen(viewModel: MemViewModel = koinViewModel()) {
    val state by viewModel.state.collectAsStateWithLifecycle()
    var detail by remember { mutableStateOf<MemAppEntry?>(null) }
    val lifecycleOwner = LocalLifecycleOwner.current
    val bottomInset = WindowInsets.navigationBars.asPaddingValues().calculateBottomPadding()

    LaunchedEffect(Unit) { viewModel.loadHistory() }
    // Memory is a slow variable, so 3 s is enough; the other pages poll every 1.5–2.5 s. It only
    // runs while this screen is STARTED — the APK still has no background work.
    LaunchedEffect(Unit) {
        lifecycleOwner.repeatOnLifecycle(Lifecycle.State.STARTED) {
            while (isActive) {
                delay(3_000)
                viewModel.refresh()
            }
        }
    }

    val snapshot = state.snapshot
    if (snapshot == null) {
        Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
            if (state.loading) {
                LoadingIndicator()
            } else {
                EmptyActionTile(
                    icon = Icons.Rounded.Memory,
                    text = state.error ?: "设备没有提供 /proc/meminfo，或模块尚未加载。",
                    actionLabel = "重试",
                    onAction = viewModel::refresh,
                    modifier = Modifier.padding(16.dp),
                )
            }
        }
    } else {
        LazyColumn(
            modifier = Modifier.fillMaxSize(),
            contentPadding = PaddingValues(
                start = 16.dp,
                end = 16.dp,
                top = 8.dp,
                bottom = 24.dp + bottomInset,
            ),
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            item(key = "hero") { MemoryHero(snapshot) }
            item(key = "composition") { CompositionSection(snapshot) }
            item(key = "pressure") {
                PressureSection(
                    snapshot = snapshot,
                    history = state.history,
                    historyLoading = state.historyLoading,
                    onRefresh = { viewModel.loadHistory() },
                )
            }
            item(key = "swap") { SwapSection(snapshot) }
            item(key = "apps") {
                AppsSection(
                    state = state,
                    onLoadApps = { refresh -> viewModel.loadApps(refresh) },
                    onLoadBreakdown = { refresh -> viewModel.loadBreakdown(refresh) },
                    onOpen = { detail = it },
                )
            }
            if (state.ranges.isNotEmpty()) {
                item(key = "tunables") {
                    TunablesSection(
                        state = state,
                        onDraft = viewModel::setDraft,
                        onSave = viewModel::save,
                        onRestore = viewModel::restoreDefaults,
                        onDismissMessage = viewModel::dismissMessage,
                    )
                }
            }
            item(key = "details") {
                DetailsSection(snapshot = snapshot, onRefresh = { viewModel.refresh() })
            }
        }
    }

    detail?.let { entry ->
        AppDetailSheet(entry = entry, onDismiss = { detail = null })
    }
}
