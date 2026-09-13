package com.rc.launcher.ui.component

import androidx.compose.animation.animateColorAsState
import androidx.compose.animation.core.Spring
import androidx.compose.animation.animateFloatAsState
import androidx.compose.animation.core.spring
import androidx.compose.animation.core.tween
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.PlayArrow
import androidx.compose.material.icons.filled.Star
import androidx.compose.material.icons.outlined.StarBorder
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ElevatedCard
import androidx.compose.material3.FilledIconButton
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.IconButtonDefaults
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import com.rc.launcher.ui.model.GameInstance
import com.rc.launcher.ui.model.lastPlayedLabel

/**
 * Modern game-instance card (task 18 visual redesign).
 *
 * Surfaces in a single tile:
 *  - **Cover / icon**: a deterministic gradient derived from [GameInstance.iconColor]
 *    with the short version text as a watermark, mirroring the FCL launcher
 *    splash style.
 *  - **Loader badge** + **version chip** as discrete Material3 tonal chips so the
 *    card stays scannable at a glance.
 *  - **Last played** as a relative timestamp (zh-CN, mirrors [lastPlayedLabel]).
 *  - **Favorite star** toggle in the top-end corner (optional, only when the
 *    caller wires [onToggleFavorite]).
 *  - **Circular progress ring** in the cover area, repurposed to indicate the
 *    download / install progress for the instance when known (the
 *    [installProgress] is 0\u20131f; 0 hides the ring). When a one-tap launch is in
 *    flight the same ring is filled with an indeterminate
 *    [CircularProgressIndicator] for visual continuity.
 *  - **One-tap launch** as a FilledIconButton, kept accessible (the Icon has a
 *    contentDescription naming the instance).
 *
 * The card is built on [ElevatedCard] (Material3 tonal + elevation) so it reads
 * well on both light and dark surfaces. The whole surface is clickable to open
 * the detail screen; the inner buttons stop the click from bubbling.
 */
