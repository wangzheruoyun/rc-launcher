package com.rc.launcher.ui.component

import androidx.compose.material3.MaterialTheme
import androidx.compose.ui.test.assertDoesNotExist
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.test.ext.junit.runners.AndroidJUnit4
import com.rc.launcher.ui.model.GameInstance
import com.rc.launcher.ui.model.ModLoader
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

/**
 * Compose UI tests for [InstanceCard] (task 12 / task 21).
 *
 * These run on the JVM unit-test runner through Robolectric + the Compose test
 * rule (referencing the FCL / MCTier UI-test pattern of driving a real
 * composable and asserting on its rendered semantics). They are part of the
 * task-21 CI gate: `./gradlew testDebugUnitTest` fails the build if any of
 * them regress.
 */
@RunWith(AndroidJUnit4::class)
class InstanceCardTest {

    @get:Rule
    val composeTestRule = createComposeRule()

    @Test
    fun showsNameVersionAndLoaderAndLaunches() {
        val instance = GameInstance(
            id = "t1",
            name = "My World",
            version = "1.20.1",
            modLoader = ModLoader.FABRIC,
            loaderVersion = "0.16.0",
        )
        var launched = false
        var opened = false

        composeTestRule.setContent {
            MaterialTheme {
                InstanceCard(
                    instance = instance,
                    onLaunch = { launched = true },
                    onOpen = { opened = true },
                )
            }
        }

        // Name + version (the cover renders version.take(5)) + loader badge.
        composeTestRule.onNodeWithText("My World").assertIsDisplayed()
        composeTestRule.onNode(hasText("1.20", substring = true)).assertIsDisplayed()
        composeTestRule.onNodeWithText("Fabric").assertIsDisplayed()

        // The play button launches the instance.
        composeTestRule
            .onNodeWithContentDescription("一键启动 My World")
            .assertIsDisplayed()
            .performClick()
        assertTrue("play button should launch the instance", launched)

        // Tapping the card body opens the detail screen.
        composeTestRule.onNodeWithText("My World").performClick()
        assertTrue("tapping the card should open the detail screen", opened)
    }

    @Test
    fun launchingStateHidesPlayButtonAndShowsProgress() {
        val instance = GameInstance(id = "t2", name = "Loading", version = "1.19.2")

        composeTestRule.setContent {
            MaterialTheme {
                InstanceCard(
                    instance = instance,
                    onLaunch = {},
                    onOpen = {},
                    launching = true,
                )
            }
        }

        // While launching, the play button is replaced by an indeterminate
        // progress indicator (so the content description must be gone).
        composeTestRule.onNodeWithContentDescription("一键启动 Loading").assertDoesNotExist()
        composeTestRule.onNodeWithText("Loading").assertIsDisplayed()
    }

    @Test
    fun vanillaInstanceShowsNoLoaderVersion() {
        val instance = GameInstance(id = "t3", name = "Vanilla", version = "1.21.1")

        composeTestRule.setContent {
            MaterialTheme {
                InstanceCard(instance = instance, onLaunch = {}, onOpen = {})
            }
        }

        composeTestRule.onNodeWithText("Vanilla").assertIsDisplayed()
        // Vanilla has no loaderVersion, so only the bare loader label is shown.
        composeTestRule.onNode(hasText("原版", substring = true)).assertIsDisplayed()
    }
}
