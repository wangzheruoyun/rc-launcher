package com.rc.launcher.ui

import androidx.compose.ui.test.assertExists
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithText
import androidx.test.ext.junit.runners.AndroidJUnit4
import com.rc.launcher.MainActivity
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

/**
 * Instrumented Compose UI test (task 21) — references the FCL `androidTest` /
 * MCTier instrumented-test pattern: launch the real [MainActivity] through the
 * Compose test rule and assert that the home dashboard actually renders.
 *
 * Runs on a device / emulator via `connectedDebugAndroidTest`. The task-21 CI
 * gate drives these on a GitHub-hosted emulator so a broken home screen fails
 * the build.
 *
 * Assertions use stable literal strings (the home header "主页" and the
 * "游戏实例" section) rather than i18n nav labels, so they hold under any
 * device locale.
 */
@RunWith(AndroidJUnit4::class)
class ComposeNavigationTest {

    @get:Rule
    val composeTestRule = createAndroidComposeRule<MainActivity>()

    @Test
    fun homeScreenRendersInstanceSection() {
        // The home dashboard always shows the "game instances" section header.
        composeTestRule
            .onNode(hasText("游戏实例", substring = true))
            .assertExists()
    }

    @Test
    fun homeScreenHeaderRenders() {
        // The home screen top header is a stable literal ("主页").
        composeTestRule.onNodeWithText("主页").assertExists()
    }
}
