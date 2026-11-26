@file:OptIn(ExperimentalMaterial3ExpressiveApi::class)

package io.sysflow.ui

import androidx.activity.compose.BackHandler
import androidx.compose.animation.AnimatedContent
import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.togetherWith
import androidx.compose.animation.core.animateDpAsState
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.slideInHorizontally
import androidx.compose.animation.slideInVertically
import androidx.compose.animation.slideOutHorizontally
import androidx.compose.animation.slideOutVertically
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.material3.ExperimentalMaterial3ExpressiveApi
import androidx.compose.material3.FloatingToolbarDefaults
import androidx.compose.material3.FloatingToolbarExitDirection
import androidx.compose.material3.LoadingIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.input.nestedscroll.nestedScroll
import androidx.compose.ui.unit.IntOffset
import androidx.compose.ui.unit.dp
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.compose.LocalLifecycleOwner
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.repeatOnLifecycle
import io.sysflow.core.event.EventRepository
import io.sysflow.core.model.BatterySnapshot
import io.sysflow.feature.home.BatteryRankingScreen
import io.sysflow.feature.home.BatteryScreen
import io.sysflow.core.ui.components.FlexibleSubPage
import io.sysflow.feature.adaptive.AdaptiveScreen
import io.sysflow.feature.cpu.CpuScreen
import io.sysflow.feature.cpuset.CpusetScreen
import io.sysflow.feature.diag.DiagnosticsScreen
import io.sysflow.feature.home.AppViewModel
import io.sysflow.feature.home.ClientStatus
import io.sysflow.feature.home.EventHistoryScreen
import io.sysflow.feature.home.ModuleActivityScreen
import io.sysflow.feature.home.HomeScreen
import io.sysflow.feature.home.ModuleHubScreen
import io.sysflow.feature.screenoff.ScreenOffScreen
import io.sysflow.feature.settings.SettingsScreen
import io.sysflow.feature.mem.MemScreen
import io.sysflow.feature.process.ProcessScreen
import io.sysflow.feature.storage.StorageScreen
import io.sysflow.feature.wakelock.WakelockScreen
import io.sysflow.ui.navigation.Destination
import io.sysflow.ui.navigation.FloatingNavBar
import io.sysflow.ui.navigation.FloatingNavBarDefaults
import kotlinx.coroutines.delay
import kotlinx.coroutines.isActive
import org.koin.androidx.compose.koinViewModel
import org.koin.compose.koinInject