@Composable
fun InstanceCard(
    instance: GameInstance,
    onLaunch: (GameInstance) -> Unit,
    onOpen: (GameInstance) -> Unit,
    modifier: Modifier = Modifier,
    launching: Boolean = false,
    installProgress: Float = 0f,
    onToggleFavorite: ((GameInstance) -> Unit)? = null,
) {
    val coverColors = remember(instance.iconColor) { coverGradientFor(instance.iconColor) }
    val progress by animateFloatAsState(
        targetValue = installProgress.coerceIn(0f, 1f),
        animationSpec = tween(durationMillis = 320),
        label = "install-progress",
    )

    ElevatedCard(
        modifier = modifier
            .fillMaxWidth()
            .clip(RoundedCornerShape(16.dp))
            .clickable { onOpen(instance) },
        elevation = CardDefaults.elevatedCardElevation(
            defaultElevation = 2.dp,
            pressedElevation = 4.dp,
            focusedElevation = 4.dp,
            hoveredElevation = 3.dp,
        ),
        shape = RoundedCornerShape(16.dp),
    ) {
        Column(
            modifier = Modifier.fillMaxWidth(),
            verticalArrangement = Arrangement.Vertical.spacedBy(0.dp),
        ) {
            // Cover area with progress ring + favorite + launch overlay.
            Box(
                modifier = Modifier
                    .fillMaxWidth()
                    .height(96.dp),
            ) {
                // Gradient cover plate (the visual anchor of the card).
                Box(
                    modifier = Modifier
                        .fillMaxSize()
                        .clip(RoundedCornerShape(topStart = 16.dp, topEnd = 16.dp))
                        .background(coverColors),
                ) {}

                // Version watermark (white, soft).
                Text(
                    text = instance.version.take(8),
                    modifier = Modifier
                        .align(Alignment.CenterStart)
                        .padding(start = 14.dp),
                    style = MaterialTheme.typography.headlineMedium,
                    color = Color.White.copy(alpha = 0.92f),
                    fontWeight = FontWeight.Black,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )

                // Top-end: favorite star (optional).
                if (onToggleFavorite != null) {
                    FavoriteStar(
                        selected = instance.isFavorite,
                        onClick = { onToggleFavorite(instance) },
                        modifier = Modifier
                            .align(Alignment.TopEnd)
                            .padding(6.dp),
                    )
                }

                // Bottom-end of the cover: progress ring + launch button.
                Box(
                    modifier = Modifier
                        .align(Alignment.BottomEnd)
                        .padding(8.dp),
                ) {
                    when {
                        launching -> CircularProgressIndicator(
                            modifier = Modifier.size(40.dp),
                            color = Color.White,
                            strokeWidth = 4.dp,
                        )
                        installProgress > 0f -> ProgressRing(
                            progress = progress,
                            color = Color.White,
                            track = Color.White.copy(alpha = 0.28f),
                            modifier = Modifier.size(40.dp),
                        )
                        else -> FilledIconButton(
                            onClick = { onLaunch(instance) },
                            modifier = Modifier.size(40.dp),
                            colors = IconButtonDefaults.filledIconButtonColors(
                                containerColor = Color.White.copy(alpha = 0.92f),
                                contentColor = MaterialTheme.colorScheme.primary,
                            ),
                        ) {
                            Icon(
                                imageVector = Icons.Filled.PlayArrow,
                                contentDescription = "\u4e00\u952e\u542f\u52a8 ${instance.name}",
                            )
                        }
                    }
                }
            }

            // Body: name + chips + last played.
            Column(
                modifier = Modifier
                    .fillMaxWidth()
                    .padding(horizontal = 12.dp, vertical = 10.dp),
                verticalArrangement = Arrangement.Vertical.spacedBy(6.dp),
            ) {
                Text(
                    text = instance.name,
                    style = MaterialTheme.typography.titleSmall,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                    fontWeight = FontWeight.SemiBold,
                )
                Row(
                    verticalAlignment = Alignment.Vertical.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(6.dp),
                ) {
                    LoaderBadge(instance.modLoader.label, instance.modLoader.color)
                    VersionBadge(instance.version)
                }
                Text(
                    text = instance.lastPlayedLabel(),
                    style = MaterialTheme.typography.labelSmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
                // Slim linear progress so the card also exposes install progress
                // for instances with a known percentage (the ring only fits
                // when there's room; the bar is the always-on signal).
                if (installProgress > 0f && installProgress < 1f) {
                    LinearProgressIndicator(
                        progress = { progress },
                        modifier = Modifier
                            .fillMaxWidth()
                            .height(4.dp)
                            .clip(RoundedCornerShape(2.dp)),
                        strokeCap = StrokeCap.Round,
                    )
                }
            }
        }
    }
}

@Composable
private fun LoaderBadge(label: String, color: Color) {
    // Tinted chip keeps the loader family recognisable without adding a heavy
    // Surface. Material3's tonal scheme already desaturates the container so
    // the on-container colour reads as accent.
    Surface(
        color = color.copy(alpha = 0.18f),
        contentColor = color,
        shape = RoundedCornerShape(6.dp),
    ) {
        Text(
            text = label,
            modifier = Modifier.padding(horizontal = 8.dp, vertical = 2.dp),
            style = MaterialTheme.typography.labelSmall,
            maxLines = 1,
        )
    }
}

@Composable
private fun VersionBadge(version: String) {
    Surface(
        color = MaterialTheme.colorScheme.secondaryContainer,
        contentColor = MaterialTheme.colorScheme.onSecondaryContainer,
        shape = RoundedCornerShape(6.dp),
    ) {
        Text(
            text = version,
            modifier = Modifier.padding(horizontal = 8.dp, vertical = 2.dp),
            style = MaterialTheme.typography.labelSmall,
            maxLines = 1,
        )
    }
}

@Composable
private fun FavoriteStar(
    selected: Boolean,
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val tint by animateColorAsState(
        targetValue = if (selected) MaterialTheme.colorScheme.tertiary else Color.White.copy(alpha = 0.78f),
        label = "favorite-tint",
    )
    IconButton(
        onClick = onClick,
        modifier = modifier
            .size(32.dp)
            .clip(CircleShape)
            .background(Color.Black.copy(alpha = 0.18f)),
    ) {
        Icon(
            imageVector = if (selected) Icons.Filled.Star else Icons.Outlined.StarBorder,
            contentDescription = if (selected) "\u53d6\u6d88\u6536\u85cf" else "\u6536\u85cf\u5b9e\u4f8b",
            tint = tint,
            modifier = Modifier.size(18.dp),
        )
    }
}

/**
 * Circular progress ring drawn with [Canvas]. Reused as the cover decoration
 * when an install percentage is known, so the ring color tracks the brand
 * without depending on [CircularProgressIndicator] (which always uses the
 * Material primary).
 */
@Composable
fun ProgressRing(
    progress: Float,
    color: Color,
    track: Color,
    modifier: Modifier = Modifier,
    strokeWidthDp: Float = 4f,
) {
    val animated by animateFloatAsState(
        targetValue = progress.coerceIn(0f, 1f),
        animationSpec = spring(stiffness = Spring.StiffnessLow),
        label = "ring",
    )
    Box(modifier = modifier, contentAlignment = Alignment.Center) {
        Canvas(modifier = Modifier.fillMaxSize()) {
            val stroke = strokeWidthDp.dp.toPx()
            val s = Size(size.width - stroke, size.height - stroke)
            val topLeft = Offset(stroke / 2f, stroke / 2f)
            drawArc(
                color = track,
                startAngle = 0f,
                sweepAngle = 360f,
                useCenter = false,
                topLeft = topLeft,
                size = s,
                style = Stroke(width = stroke, cap = StrokeCap.Round),
            )
            drawArc(
                color = color,
                startAngle = -90f,
                sweepAngle = 360f * animated,
                useCenter = false,
                topLeft = topLeft,
                size = s,
                style = Stroke(width = stroke, cap = StrokeCap.Round),
            )
        }
    }
}

/**
 * Deterministic two-stop gradient derived from the seed ARGB color: stops at 28%
 * (slightly darker) and 100% (the seed), with a 12\u00b0 sweep so the card surface
 * looks the same across orientations. Keeps the launcher visually stable
 * without consuming another asset slot.
 */
private fun coverGradientFor(seed: Long): Brush {
    val base = Color(seed)
    val dark = base.copy(
        red = (base.red * 0.72f).coerceIn(0f, 1f),
        green = (base.green * 0.72f).coerceIn(0f, 1f),
        blue = (base.blue * 0.72f).coerceIn(0f, 1f),
        alpha = 1f,
    )
    return Brush.linearGradient(listOf(dark, base))
}
