@file:OptIn(ExperimentalMaterial3ExpressiveApi::class)

package io.sysflow.feature.wakelock

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.asPaddingValues
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.navigationBars
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.Apps
import androidx.compose.material.icons.rounded.Bolt
import androidx.compose.material.icons.rounded.Lock
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.ButtonGroupDefaults
import androidx.compose.material3.ExperimentalMaterial3ExpressiveApi
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.ToggleButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import io.sysflow.core.model.mergeWithRules
import io.sysflow.core.ui.components.rememberAppIdentities
import org.koin.androidx.compose.koinViewModel

/**
 * The three views of the wake-lock page.
 *
 * They are peers, not navigation levels: a connected button group keeps the page a single
 * destination in the back stack (a drawer would hide content on a phone and add a tap to every
 * switch).
 */
private enum class WakelockTab(val label: String, val icon: ImageVector) {
    Sources("唤醒源", Icons.Rounded.Bolt),
    Apps("应用", Icons.Rounded.Apps),
    Held("持锁", Icons.Rounded.Lock),
}

/**
 * Wake-lock destination: a connected tab group on top, one scrolling list below.
 *
 * `SysFlowApp` already wraps every titled route in a `FlexibleSubPage`; adding another one here is
 * what once produced two stacked app bars, so this page only renders its content.
 *
 * Each tab lives in its own file ([sourcesTab], [appsTab], [heldTab]) and contributes items to the
 * shared [LazyColumn], so rows animate in place and the scroll position is not lost on rotation.
 */
@Composable
fun WakelockScreen(
    viewModel: WakelockViewModel = koinViewModel(),
) {
    val state by viewModel.state.collectAsStateWithLifecycle()
    var tabIndex by rememberSaveable { mutableIntStateOf(0) }
    // 0 = closed, 1 = blocking picker, 2 = whitelist picker. An Int keeps it saveable across
    // process death without a custom Saver.
    var pickerMode by rememberSaveable { mutableIntStateOf(0) }
    var confirmRelease by rememberSaveable { mutableStateOf(false) }
    // 唤醒源页：按时长还是按唤醒次数排行。
    var sourcesByCount by rememberSaveable { mutableStateOf(false) }
    // The last-wakeup reason needs a `dumpsys power`, so it loads on the first visit to 唤醒源.
    var wakeupLoadAttempted by rememberSaveable { mutableStateOf(false) }
    // The app ranking needs a `dumpsys batterystats` run, so it must not load with the page. It does
    // load on the first visit to the 应用 tab, though: making the user hunt for a button to answer
    // "which app did this" is the wrong default. One attempt per screen instance.
    var appsAutoLoadAttempted by rememberSaveable { mutableStateOf(false) }
    val tabs = WakelockTab.entries
    val tab = tabs[tabIndex.coerceIn(0, tabs.lastIndex)]

    LaunchedEffect(tab) {
        if (tab == WakelockTab.Sources && !wakeupLoadAttempted) {
            wakeupLoadAttempted = true
            viewModel.loadWakeup()
            viewModel.loadHistory()
        }
        if (tab == WakelockTab.Apps && !appsAutoLoadAttempted && state.apps == null && !state.appsLoading) {
            appsAutoLoadAttempted = true
            viewModel.loadApps()
        }
    }

    // BatteryStats and AppOps only carry package names; resolve labels and icons once per list,
    // not per row, so the tables read as normal app lists.
    // The module publishes its protection policy, so the UI renders the same rule the module
    // enforces instead of a second copy that can drift.
    val protectedPackages = state.state?.protectedPackages.orEmpty().toSet()
    val appRows = remember(state.apps, state.blocked, protectedPackages, state.exempt) {
        state.apps.mergeWithRules(state.blocked, protectedPackages, state.exempt)
    }
    val appIdentities = rememberAppIdentities(appRows.mapNotNull { it.packageName })
    val heldIdentities = rememberAppIdentities(state.held?.locks.orEmpty().mapNotNull { it.packageName })
    val bottomInset = WindowInsets.navigationBars.asPaddingValues().calculateBottomPadding()

    Column(modifier = Modifier.fillMaxSize()) {
        WakelockTabs(
            selected = tab,
            onSelect = { tabIndex = it.ordinal },
        )

        LazyColumn(
            modifier = Modifier.fillMaxSize(),
            contentPadding = PaddingValues(
                start = 16.dp,
                end = 16.dp,
                top = 8.dp,
                bottom = 24.dp + bottomInset,
            ),
            verticalArrangement = Arrangement.spacedBy(4.dp),
        ) {
            state.error?.let { error ->
                item(key = "notice-error") {
                    NoticeTile(
                        text = error,
                        kind = NoticeKind.Error,
                        modifier = Modifier
                            .animateItem()
                            .padding(bottom = 8.dp),
                    )
                }
            }
            state.message?.let { message ->
                item(key = "notice-message") {
                    NoticeTile(
                        text = message,
                        kind = NoticeKind.Info,
                        modifier = Modifier
                            .animateItem()
                            .padding(bottom = 8.dp),
                        onDismiss = { viewModel.dismissMessage() },
                    )
                }
            }

            when (tab) {
                WakelockTab.Sources -> sourcesTab(
                    state = state,
                    byCount = sourcesByCount,
                    onToggleMetric = { sourcesByCount = !sourcesByCount },
                    onRefreshWakeup = { viewModel.loadWakeup(refresh = true) },
                )
                WakelockTab.Apps -> appsTab(
                    state = state,
                    rows = appRows,
                    identities = appIdentities,
                    onLoadApps = { viewModel.loadApps(refresh = true) },
                    onOpenPicker = { mode ->
                        pickerMode = if (mode == WakelockPickerMode.Exempt) 2 else 1
                    },
                    onToggle = { packageName, blocked -> viewModel.setBlocked(packageName, blocked) },
                    onExempt = { packageName, exempt -> viewModel.setExempt(packageName, exempt) },
                    onReleaseAll = { confirmRelease = true },
                )
                WakelockTab.Held -> heldTab(
                    state = state,
                    identities = heldIdentities,
                    protectedPackages = protectedPackages,
                    onRefresh = { viewModel.loadHeld(refresh = true) },
                    onBlock = { packageName, blocked ->
                        viewModel.setBlocked(packageName, blocked)
                    },
                )
            }
        }
    }

    if (pickerMode != 0) {
        val mode = if (pickerMode == 2) WakelockPickerMode.Exempt else WakelockPickerMode.Block
        WakelockAppPickerSheet(
            blocked = state.blocked,
            pending = state.pending,
            protectedPackages = protectedPackages,
            mode = mode,
            exempt = state.exempt,
            onToggle = { packageName, want ->
                if (mode == WakelockPickerMode.Exempt) {
                    viewModel.setExempt(packageName, want)
                } else {
                    viewModel.setBlocked(packageName, want)
                }
            },
            onDismiss = { pickerMode = 0 },
        )
    }

    if (confirmRelease) {
        AlertDialog(
            onDismissRequest = { confirmRelease = false },
            title = { Text("恢复全部拦截？") },
            text = {
                Text(
                    "会解除本模块保存的所有 WAKE_LOCK 拦截，并清空保护名单。" +
                        "这些应用的唤醒任务将恢复正常。",
                )
            },
            confirmButton = {
                TextButton(
                    onClick = {
                        confirmRelease = false
                        viewModel.releaseAll()
                    },
                ) { Text("恢复") }
            },
            dismissButton = {
                TextButton(onClick = { confirmRelease = false }) { Text("取消") }
            },
        )
    }
}

