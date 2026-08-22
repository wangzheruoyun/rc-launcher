package com.rc.launcher.ui.resource

import android.view.Choreographer
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.State
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import java.util.concurrent.atomic.AtomicInteger

/**
 * Live UI frame-rate (task 12 "悬浮帧率 HUD"), measured with [Choreographer].
 *
 * A [Choreographer.FrameCallback] counts rendered frames and reports the rolling
 * frames-per-second once per second. This reflects the *launcher* UI smoothness;
 * the in-game FPS overlay (MCTier `GameHudOverlay` style) would subscribe to the
 * Rust game-process event bus (task 10) and render on top of the game surface.
 *
 * The callback chain is torn down in [DisposableEffect] and the whole thing is
 * guarded so a missing [Choreographer] (e.g. some preview hosts) yields 0 FPS
 * instead of crashing.
 */
@Composable
fun rememberFps(): State<Int> {
    val fps = remember { mutableStateOf(0) }
    DisposableEffect(Unit) {
        val choreographer = try {
            Choreographer.getInstance()
        } catch (_: Throwable) {
            null
        }
        if (choreographer == null) return@DisposableEffect onDispose {}

        val frames = AtomicInteger(0)
        val lastTime = longArrayOf(System.nanoTime())
        val callback = object : Choreographer.FrameCallback {
            override fun doFrame(frameTimeNanos: Long) {
                val now = System.nanoTime()
                val counted = frames.incrementAndGet()
                val elapsed = now - lastTime[0]
                if (elapsed >= 1_000_000_000L) {
                    // frames * (1e9 ns / elapsed ns) → exact fps for the window
                    fps.value = (counted * 1_000_000_000L / elapsed).toInt().coerceAtLeast(0)
                    frames.set(0)
                    lastTime[0] = now
                }
                choreographer.postFrameCallback(this)
            }
        }
        choreographer.postFrameCallback(callback)
        onDispose { choreographer.removeFrameCallback(callback) }
    }
    return fps
}
