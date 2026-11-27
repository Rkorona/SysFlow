package io.sysflow

import android.app.Application
import android.content.Context
import androidx.test.core.app.ApplicationProvider
import io.sysflow.core.event.EventRepository
import io.sysflow.core.event.EventSource
import io.sysflow.core.event.SysFlowClientEventSource
import org.junit.After
import org.junit.Assert.assertSame
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.koin.android.ext.koin.androidContext
import org.koin.core.KoinApplication
import org.koin.dsl.koinApplication
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config

/**
 * Pins the production Koin wiring that [SysFlowApplication] installs on a device.
 *
 * The graph is built in a **local** `KoinApplication` (not `startKoin`), so these tests never touch
 * the global Koin context: Robolectric creates one `Application` per test method, and a global
 * `startKoin` would throw `KoinApplicationAlreadyStartedException` from the second test on. The blank
 * `Application` in `@Config` keeps the manifest's `SysFlowApplication.onCreate()` (which starts the
 * real global Koin) out of the picture.
 */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [37], application = Application::class)
class KoinWiringTest {

    private lateinit var koinApp: KoinApplication

    @Before
    fun setUp() {
        val context = ApplicationProvider.getApplicationContext<Context>()
        koinApp = koinApplication {
            androidContext(context)
            // `appModule` is `internal` so the unit test can build the same graph production builds;
            // this is a local Koin instance, so the global context is never started.
            modules(appModule)
        }
    }

    @After
    fun tearDown() {
        if (::koinApp.isInitialized) koinApp.close()
    }

    @Test
    fun eventSourceIsBoundToTheRootClientAdapter() {
        val source = koinApp.koin.get<EventSource>()

        assertTrue(
            "expected the root CLI adapter, got ${source::class.qualifiedName}",
            source is SysFlowClientEventSource,
        )
    }

    @Test
    fun eventSourceBindingIsASingleton() {
        val first = koinApp.koin.get<EventSource>()
        val second = koinApp.koin.get<EventSource>()

        assertSame("EventSource must be a Koin single, not a per-call factory", first, second)
    }

    @Test
    fun repositoryResolvesFromTheProductionModule() {
        val repository = koinApp.koin.get<EventRepository>()

        assertSame(repository, koinApp.koin.get<EventRepository>())
    }
}
