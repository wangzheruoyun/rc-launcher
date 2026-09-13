package com.rc.launcher.ui.screen

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.ArrowBack
import androidx.compose.material.icons.filled.CheckCircle
import androidx.compose.material.icons.filled.CloudDownload
import androidx.compose.material.icons.filled.Extension
import androidx.compose.material.icons.filled.PersonAdd
import androidx.compose.material.icons.filled.RocketLaunch
import androidx.compose.material.icons.filled.Storage
import androidx.compose.material3.Button
import androidx.compose.material3.Icon
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.tooling.preview.Preview
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.viewmodel.compose.viewModel
import com.rc.launcher.ui.i18n.LocalRcStrings
import com.rc.launcher.ui.i18n.RcStringKeys
import com.rc.launcher.ui.model.TutorialStep
import com.rc.launcher.ui.theme.RcBuiltInThemes
import com.rc.launcher.ui.theme.RcTheme
import com.rc.launcher.ui.theme.ThemeNightMode
import com.rc.launcher.ui.viewmodel.TutorialViewModel

/**
 * First-run onboarding / newbie tutorial (task 14).
 *
 * A linear, deterministic flow that walks the user through the launcher's
 * core features and saves a [com.rc.launcher.ui.model.TutorialState] flag at
 * the end so the banner never re-appears on its own. The user can always
 * revisit it from "Settings → Help" (see [com.rc.launcher.ui.model.TutorialState]
 * and [com.rc.launcher.ui.viewmodel.TutorialViewModel]).
 *
 * **Adaptive (task 9).** The card width is capped via `widthIn(max = 520.dp)`
 * and the action row uses `weight(1f)` so the layout reflows correctly on
 * a phone in portrait, a short-landscape phone, and a tablet in landscape
 * without re-measuring the window here (the parent scaffold already provides
 * `LocalRcWindowInfo`, which every nested screen reads through
 * [com.rc.launcher.ui.rcWindowInfo] when it needs column counts).
 *
 * **i18n (task 20).** Every visible string comes from [LocalRcStrings] /
 * [rcString], so the tutorial is fully translated via the same catalogue as
 * the rest of the launcher. The step indicator's `{current}` / `{total}` placeholders
 * go through [RcStringKeys.TUTORIAL_STEP_INDICATOR] which the
 * `check_i18n.py` script asserts on (the same `required` set in
 * [com.rc.launcher.ui.i18n.RcStringKeys.required]).
 *
 * **Persistence (task 19).** Every step update is written through
 * [TutorialViewModel.finish] / [TutorialViewModel.skip] etc. and surfaced as a
 * single [com.rc.launcher.ui.model.TutorialState]; no in-memory state survives
 * a process death, the screen always resumes from the persisted
 * [com.rc.launcher.ui.model.TutorialState.lastStep].
 */