@Composable
fun SysFlowApp(viewModel: AppViewModel = koinViewModel()) {
    val eventRepository: EventRepository = koinInject()
    val state by viewModel.state.collectAsStateWithLifecycle()
    var current by rememberSaveable { mutableStateOf(Destination.Overview) }
    var cpuDetail by rememberSaveable { mutableStateOf(false) }
    var batteryDetail by rememberSaveable { mutableStateOf(false) }
    var batteryRankingOpen by rememberSaveable { mutableStateOf(false) }
    var wakelockDetail by rememberSaveable { mutableStateOf(false) }
    var memDetail by rememberSaveable { mutableStateOf(false) }
    var processDetail by rememberSaveable { mutableStateOf(false) }
    var storageDetail by rememberSaveable { mutableStateOf(false) }
    var batteryOpenedFromOverview by rememberSaveable { mutableStateOf(false) }
    var adaptiveDetail by rememberSaveable { mutableStateOf(false) }
    var cpusetDetail by rememberSaveable { mutableStateOf(false) }
    var screenOffDetail by rememberSaveable { mutableStateOf(false) }
    var diagnosticsDetail by rememberSaveable { mutableStateOf(false) }
    var eventHistoryOpen by rememberSaveable { mutableStateOf(false) }
    // Module whose events the history page is filtered to; null means "all modules".
    var eventModuleFilter by rememberSaveable { mutableStateOf<String?>(null) }
    var moduleActivityOpen by rememberSaveable { mutableStateOf(false) }
    // CPU management can be opened either from Overview or from the Modules hub.
    var cpuOpenedFromOverview by rememberSaveable { mutableStateOf(false) }
    var wasAwayFromOverview by rememberSaveable { mutableStateOf(false) }

    val battery = remember(state.snapshot) { BatterySnapshot.from(state.snapshot?.modules?.get("battery")) }
    val snackbar = remember { SnackbarHostState() }
    val lifecycleOwner = LocalLifecycleOwner.current
    val motion = MaterialTheme.motionScheme
    val navScrollBehavior = FloatingToolbarDefaults.exitAlwaysScrollBehavior(
        exitDirection = FloatingToolbarExitDirection.Bottom,
    )

    val route = ScreenRoute(
        destination = current,
        cpuDetail = cpuDetail,
        batteryDetail = batteryDetail,
        batteryRankingOpen = batteryRankingOpen,
        wakelockDetail = wakelockDetail,
        memDetail = memDetail,
        processDetail = processDetail,
        storageDetail = storageDetail,
        eventHistoryOpen = eventHistoryOpen,
        moduleActivityOpen = moduleActivityOpen,
        adaptiveDetail = adaptiveDetail,
        screenOffDetail = screenOffDetail,
        cpusetDetail = cpusetDetail,
        diagnosticsDetail = diagnosticsDetail,
    )
    val pageTitle = routeTitle(route)
    val pageSubtitle = routeSubtitle(pageTitle)
    val isSubPage = pageTitle != null
    val navVisible = state.status is ClientStatus.Ready && !isSubPage

    // Keep the floating navigation toolbar in its expanded position when returning to a main tab.
    LaunchedEffect(current, cpuDetail, adaptiveDetail, cpusetDetail, screenOffDetail, diagnosticsDetail, eventHistoryOpen) {
        navScrollBehavior.state.offset = 0f
    }

    // Immediately pull latest module events when returning to Overview from any control screen.
    LaunchedEffect(current, isSubPage, state.status) {
        if (state.status != ClientStatus.Ready) return@LaunchedEffect
        if (current != Destination.Overview || isSubPage) {
            wasAwayFromOverview = true
        } else if (wasAwayFromOverview) {
            wasAwayFromOverview = false
            viewModel.syncEvents(force = true)
        }
    }

    LaunchedEffect(state.message) {
        if (state.status !is ClientStatus.Ready) return@LaunchedEffect
        val message = state.message ?: return@LaunchedEffect
        snackbar.showSnackbar(message)
        viewModel.dismissMessage()
    }

    LaunchedEffect(current, isSubPage, state.status, batteryDetail, batteryRankingOpen) {
        // The overview polls continuously; the battery page polls too because its session curve is
        // built from successive snapshots and would otherwise stay at a single point.
        val polling = state.status == ClientStatus.Ready &&
            ((current == Destination.Overview && !isSubPage) ||
                (current == Destination.Modules && batteryDetail && !batteryRankingOpen))
        if (polling) {
            lifecycleOwner.repeatOnLifecycle(Lifecycle.State.STARTED) {
                while (isActive) {
                    delay(2_000)
                    viewModel.refresh()
                }
            }
        }
    }

    val canNavigateBack = current != Destination.Overview || isSubPage
    val navigateBack: () -> Unit = {
        when {
            route.eventHistoryOpen -> {
                eventHistoryOpen = false
                eventModuleFilter = null
            }
            moduleActivityOpen -> moduleActivityOpen = false
            route.destination == Destination.Modules && route.batteryRankingOpen -> batteryRankingOpen = false
            route.destination == Destination.Modules && cpusetDetail -> cpusetDetail = false
            route.destination == Destination.Modules && screenOffDetail -> screenOffDetail = false
            route.destination == Destination.Modules && adaptiveDetail -> adaptiveDetail = false
            route.destination == Destination.Modules && wakelockDetail -> {
                wakelockDetail = false
                current = Destination.Modules
            }
            route.destination == Destination.Modules && memDetail -> {
                memDetail = false
                current = Destination.Modules
            }
            route.destination == Destination.Modules && processDetail -> {
                processDetail = false
                current = Destination.Modules
            }
            route.destination == Destination.Modules && storageDetail -> {
                storageDetail = false
                current = Destination.Modules
            }
            route.destination == Destination.Modules && route.batteryDetail -> {
                batteryDetail = false
                batteryRankingOpen = false
                current = if (batteryOpenedFromOverview) Destination.Overview else Destination.Modules
                batteryOpenedFromOverview = false
            }
            route.destination == Destination.Modules && route.cpuDetail -> {
                cpuDetail = false
                current = if (cpuOpenedFromOverview) Destination.Overview else Destination.Modules
                cpuOpenedFromOverview = false
            }
            route.destination == Destination.Modules -> current = Destination.Overview
            route.destination == Destination.Settings && route.diagnosticsDetail -> diagnosticsDetail = false
            route.destination == Destination.Settings && route.adaptiveDetail -> adaptiveDetail = false
            route.destination == Destination.Settings -> current = Destination.Overview
            else -> {
                diagnosticsDetail = false
                screenOffDetail = false
                cpusetDetail = false
                adaptiveDetail = false
                cpuDetail = false
                batteryDetail = false
                batteryRankingOpen = false
                cpuOpenedFromOverview = false
                batteryOpenedFromOverview = false
                current = Destination.Overview
            }
        }
    }
    BackHandler(enabled = canNavigateBack, onBack = navigateBack)

    val snackbarBottom by animateDpAsState(
        targetValue = if (navVisible) FloatingNavBarDefaults.ContentBottomInset else 16.dp,
        animationSpec = motion.defaultSpatialSpec(),
        label = "snackbarBottom",
    )

    val renderRoute: @Composable (ScreenRoute) -> Unit = { route ->
        when {
            state.loading -> Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
                LoadingIndicator()
            }
            state.status !is ClientStatus.Ready -> {
                GuideScreen(
                    kind = state.status.toGuideKind(),
                    version = state.caps?.sysflow_version,
                    reason = when (state.status) {
                        is ClientStatus.Unsupported, is ClientStatus.Error ->
                            state.unsupportedReason ?: state.message
                        else -> null
                    },
                    onRetry = viewModel::boot,
                )
            }
            route.moduleActivityOpen && !route.eventHistoryOpen -> ModuleActivityScreen(
                events = state.activityEvents,
                onOpenModule = { module ->
                    eventModuleFilter = module
                    eventHistoryOpen = true
                },
                onClearEvents = viewModel::clearEvents,
                onDiagnostics = viewModel::eventsDiagnostics,
            )
            route.eventHistoryOpen -> EventHistoryScreen(
                repository = eventRepository,
                initialModule = eventModuleFilter,
            )
            route.destination == Destination.Overview -> {
                val snapshot = state.snapshot
                val caps = state.caps
                if (snapshot != null && caps != null) {
                    HomeScreen(
                        snapshot = snapshot,
                        caps = caps,
                        safe = state.safe,
                        deviceInfo = state.deviceInfo,
                        activityEvents = state.activityEvents,
                        onOpenCpu = {
                            cpuOpenedFromOverview = true
                            current = Destination.Modules
                            cpuDetail = true
                            adaptiveDetail = false
                            cpusetDetail = false
                            screenOffDetail = false
                        },
                        onClearSafe = viewModel::clearSafeMode,
                        onOpenEvents = { module ->
                            eventModuleFilter = module
                            moduleActivityOpen = false
                            eventHistoryOpen = true
                        },
                        onOpenActivity = { moduleActivityOpen = true },
                        battery = battery,
                        onOpenBattery = {
                            batteryOpenedFromOverview = true
                            cpuDetail = false
                            current = Destination.Modules
                            batteryDetail = true
                        },
                    )
                }
            }
            route.destination == Destination.Modules && route.wakelockDetail -> WakelockScreen()
            route.destination == Destination.Modules && route.memDetail -> MemScreen()
            route.destination == Destination.Modules && route.processDetail -> ProcessScreen()
            route.destination == Destination.Modules && route.storageDetail -> StorageScreen()
            route.destination == Destination.Modules && route.batteryDetail && route.batteryRankingOpen ->
                BatteryRankingScreen(loadApps = { refresh -> viewModel.loadBatteryApps(refresh) })
            route.destination == Destination.Modules && route.batteryDetail -> BatteryScreen(
                battery = battery,
                refreshTick = state.snapshot?.ts ?: 0L,
                loadApps = { refresh -> viewModel.loadBatteryApps(refresh) },
                loadHealth = { refresh -> viewModel.loadBatteryHealth(refresh) },
                loadHistory = { viewModel.loadBatteryHistory() },
                onOpenRanking = { batteryRankingOpen = true },
            )
            route.destination == Destination.Modules && route.cpuDetail && route.cpusetDetail -> CpusetScreen()
            route.destination == Destination.Modules && route.cpuDetail && route.screenOffDetail -> ScreenOffScreen()
            route.destination == Destination.Modules && route.cpuDetail && route.adaptiveDetail -> AdaptiveScreen()
            route.destination == Destination.Modules && route.cpuDetail -> CpuScreen(
                onOpenAdaptive = { adaptiveDetail = true },
                onOpenCpuset = { cpusetDetail = true },
                onOpenScreenOff = { screenOffDetail = true },
            )
            route.destination == Destination.Modules -> {
                val caps = state.caps
                if (caps != null) {
                    ModuleHubScreen(
                        caps = caps,
                        onOpenCpu = {
                            cpuOpenedFromOverview = false
                            batteryDetail = false
                            cpuDetail = true
                        },
                        batteryReady = battery != null,
                        wakelockReady = caps.modules["wakelock"]?.supported == true,
                        memReady = state.snapshot?.modules?.get("mem") != null,
                        processReady = caps.modules["process"]?.supported == true,
                        storageReady = caps.modules["storage"]?.supported == true,
                        onOpenStorage = {
                            cpuDetail = false
                            batteryDetail = false
                            wakelockDetail = false
                            memDetail = false
                            processDetail = false
                            storageDetail = true
                            current = Destination.Modules
                        },
                        onOpenProcess = {
                            cpuDetail = false
                            batteryDetail = false
                            wakelockDetail = false
                            memDetail = false
                            processDetail = true
                            current = Destination.Modules
                        },
                        onOpenMem = {
                            cpuDetail = false
                            batteryDetail = false
                            wakelockDetail = false
                            memDetail = true
                            current = Destination.Modules
                        },
                        onOpenWakelock = {
                            cpuDetail = false
                            batteryDetail = false
                            wakelockDetail = true
                            current = Destination.Modules
                        },
                        onOpenBattery = {
                            batteryOpenedFromOverview = false
                            cpuDetail = false
                            current = Destination.Modules
                            batteryDetail = true
                        },
                    )
                }
            }
            route.destination == Destination.Settings && route.diagnosticsDetail -> DiagnosticsScreen()
            route.destination == Destination.Settings && route.adaptiveDetail -> AdaptiveScreen()
            route.destination == Destination.Settings -> SettingsScreen(
                onOpenAdaptive = { adaptiveDetail = true },
                onOpenDiagnostics = { diagnosticsDetail = true },
            )
        }
    }

    Box(
        modifier = Modifier
            .fillMaxSize()
            .background(MaterialTheme.colorScheme.background)
            .then(if (navVisible) Modifier.nestedScroll(navScrollBehavior) else Modifier),
    ) {
        AnimatedContent(
            targetState = route,
            transitionSpec = {
                // Depth decides direction: pushing into a deeper screen slides in from the end and
                // the old one leaves to the start; going back mirrors it. Same M3E motion specs as
                // the rest of the app, so transitions feel like one system.
                val forward = when {
                    // Nested screens always win: deeper slides in from the end.
                    targetState.depth != initialState.depth -> targetState.depth > initialState.depth
                    // Same depth means a tab switch; use the tab order so Settings → Overview
                    // slides back the way the user came from.
                    else -> targetState.tabIndex >= initialState.tabIndex
                }
                val enter = if (forward) 1 else -1
                val spatial = motion.defaultSpatialSpec<IntOffset>()
                val effects = motion.defaultEffectsSpec<Float>()
                (
                    slideInHorizontally(spatial) { width -> enter * width / 5 } +
                        fadeIn(effects)
                    ).togetherWith(
                    slideOutHorizontally(spatial) { width -> -enter * width / 5 } +
                        fadeOut(effects),
                )
            },
            label = "screen",
        ) { animatedRoute ->
            val title = routeTitle(animatedRoute)
            val subtitle = routeSubtitle(title)
            if (title != null && subtitle != null) {
                FlexibleSubPage(
                    title = title,
                    subtitle = subtitle,
                    onBack = navigateBack,
                ) {
                    renderRoute(animatedRoute)
                }
            } else {
                // Main tabs use their own in-page headings; only detail pages get a flexible app bar.
                Box(
                    modifier = Modifier
                        .fillMaxSize()
                        .statusBarsPadding(),
                ) {
                    renderRoute(animatedRoute)
                }
            }
        }

        SnackbarHost(
            hostState = snackbar,
            modifier = Modifier
                .align(Alignment.BottomCenter)
                .navigationBarsPadding()
                .padding(bottom = snackbarBottom),
        )

        AnimatedVisibility(
            visible = navVisible,
            modifier = Modifier
                .align(Alignment.BottomCenter)
                .navigationBarsPadding()
                .offset(y = -FloatingToolbarDefaults.ScreenOffset),
            enter = slideInVertically(motion.defaultSpatialSpec()) { it } + fadeIn(motion.defaultEffectsSpec()),
            exit = slideOutVertically(motion.defaultSpatialSpec()) { it } + fadeOut(motion.defaultEffectsSpec()),
        ) {
            FloatingNavBar(
                current = current,
                onSelect = {
                    current = it
                    cpuDetail = false
                    batteryDetail = false
                    batteryRankingOpen = false
                    adaptiveDetail = false
                    cpusetDetail = false
                    screenOffDetail = false
                    diagnosticsDetail = false
                    eventHistoryOpen = false
                    moduleActivityOpen = false
                    cpuOpenedFromOverview = false
                    batteryOpenedFromOverview = false
                },
                scrollBehavior = navScrollBehavior,
            )
        }
    }
}

