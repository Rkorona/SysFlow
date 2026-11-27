@file:OptIn(ExperimentalMaterial3ExpressiveApi::class)

package io.sysflow.feature.diag

import android.content.ClipData
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.ContentCopy
import androidx.compose.material.icons.rounded.ErrorOutline
import androidx.compose.material.icons.rounded.FactCheck
import androidx.compose.material.icons.rounded.Inbox
import androidx.compose.material.icons.rounded.PlayArrow
import androidx.compose.material.icons.rounded.Refresh
import androidx.compose.material.icons.rounded.Terminal
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.ButtonGroupDefaults
import androidx.compose.material3.ExperimentalMaterial3ExpressiveApi
import androidx.compose.material3.FilledTonalButton
import androidx.compose.material3.Icon
import androidx.compose.material3.LoadingIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.ToggleButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.platform.ClipEntry
import androidx.compose.ui.platform.LocalClipboard
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import io.sysflow.core.ui.components.ExpressiveShapes
import io.sysflow.core.ui.components.InfoPill
import kotlinx.coroutines.launch
import org.koin.androidx.compose.koinViewModel

private enum class DiagnosticsTab(val label: String, val icon: ImageVector) {
    Report("诊断报告", Icons.Rounded.FactCheck),
    Logs("模块日志", Icons.Rounded.Terminal),
}

private val LogLineOptions = listOf(50, 100, 200, 500)

@Composable
fun DiagnosticsScreen(
    modifier: Modifier = Modifier,
    viewModel: DiagnosticsViewModel = koinViewModel(),
) {
    val state by viewModel.state.collectAsStateWithLifecycle()
    var tab by rememberSaveable { mutableStateOf(DiagnosticsTab.Report) }
    var logLines by rememberSaveable { mutableIntStateOf(200) }
    val clipboard = LocalClipboard.current
    val snackbar = remember { SnackbarHostState() }
    val scope = rememberCoroutineScope()
    var copied by remember { mutableStateOf(false) }

    LaunchedEffect(Unit) {
        viewModel.runDiagnostics()
        viewModel.loadLogs(logLines)
    }
    LaunchedEffect(copied) {
        if (copied) {
            snackbar.showSnackbar("已复制到剪贴板")
            copied = false
        }
    }

    val isReport = tab == DiagnosticsTab.Report
    val text = if (isReport) state.report else state.logs
    val loading = if (isReport) state.loading else state.logsLoading

    Box(modifier.fillMaxSize()) {
        Column(
            modifier = Modifier
                .fillMaxSize()
                .padding(horizontal = 16.dp, vertical = 8.dp),
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            TabSwitcher(selected = tab, onSelect = { tab = it })

            if (!isReport) {
                LogLinePicker(
                    selected = logLines,
                    onSelect = {
                        logLines = it
                        viewModel.loadLogs(it)
                    },
                )
            }

            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                Button(
                    onClick = {
                        if (isReport) viewModel.runDiagnostics() else viewModel.loadLogs(logLines)
                    },
                    enabled = !loading,
                    shapes = ButtonDefaults.shapes(),
                    modifier = Modifier.weight(1f),
                ) {
                    Icon(
                        imageVector = if (isReport) Icons.Rounded.PlayArrow else Icons.Rounded.Refresh,
                        contentDescription = null,
                        modifier = Modifier.size(ButtonDefaults.IconSize),
                    )
                    Spacer(Modifier.width(ButtonDefaults.IconSpacing))
                    Text(if (isReport) "运行诊断" else "刷新日志")
                }
                FilledTonalButton(
                    onClick = {
                        scope.launch {
                            clipboard.setClipEntry(ClipEntry(ClipData.newPlainText("SysFlow", text)))
                            copied = true
                        }
                    },
                    enabled = text.isNotBlank(),
                    shapes = ButtonDefaults.shapes(),
                ) {
                    Icon(
                        Icons.Rounded.ContentCopy,
                        contentDescription = null,
                        modifier = Modifier.size(ButtonDefaults.IconSize),
                    )
                    Spacer(Modifier.width(ButtonDefaults.IconSpacing))
                    Text("复制")
                }
            }

            state.error?.let { ErrorBanner(it) }

            OutputCard(
                text = text,
                loading = loading,
                isReport = isReport,
                modifier = Modifier.weight(1f),
            )
        }
        SnackbarHost(
            snackbar,
            modifier = Modifier
                .align(Alignment.BottomCenter)
                .padding(bottom = 12.dp),
        )
    }
}

/** Connected button group; the checked segment becomes a full pill (same pattern as the wake-lock tabs). */
@Composable
private fun TabSwitcher(
    selected: DiagnosticsTab,
    onSelect: (DiagnosticsTab) -> Unit,
) {
    val tabs = DiagnosticsTab.entries
    Row(
        modifier = Modifier.fillMaxWidth(),
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
                Spacer(Modifier.width(ButtonDefaults.IconSpacing))
                Text(entry.label, maxLines = 1)
            }
        }
    }
}