@Composable
fun OnboardingScreen(
    onFinished: () -> Unit,
    vm: TutorialViewModel = viewModel(),
) {
    val state by vm.state.collectAsStateWithLifecycle()
    val strings = LocalRcStrings.current
    val step = state.currentStep
    val total = state.totalSteps
    val isFirst = step == TutorialStep.WELCOME
    val isLast = step == TutorialStep.FINISHED

    // Step indicator: "Step 2 / 6".
    val indicator = strings.format(
        RcStringKeys.TUTORIAL_STEP_INDICATOR,
        "current" to (TutorialStep.entries.indexOf(step) + 1).toString(),
        "total" to total.toString(),
    )

    Surface(
        modifier = Modifier.fillMaxSize(),
        color = MaterialTheme.colorScheme.background,
    ) {
        Column(
            modifier = Modifier
                .fillMaxSize()
                .verticalScroll(rememberScrollState())
                .padding(horizontal = 24.dp, vertical = 32.dp),
            horizontalAlignment = Alignment.CenterHorizontally,
            verticalArrangement = Arrangement.spacedBy(16.dp),
        ) {
            // Top bar: skip button only on non-final steps. The "Back" arrow
            // is intentionally absent so first-time users do not get stuck
            // cycling the carousel without finishing it.
            Row(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.End,
            ) {
                if (!isLast) {
                    TextButton(onClick = {
                        vm.skip()
                        onFinished()
                    }) {
                        Text(strings[RcStringKeys.TUTORIAL_SKIP])
                    }
                }
            }

            OnboardingCard(
                step = step,
                isFirst = isFirst,
                modifier = Modifier.widthIn(max = 520.dp).fillMaxWidth(),
            )

            // Progress + step indicator.
            LinearProgressIndicator(
                progress = {
                    val idx = TutorialStep.entries.indexOf(step) + 1
                    idx.toFloat() / total.coerceAtLeast(1).toFloat()
                },
                modifier = Modifier.fillMaxWidth().heightIn(min = 4.dp),
            )
            Text(
                text = indicator,
                style = MaterialTheme.typography.labelMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )

            // Bottom action row: previous / next (or finish on the last step).
            Row(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.spacedBy(12.dp, Alignment.CenterHorizontally),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                if (!isFirst) {
                    OutlinedButton(
                        onClick = { vm.previous() },
                        modifier = Modifier.weight(1f),
                    ) {
                        Icon(
                            Icons.Filled.ArrowBack,
                            contentDescription = null,
                            modifier = Modifier.size(18.dp),
                        )
                        Text(
                            text = "  " + strings[RcStringKeys.TUTORIAL_PREVIOUS],
                        )
                    }
                } else {
                    // Spacer to keep the layout symmetric / balanced when the
                    // back button is hidden on the very first step.
                    Box(modifier = Modifier.weight(1f))
                }
                Button(
                    onClick = {
                        if (isLast) {
                            vm.finish()
                            onFinished()
                        } else {
                            vm.next()
                        }
                    },
                    modifier = Modifier.weight(1f),
                ) {
                    Text(
                        text = strings[
                            if (isLast) RcStringKeys.TUTORIAL_FINISH
                            else RcStringKeys.TUTORIAL_NEXT
                        ],
                    )
                }
            }

            // Helper hint: visible on every step so the user knows they can
            // revisit this screen from Settings at any time. Keeps the message
            // short so the tutorial stays compact.
            Text(
                text = strings[RcStringKeys.TUTORIAL_REPLAY_SUMMARY],
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
    }
}

/**
 * The actual content card. Each step supplies an icon, a colour and the two
 * strings (title + body); the layout is shared so a step cannot accidentally
 * drift the spacing.
 */
@Composable
private fun OnboardingCard(
    step: TutorialStep,
    isFirst: Boolean,
    modifier: Modifier = Modifier,
) {
    val strings = LocalRcStrings.current
    val (icon, accent, titleKey, bodyKey) = when (step) {
        TutorialStep.WELCOME ->
            Quad(
                Icons.Filled.RocketLaunch,
                MaterialTheme.colorScheme.primary,
                RcStringKeys.TUTORIAL_WELCOME_TITLE,
                RcStringKeys.TUTORIAL_WELCOME_BODY,
            )
        TutorialStep.ADD_ACCOUNT ->
            Quad(
                Icons.Filled.PersonAdd,
                MaterialTheme.colorScheme.secondary,
                RcStringKeys.TUTORIAL_STEP_ACCOUNT_TITLE,
                RcStringKeys.TUTORIAL_STEP_ACCOUNT_BODY,
            )
        TutorialStep.CREATE_INSTANCE ->
            Quad(
                Icons.Filled.Storage,
                MaterialTheme.colorScheme.tertiary,
                RcStringKeys.TUTORIAL_STEP_INSTANCE_TITLE,
                RcStringKeys.TUTORIAL_STEP_INSTANCE_BODY,
            )
        TutorialStep.PICK_RENDERER ->
            Quad(
                Icons.Filled.Extension,
                MaterialTheme.colorScheme.primary,
                RcStringKeys.TUTORIAL_STEP_RENDERER_TITLE,
                RcStringKeys.TUTORIAL_STEP_RENDERER_BODY,
            )
        TutorialStep.IMPORT_MODPACK ->
            Quad(
                Icons.Filled.CloudDownload,
                MaterialTheme.colorScheme.secondary,
                RcStringKeys.TUTORIAL_STEP_IMPORT_TITLE,
                RcStringKeys.TUTORIAL_STEP_IMPORT_BODY,
            )
        TutorialStep.FINISHED ->
            Quad(
                Icons.Filled.CheckCircle,
                MaterialTheme.colorScheme.primary,
                RcStringKeys.TUTORIAL_STEP_DONE_TITLE,
                RcStringKeys.TUTORIAL_STEP_DONE_BODY,
            )
    }

    Surface(
        modifier = modifier,
        shape = RoundedCornerShape(20.dp),
        color = MaterialTheme.colorScheme.surfaceVariant,
        tonalElevation = 1.dp,
    ) {
        Column(
            modifier = Modifier.padding(20.dp),
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            // Coloured icon badge — chosen to match the Material 3 emphasis
            // tokens, not the background, so it stays legible across themes.
            Box(
                modifier = Modifier
                    .size(56.dp)
                    .clip(CircleShape)
                    .background(accent.copy(alpha = 0.15f)),
                contentAlignment = Alignment.Center,
            ) {
                Icon(
                    imageVector = icon,
                    contentDescription = null,
                    tint = accent,
                    modifier = Modifier.size(28.dp),
                )
            }

            Text(
                text = strings[titleKey],
                style = MaterialTheme.typography.headlineSmall,
                color = MaterialTheme.colorScheme.onSurface,
            )
            Text(
                text = strings[bodyKey],
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
            // First step: also render the friendly tagline so the welcome
            // card does not feel empty.
            if (isFirst) {
                Text(
                    text = strings[RcStringKeys.APP_TAGLINE],
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
        }
    }
}

/**
 * Tiny four-tuple helper to keep the [OnboardingCard] `when` block readable.
 * A `data class` of four would do, but a generic alias makes the call site
 * read like a tuple of (icon, colour, title key, body key).
 */
private data class Quad<A, B, C, D>(val a: A, val b: B, val c: C, val d: D)

@Preview(name = "Onboarding", showBackground = true)
@Composable
private fun OnboardingScreenPreview() {
    RcTheme(theme = RcBuiltInThemes.first(), nightMode = ThemeNightMode.LIGHT) {
        OnboardingScreen(onFinished = {})
    }
}