/**
 * Connected button group acting as the tab switcher.
 *
 * The checked segment morphs into a full pill while its neighbours keep soft inner corners, which
 * is the M3 Expressive replacement for the classic segmented control.
 */
@Composable
private fun WakelockTabs(
    selected: WakelockTab,
    onSelect: (WakelockTab) -> Unit,
    modifier: Modifier = Modifier,
) {
    val tabs = WakelockTab.entries
    Row(
        modifier = modifier
            .fillMaxWidth()
            .padding(horizontal = 16.dp, vertical = 8.dp),
        horizontalArrangement = Arrangement.spacedBy(ButtonGroupDefaults.ConnectedSpaceBetween),
    ) {
        tabs.forEachIndexed { index, entry ->
            val shapes = when (index) {
                0 -> ButtonGroupDefaults.connectedLeadingButtonShapes()
                tabs.lastIndex -> ButtonGroupDefaults.connectedTrailingButtonShapes()
                else -> ButtonGroupDefaults.connectedMiddleButtonShapes()
            }
            ToggleButton(
                checked = entry == selected,
                onCheckedChange = { onSelect(entry) },
                modifier = Modifier.weight(1f),
                shapes = shapes,
            ) {
                Icon(
                    imageVector = entry.icon,
                    contentDescription = null,
                    modifier = Modifier.size(ButtonDefaults.IconSize),
                )
                Spacer(Modifier.size(ButtonDefaults.IconSpacing))
                Text(entry.label, maxLines = 1)
            }
        }
    }
}
