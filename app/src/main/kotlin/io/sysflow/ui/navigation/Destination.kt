package io.sysflow.ui.navigation

import androidx.annotation.StringRes
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Apps
import androidx.compose.material.icons.filled.Home
import androidx.compose.material.icons.filled.Settings
import androidx.compose.material.icons.outlined.Apps
import androidx.compose.material.icons.outlined.Home
import androidx.compose.material.icons.outlined.Settings
import androidx.compose.ui.graphics.vector.ImageVector
import io.sysflow.R

/** Top-level sections. Feature detail pages keep the Modules tab selected. */
enum class Destination(
    @StringRes val label: Int,
    val icon: ImageVector,
    val selectedIcon: ImageVector,
) {
    Overview(R.string.nav_overview, Icons.Outlined.Home, Icons.Filled.Home),
    Modules(R.string.nav_modules, Icons.Outlined.Apps, Icons.Filled.Apps),
    Settings(R.string.nav_settings, Icons.Outlined.Settings, Icons.Filled.Settings),
}
