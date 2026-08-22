package com.rc.launcher

import androidx.test.core.app.ApplicationProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import org.junit.Assert.assertEquals
import org.junit.Test
import org.junit.runner.RunWith

/**
 * Baseline instrumented test (task 21) mirroring the FCL / MCTier
 * `ExampleInstrumentedTest` shape: verify the application context comes up
 * under instrumentation before the Compose UI scenarios run.
 */
@RunWith(AndroidJUnit4::class)
class ExampleInstrumentedTest {
    @Test
    fun useAppContext() {
        val appContext = ApplicationProvider.getApplicationContext<android.content.Context>()
        assertEquals("com.rc.launcher", appContext.packageName)
    }
}