private fun ClientStatus.toGuideKind(): GuideKind = when (this) {
    ClientStatus.NoRoot -> GuideKind.NoRoot
    ClientStatus.NoModule -> GuideKind.NoModule
    ClientStatus.Unsupported -> GuideKind.Unsupported
    else -> GuideKind.Error
}

/**
 * Immutable snapshot of "which screen is showing".
 *
 * Animating between screens needs both the outgoing and incoming screen to render their own frame,
 * which is impossible while the route lives in scattered mutable flags: the outgoing slot would
 * render the *new* content. Keeping it in a value class gives `AnimatedContent` something stable to
 * key on.
 */
private data class ScreenRoute(
    val destination: Destination,
    val cpuDetail: Boolean = false,
    val batteryDetail: Boolean = false,
    val batteryRankingOpen: Boolean = false,
    val wakelockDetail: Boolean = false,
    val memDetail: Boolean = false,
    val processDetail: Boolean = false,
    val storageDetail: Boolean = false,
    val eventHistoryOpen: Boolean = false,
    val moduleActivityOpen: Boolean = false,
    val adaptiveDetail: Boolean = false,
    val screenOffDetail: Boolean = false,
    val cpusetDetail: Boolean = false,
    val diagnosticsDetail: Boolean = false,
) {
    /** Position of the tab in the bottom bar, used to orient same-depth transitions. */
    val tabIndex: Int
        get() = when (destination) {
            Destination.Overview -> 0
            Destination.Modules -> 1
            Destination.Settings -> 2
        }

    /** 0 = tab root, 1..n = nested screens; drives the transition direction. */
    val depth: Int
        get() = when {
            batteryDetail && batteryRankingOpen -> 3
            eventHistoryOpen || moduleActivityOpen || batteryDetail || wakelockDetail || memDetail || processDetail || storageDetail -> 2
            cpuDetail && (adaptiveDetail || screenOffDetail || cpusetDetail) -> 3
            cpuDetail -> 2
            destination == Destination.Settings && (diagnosticsDetail || adaptiveDetail) -> 2
            else -> 0
        }
}

