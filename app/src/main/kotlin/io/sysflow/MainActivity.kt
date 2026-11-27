package io.sysflow

import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import io.sysflow.core.ui.theme.SysFlowTheme
import io.sysflow.ui.SysFlowApp

class MainActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()
        setContent {
            SysFlowTheme {
                SysFlowApp()
            }
        }
    }
}
