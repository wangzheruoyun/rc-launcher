package com.rc.launcher.ui.theme

import android.content.Context
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.net.Uri
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.blur
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext

/**
 * Renders the user's custom launcher background behind the chrome (task 11).
 *
 * It reads [ThemeEngine.backgroundConfig] + the active night mode, loads the
 * chosen image off the main thread, and layers the requested blur and darken
 * treatment. When the config is disabled / has no valid source it draws nothing,
 * so the Material [androidx.compose.material3.MaterialTheme] surface shows
 * through unchanged.
 *
 * The effective darken strength follows the light/dark mode when
 * [BackgroundConfig.followTheme] is on, keeping text legible over photos in both
 * themes ("与深色/浅色主题协调").
 */
@Composable
fun RcBackground(
    config: BackgroundConfig = ThemeEngine.backgroundConfig.collectAsStateWithLifecycle().value,
    nightMode: ThemeNightMode = ThemeEngine.nightMode.collectAsStateWithLifecycle().value,
    modifier: Modifier = Modifier,
) {
    if (!config.enabled || config.uri.isNullOrBlank()) return
    val context = LocalContext.current
    val systemDark = isSystemInDarkTheme()
    val isDark = when (nightMode) {
        ThemeNightMode.SYSTEM -> systemDark
        ThemeNightMode.LIGHT -> false
        ThemeNightMode.DARK -> true
    }
    val bitmap = rememberBackgroundBitmap(config.uri, context) ?: return

    // The caller owns the sizing via [modifier] (root uses fillMaxSize, the
    // settings preview uses a fixed-height box). The image + overlay always fill
    // whatever space that modifier provides.
    Box(modifier = modifier) {
        Image(
            bitmap = bitmap,
            contentDescription = null,
            contentScale = ContentScale.Crop,
            modifier = Modifier
                .fillMaxSize()
                .then(if (config.effect.blurs) Modifier.blur(config.blurRadiusDp.dp) else Modifier),
        )
        if (config.effect.darkens) {
            val alpha = config.effectiveDarkenAlpha(isDark).coerceIn(0f, 1f)
            if (alpha > 0.001f) {
                Box(
                    modifier = Modifier
                        .fillMaxSize()
                        .background(Color.Black.copy(alpha = alpha)),
                )
            }
        }
    }
}

/**
 * Loads [uri] into an [ImageBitmap] on an IO dispatcher, re-loading whenever the
 * source changes. Returns null on any failure so the caller can gracefully fall
 * back to the plain themed surface.
 */
@Composable
private fun rememberBackgroundBitmap(uri: String, context: Context): ImageBitmap? {
    var bitmap by remember(uri) { mutableStateOf<ImageBitmap?>(null) }
    LaunchedEffect(uri) {
        bitmap = null
        val loaded = withContext(Dispatchers.IO) { loadBitmap(context, uri) }
        bitmap = loaded?.asImageBitmap()
    }
    return bitmap
}

/** Load a bitmap from a `content://` grant or a `file://` path. Pure-ish, no UI. */
private fun loadBitmap(context: Context, uri: String): Bitmap? = runCatching {
    when {
        uri.startsWith("content://", ignoreCase = true) -> {
            context.contentResolver.openInputStream(Uri.parse(uri))?.use {
                BitmapFactory.decodeStream(it)
            }
        }
        uri.startsWith("file://", ignoreCase = true) -> {
            BitmapFactory.decodeFile(uri.removePrefix("file://"))
        }
        else -> null
    }
}.getOrNull()