private fun routeTitle(route: ScreenRoute): String? = when {
    route.eventHistoryOpen -> "系统事件"
    route.moduleActivityOpen -> "模块动态"
    route.destination == Destination.Modules && route.cpuDetail && route.cpusetDetail -> "进程组"
    route.destination == Destination.Modules && route.cpuDetail && route.screenOffDetail -> "熄屏管理"
    route.destination == Destination.Modules && route.cpuDetail && route.adaptiveDetail -> "自适应性能管理"
    route.destination == Destination.Modules && route.cpuDetail -> "CPU 管理"
    route.destination == Destination.Modules && route.batteryDetail && route.batteryRankingOpen -> "耗电排行"
    route.destination == Destination.Modules && route.batteryDetail -> "电池管理"
    route.destination == Destination.Modules && route.wakelockDetail -> "唤醒锁"
    route.destination == Destination.Modules && route.memDetail -> "内存管理"
    route.destination == Destination.Modules && route.processDetail -> "进程管理"
    route.destination == Destination.Modules && route.storageDetail -> "储存管理"
    route.destination == Destination.Settings && route.diagnosticsDetail -> "诊断与日志"
    route.destination == Destination.Settings && route.adaptiveDetail -> "自适应性能管理"
    else -> null
}

private fun routeSubtitle(title: String?): String? = when (title) {
    "系统事件" -> "查看来自所有 Root 管理模块的持久化事件记录"
    "模块动态" -> "按模块查看已产生事件的系统动态"
    "进程组" -> "限制各类进程可以使用的核心，不会改变核心的开关状态"
    "熄屏管理" -> "熄屏时独立限制各调频簇频率，并可选择额外关闭的核心"
    "自适应性能管理" -> "动态收紧频率上限，不会突破已保存的静态上限"
    "CPU 管理" -> "处理器核心、频率与性能调度控制"
    "耗电排行" -> "按应用 UID 汇总的 BatteryStats 功耗估算，仅供参考"
    "电池管理" -> "电量、充电状态与健康度（只读，不写入任何节点）"
    "唤醒锁" -> "内核唤醒源与 24 小时排行，可按应用拦截 WAKE_LOCK"
    "内存管理" -> "内存构成、压力曲线与应用占用，可调整交换倾向与缓存回收压力"
    "进程管理" -> "查看运行中的进程与资源占用，可对单个进程执行一次性操作"
    "储存管理" -> "容量、实时 I/O 与吞吐历史，可调整调度器与预读，并执行 TRIM"
    "诊断与日志" -> "只读检查，不主动写入调频节点"
    else -> null
}
