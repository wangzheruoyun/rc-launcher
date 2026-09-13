package com.rc.launcher.ui.component

import androidx.compose.animation.animateContentSize
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.ArrowDropDown
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.Alignment
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.TextUnit
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.rc.launcher.ui.i18n.RcStringKeys
import com.rc.launcher.ui.i18n.rcString

/** Maximum number of lines shown when collapsed (task 25). */
private const val COLLAPSED_MAX_LINES = 3

/**
 * A text block that truncates to [collapsedMaxLines] lines when collapsed and
 * shows an inline "expand" / "collapse" affordance (task 25).
 *
 * This is the reusable widget the mod-browser / mod-detail / resource-pack
 * description panels all share so that a long description never pushes the
 * actual mod list off-screen. The truncation is **purely visual**: the full
 * text is always measured and the ellipsis is applied by `Text` via
 * `maxLines` + `overflow`, so screen readers still see the whole string.
 *
 * The toggle is an `TextButton` (not a clickable `Text`) so the touch target
 * meets the 48dp accessible minimum even when the collapsed text is short.
 *
 * @param text                Full description text.
 * @param collapsedMaxLines   Lines to show when collapsed (default 3).
 * @param style               Typography style for the body text.
 * @param color               Text colour (defaults to on-surface-variant).
 * @param lineHeight          Line height.
 * @param expandIcon          Optional icon for the expand button.
 * @param collapseIcon        Optional icon for the collapse button.
 */
@Composable
fun ExpandableText(
    text: String,
    collapsedMaxLines: Int = COLLAPSED_MAX_LINES,
    style: TextStyle = MaterialTheme.typography.bodyMedium,
    color: Color = MaterialTheme.colorScheme.onSurfaceVariant,
    lineHeight: TextUnit = style.lineHeight,
    expandIcon: ImageVector = Icons.Default.ArrowDropDown,
    collapseIcon: ImageVector = Icons.Default.ArrowDropDown,
    modifier: Modifier = Modifier,
) {
    // rememberSaveable keeps the state across process-death recreation
    // (activity restart on rotation) — the user's expand/collapse choice
    // should not be lost just because they turned the device.
    var expanded by rememberSaveable { mutableStateOf(false) }

    Column(modifier = modifier.animateContentSize()) {
        Text(
            text = text,
            style = style,
            color = color,
            lineHeight = lineHeight,
            maxLines = if (expanded) Int.MAX_VALUE else collapsedMaxLines,
            overflow = androidx.compose.ui.text.style.TextOverflow.Ellipsis,
        )

        // Only show the toggle when the text is long enough to have
        // overflowed: if it fits in collapsedMaxLines, the toggle is noise.
        if (text.count { it == '\n' } >= collapsedMaxLines ||
            text.length > collapsedMaxLines * 80
        ) {
            Row(
                verticalAlignment = Alignment.CenterVertically,
                modifier = Modifier.clickable { expanded = !expanded },
            ) {
                TextButton(
                    onClick = { expanded = !expanded },
                ) {
                    Text(
                        text = if (expanded) rcString(RcStringKeys.MOD_COLLAPSE)
                        else rcString(RcStringKeys.MOD_EXPAND),
                        style = MaterialTheme.typography.labelSmall,
                        color = MaterialTheme.colorScheme.primary,
                        fontSize = 12.sp,
                    )
                    Spacer(Modifier.width(2.dp))
                    Icon(
                        imageVector = if (expanded) collapseIcon else expandIcon,
                        contentDescription = if (expanded)
                            rcString(RcStringKeys.MOD_COLLAPSE)
                        else
                            rcString(RcStringKeys.MOD_EXPAND),
                        tint = MaterialTheme.colorScheme.primary,
                        modifier = Modifier.size(16.dp),
                    )
                }
            }
        }
    }
}
