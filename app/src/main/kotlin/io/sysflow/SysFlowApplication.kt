package io.sysflow

import android.app.Application
import androidx.room.Room
import io.sysflow.core.event.EventDatabase
import io.sysflow.core.event.EventRepository
import io.sysflow.core.event.EventSource
import io.sysflow.core.event.SysFlowClientEventSource
import io.sysflow.core.root.RootExecutor
import io.sysflow.core.root.SysFlowClient
import io.sysflow.feature.cpu.CpuViewModel
import io.sysflow.feature.adaptive.AdaptiveViewModel
import io.sysflow.feature.home.AppViewModel
import io.sysflow.feature.settings.SettingsViewModel
import io.sysflow.feature.mem.MemViewModel
import io.sysflow.feature.process.ProcessTuningStore
import io.sysflow.feature.process.ProcessViewModel
import io.sysflow.feature.storage.StorageViewModel
import io.sysflow.feature.wakelock.WakelockViewModel
import io.sysflow.feature.diag.DiagnosticsViewModel
import io.sysflow.feature.cpuset.CpusetViewModel
import io.sysflow.feature.screenoff.ScreenOffViewModel
import org.koin.android.ext.koin.androidContext
import org.koin.core.module.dsl.viewModel
import org.koin.core.context.startKoin
import org.koin.dsl.module

// `internal`, not file-private: unit tests start this module in a non-global Koin instance to
// verify the bindings; production still starts exactly one graph from the Application.
internal val appModule = module {
    single { RootExecutor() }
    single { SysFlowClient(get()) }
    single {
        Room.databaseBuilder(androidContext(), EventDatabase::class.java, "sysflow-events.db")
            .addMigrations(EventDatabase.MIGRATION_1_2)
            .build()
    }
    single<EventSource> { SysFlowClientEventSource(get()) }
    single { EventRepository(get(), get()) }
    single { ProcessTuningStore(androidContext()) }
    viewModel { AppViewModel(get(), get()) }
    viewModel { CpuViewModel(get()) }
    viewModel { CpusetViewModel(get()) }
    viewModel { ScreenOffViewModel(get()) }
    viewModel { AdaptiveViewModel(get()) }
    viewModel { SettingsViewModel(get()) }
    viewModel { DiagnosticsViewModel(get()) }
    viewModel { WakelockViewModel(get()) }
    viewModel { MemViewModel(get()) }
    viewModel { ProcessViewModel(get(), get()) }
    viewModel { StorageViewModel(get()) }
}

class SysFlowApplication : Application() {
    override fun onCreate() {
        super.onCreate()
        startKoin {
            androidContext(this@SysFlowApplication)
            modules(appModule)
        }
    }
}