/** "最近 N 行" is a real single-choice state now, so it is a connected toggle group instead of chips. */
@Composable
private fun LogLinePicker(
    selected: Int,
    onSelect: (Int) -> Unit,
) {
    Row(
        modifier = Modifier.fillMaxWidth(),
        horizontalArrangement = Arrangement.spacedBy(ButtonGroupDefaults.ConnectedSpaceBetween),
    ) {
        LogLineOptions.forEachIndexed { index, count ->
            val shapes = when (index) {
                0 -> ButtonGroupDefaults.connectedLeadingButtonShapes()
                LogLineOptions.lastIndex -> ButtonGroupDefaults.connectedTrailingButtonShapes()
                else -> ButtonGroupDefaults.connectedMiddleButtonShapes()
            }
            ToggleButton(
                checked = count == selected,
                onCheckedChange = { onSelect(count) },
                modifier = Modifier.weight(1f),
                shapes = shapes,
            ) { Text("$count 行", maxLines = 1) }
        }
    }
}

@Composable
private fun ErrorBanner(message: String) {
    val scheme = MaterialTheme.colorScheme
    Surface(
        modifier = Modifier.fillMaxWidth(),
        shape = MaterialTheme.shapes.large,
        color = scheme.errorContainer,
        contentColor = scheme.onErrorContainer,
    ) {
        Row(
            modifier = Modifier.padding(horizontal = 16.dp, vertical = 12.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            Icon(Icons.Rounded.ErrorOutline, contentDescription = null)
            Text(message, style = MaterialTheme.typography.bodyMedium)
        }
    }
}

/** Large tonal output surface: header with line count, then selectable monospace text. */
@Composable
private fun OutputCard(
    text: String,
    loading: Boolean,
    isReport: Boolean,
    modifier: Modifier = Modifier,
) {
    val scheme = MaterialTheme.colorScheme
    Surface(
        modifier = modifier.fillMaxWidth(),
        shape = MaterialTheme.shapes.extraLarge,
        color = scheme.surfaceContainerLow,
        tonalElevation = 1.dp,
    ) {
        Column(Modifier.fillMaxSize()) {
            Row(
                modifier = Modifier
                    .fillMaxWidth()
                    .padding(start = 20.dp, end = 16.dp, top = 14.dp, bottom = 6.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Text(
                    if (isReport) "诊断输出" else "日志输出",
                    modifier = Modifier.weight(1f),
                    style = MaterialTheme.typography.titleMediumEmphasized,
                )
                if (text.isNotBlank() && !loading) {
                    InfoPill(
                        text = "${text.lineSequence().count()} 行",
                        container = scheme.secondaryContainer,
                        content = scheme.onSecondaryContainer,
                    )
                }
            }
            Box(
                modifier = Modifier
                    .weight(1f)
                    .fillMaxWidth(),
            ) {
                when {
                    loading -> LoadingIndicator(Modifier.align(Alignment.Center))
                    text.isBlank() -> EmptyOutput(isReport, Modifier.align(Alignment.Center))
                    else -> SelectionContainer {
                        Text(
                            text = text,
                            modifier = Modifier
                                .fillMaxSize()
                                .verticalScroll(rememberScrollState())
                                .horizontalScroll(rememberScrollState())
                                .padding(start = 20.dp, end = 20.dp, top = 4.dp, bottom = 20.dp),
                            style = MaterialTheme.typography.bodySmall.copy(
                                fontFamily = FontFamily.Monospace,
                                fontSize = 12.sp,
                                lineHeight = 18.sp,
                            ),
                            color = scheme.onSurface,
                            softWrap = false,
                        )
                    }
                }
            }
        }
    }
}

@Composable
private fun EmptyOutput(isReport: Boolean, modifier: Modifier = Modifier) {
    val scheme = MaterialTheme.colorScheme
    Column(
        modifier = modifier.padding(24.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.spacedBy(10.dp),
    ) {
        Box(Modifier.size(72.dp), contentAlignment = Alignment.Center) {
            Surface(
                modifier = Modifier.matchParentSize(),
                shape = ExpressiveShapes.Cookie12,
                color = scheme.secondaryContainer,
            ) {}
            Icon(
                Icons.Rounded.Inbox,
                contentDescription = null,
                tint = scheme.onSecondaryContainer,
                modifier = Modifier.size(32.dp),
            )
        }
        Text("暂无内容", style = MaterialTheme.typography.titleMediumEmphasized)
        Text(
            if (isReport) "点击「运行诊断」生成设备与模块状态报告" else "点击「刷新日志」读取最近的模块运行日志",
            style = MaterialTheme.typography.bodySmall,
            color = scheme.onSurfaceVariant,
            textAlign = TextAlign.Center,
        )
    }
}
