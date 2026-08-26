package com.rc.launcher.ui.navigation

import androidx.compose.material3.Text
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithText
import androidx.test.ext.junit.runners.AndroidJUnit4
import com.rc.launcher.ui.theme.RcTheme
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

/**
 * Compose UI tests for the task-11 app skeleton (main framework + bottom
 * navigation), run on the JVM unit-test runner through Robolectric + the Compose
 * test rule (same pattern as [com.rc.launcher.ui.component.InstanceCardTest]).
 *
 * They are part of the task-21 CI gate: `./gradlew testDebugUnitTest` fails the
 * build if the skeleton regresses.
 */
@RunWith(AndroidJUnit4::class)
class RcAppSkeletonTest {

    @get:Rule
    val composeTestRule = createComposeRule()

    /**
     * Regression guard for the launch-time crash recorded in
     * logcat/25_08-13-50-16_522.log: Material 3's `TypographyTokens` static
     * initialiser calls `TextStyle.copy$default(...TextMotion...)`, which throws
     * `java.lang.NoSuchMethodError` when Material 3 was compiled against a newer
     * Compose UI than the one on the runtime classpath. [RcTheme] builds its
     * `MaterialTheme` with the default `Typography()`, so composing it reproduces
     * exactly that code path — the test therefore fails loudly if the Compose
     * stack ever drifts apart again (gradle/libs.versions.toml + the
     * resolutionStrategy.force / eachDependency pin in app/build.gradle.kts keep
     * it coherent).
     */
    @Test
    fun rcTheme_composesWithoutThrowing() {
        composeTestRule.setContent {
            RcTheme {
                Text("骨架冒烟测试")
            }
        }
        composeTestRule.onNodeWithText("骨架冒烟测试").assertIsDisplayed()
    }

    /**
     * The bottom navigation is the spine of task 11's main framework. This proves
     * the Material 3 `NavigationBar` / `NavigationBarItem` graph renders every
     * top-level destination without crashing — no Rust core and no locale-engine
     * init required (outside [com.rc.launcher.ui.i18n.RcLocalizationProvider],
     * [com.rc.launcher.ui.i18n.rcString] echoes the i18n key, so the home tab
     * shows the literal "nav.home").
     */
    @Test
    fun bottomNavigationRendersAllTopLevelDestinations() {
        composeTestRule.setContent {
            RcTheme {
                RcBottomNavigationBar(
                    destinations = RcTopLevelDestinations,
                    currentRoute = null,
                    onNavigate = {},
                )
            }
        }
        composeTestRule.onNodeWithText("nav.home").assertIsDisplayed()
    }
}
