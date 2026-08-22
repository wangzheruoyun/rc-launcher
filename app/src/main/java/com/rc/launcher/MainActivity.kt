package com.rc.launcher

import android.content.res.Configuration
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import com.rc.launcher.ui.RcApp
import com.rc.launcher.ui.i18n.LocaleEngine
import com.rc.launcher.ui.i18n.RcLocaleContext

class MainActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setContent {
            RcApp()
        }
    }

    /**
     * The manifest declares `android:configChanges="...|locale|layoutDirection"`,
     * so the Activity is *not* recreated when the user changes the device
     * language. Feed the new preference list to the engine instead, so a
     * "follow system" selection re-resolves immediately (task 20).
     */
    override fun onConfigurationChanged(newConfig: Configuration) {
        super.onConfigurationChanged(newConfig)
        LocaleEngine.onSystemLocalesChanged(RcLocaleContext.systemPreferredTags(this))
    }
}
