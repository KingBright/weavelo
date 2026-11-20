package `fun`.hackerlife.weavelo.ui

import android.content.Context
import android.os.Build
import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import `fun`.hackerlife.weavelo.DisplayPlacement
import `fun`.hackerlife.weavelo.VirtualDisplayPreset
import `fun`.hackerlife.weavelo.VirtualDisplayProfile
import `fun`.hackerlife.weavelo.VirtualDisplayStore
import java.util.Locale
import kotlin.math.roundToInt

private val StandardDisplayPresets = listOf(
    VirtualDisplayPreset.FHD_1080P,
    VirtualDisplayPreset.QHD_1440P,
    VirtualDisplayPreset.UHD_4K,
    VirtualDisplayPreset.ULTRAWIDE_21_9,
    VirtualDisplayPreset.SUPER_ULTRAWIDE_32_9,
)

private val RefreshRateOptions = listOf(60, 90, 120)

@Composable
fun VirtualDisplayDialog(
    onDismiss: () -> Unit,
    activeProfileId: String? = null,
    onActivateProfile: ((VirtualDisplayProfile) -> Unit)? = null,
    onDeactivateDisplay: (() -> Unit)? = null,
    onProfilesChanged: ((List<VirtualDisplayProfile>) -> Unit)? = null,
) {
    val p = WeaveloTokens.colors
    val context = LocalContext.current
    val appContext = context.applicationContext

    var profiles by remember { mutableStateOf(VirtualDisplayStore.list(appContext)) }
    var wirelessLandscape by remember { mutableStateOf(true) }
    var selectedPreset by remember { mutableStateOf(VirtualDisplayPreset.FHD_1080P) }
    var selectedPlacement by remember { mutableStateOf(DisplayPlacement.RIGHT_OF) }
    var selectedRefreshHz by remember { mutableIntStateOf(60) }
    var persistAfterDisconnect by remember { mutableStateOf(false) }
    var showCreateForm by remember { mutableStateOf(false) }
    var profileNameInput by remember { mutableStateOf("") }
    var formError by remember { mutableStateOf<String?>(null) }

    val deviceProfile = remember(wirelessLandscape, selectedPlacement, persistAfterDisconnect) {
        buildDeviceMatchedProfile(
            context = context,
            landscape = wirelessLandscape,
            placement = selectedPlacement,
            persistAfterDisconnect = persistAfterDisconnect,
        )
    }

    fun resetCreateForm() {
        profileNameInput = ""
        selectedPreset = VirtualDisplayPreset.FHD_1080P
        selectedRefreshHz = 60
        persistAfterDisconnect = false
        formError = null
    }

    PrecisionDialog(
        title = "Virtual Display",
        subtitle = "Turn this device into a wireless secondary display or manage host virtual display profiles.",
        icon = MobileSymbol.Devices,
        maxWidth = 540.dp,
        onDismissRequest = onDismiss,
        confirmButton = {
            PrecisionButton(
                text = tr("Close"),
                style = PrecisionButtonStyle.TONAL,
                compact = true,
                onClick = onDismiss,
            )
        },
    ) {
        Column(verticalArrangement = Arrangement.spacedBy(10.dp)) {
            // One-tap Wireless Secondary Display card matching current Android device
            PrecisionCard(
                highlighted = activeProfileId != null,
                cornerRadius = 14.dp,
                contentPadding = PaddingValues(12.dp),
                verticalSpacing = 8.dp,
            ) {
                Row(
                    modifier = Modifier.fillMaxWidth(),
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(8.dp),
                ) {
                    Column(modifier = Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(2.dp)) {
                        Text(
                            text = "Wireless Secondary Display",
                            color = p.textPrimary,
                            fontSize = 13.5.sp,
                            fontWeight = FontWeight.SemiBold,
                            maxLines = 1,
                            overflow = TextOverflow.Ellipsis,
                        )
                        Text(
                            text = "${deviceProfile.resolutionLabel()} · ${formatDpiScale(deviceProfile.dpiScale)} · ${selectedPlacement.displayLabel}",
                            color = p.textSecondary,
                            fontSize = 11.5.sp,
                            fontFamily = FontFamily.Monospace,
                            maxLines = 1,
                            overflow = TextOverflow.Ellipsis,
                        )
                    }
                    PrecisionPillBadge(
                        text = if (activeProfileId != null) "Active" else "Match Device",
                        tone = if (activeProfileId != null) PillTone.EMERALD else PillTone.BRAND,
                        showDot = activeProfileId != null,
                    )
                }

                Row(
                    modifier = Modifier.fillMaxWidth(),
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(8.dp),
                ) {
                    PrecisionSegmentedRow(
                        options = listOf(true to "Landscape", false to "Portrait"),
                        selected = wirelessLandscape,
                        onSelect = { wirelessLandscape = it },
                        modifier = Modifier.weight(1f),
                    )
                    if (activeProfileId != null && onDeactivateDisplay != null) {
                        PrecisionButton(
                            text = "Stop",
                            style = PrecisionButtonStyle.DANGER_TONAL,
                            compact = true,
                            onClick = onDeactivateDisplay,
                        )
                    }
                    PrecisionButton(
                        text = "Activate",
                        leadingSymbol = MobileSymbol.View,
                        style = PrecisionButtonStyle.PRIMARY,
                        compact = true,
                        onClick = { onActivateProfile?.invoke(deviceProfile) },
                    )
                }
            }

            // Placement selector (shared by quick wireless activation & new profile)
            Column(verticalArrangement = Arrangement.spacedBy(6.dp)) {
                Text(
                    text = "Display Placement",
                    color = p.textSecondary,
                    fontSize = 11.5.sp,
                    fontWeight = FontWeight.SemiBold,
                )
                PrecisionSegmentedRow(
                    options = listOf(
                        DisplayPlacement.RIGHT_OF to DisplayPlacement.RIGHT_OF.shortLabel,
                        DisplayPlacement.LEFT_OF to DisplayPlacement.LEFT_OF.shortLabel,
                        DisplayPlacement.ABOVE to DisplayPlacement.ABOVE.shortLabel,
                    ),
                    selected = selectedPlacement,
                    onSelect = { selectedPlacement = it },
                    modifier = Modifier.fillMaxWidth(),
                )
                PrecisionSegmentedRow(
                    options = listOf(
                        DisplayPlacement.BELOW to DisplayPlacement.BELOW.shortLabel,
                        DisplayPlacement.MIRROR to DisplayPlacement.MIRROR.shortLabel,
                        DisplayPlacement.STANDALONE to DisplayPlacement.STANDALONE.shortLabel,
                    ),
                    selected = selectedPlacement,
                    onSelect = { selectedPlacement = it },
                    modifier = Modifier.fillMaxWidth(),
                )
            }

            // Profiles header & add toggle
            Row(
                modifier = Modifier.fillMaxWidth(),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                PrecisionPillBadge(
                    text = "${profiles.size} saved",
                    tone = if (profiles.isNotEmpty()) PillTone.CYAN else PillTone.NEUTRAL,
                    monospace = true,
                )
                Spacer(Modifier.weight(1f))
                PrecisionButton(
                    text = if (showCreateForm) tr("Cancel") else "New Profile",
                    style = if (showCreateForm) PrecisionButtonStyle.SUBTLE else PrecisionButtonStyle.TONAL,
                    compact = true,
                    onClick = {
                        if (showCreateForm) {
                            resetCreateForm()
                            showCreateForm = false
                        } else {
                            formError = null
                            showCreateForm = true
                        }
                    },
                )
            }

            if (showCreateForm) {
                PrecisionCard(
                    sunken = true,
                    cornerRadius = 14.dp,
                    contentPadding = PaddingValues(12.dp),
                    verticalSpacing = 8.dp,
                ) {
                    Text(
                        text = "Resolution Preset",
                        color = p.textSecondary,
                        fontSize = 11.5.sp,
                        fontWeight = FontWeight.SemiBold,
                    )
                    PrecisionSegmentedRow(
                        options = StandardDisplayPresets.take(3).map { it to it.displayLabel },
                        selected = selectedPreset,
                        onSelect = { selectedPreset = it; formError = null },
                        modifier = Modifier.fillMaxWidth(),
                    )
                    PrecisionSegmentedRow(
                        options = StandardDisplayPresets.drop(3).map { it to it.displayLabel },
                        selected = selectedPreset,
                        onSelect = { selectedPreset = it; formError = null },
                        modifier = Modifier.fillMaxWidth(),
                    )
                    Text(
                        text = "${selectedPreset.defaultWidth}x${selectedPreset.defaultHeight} · ${formatDpiScale(selectedPreset.defaultDpiScale)}",
                        color = p.textTertiary,
                        fontSize = 11.5.sp,
                        fontFamily = FontFamily.Monospace,
                    )
                    PrecisionSegmentedRow(
                        options = RefreshRateOptions.map { it to "${it}Hz" },
                        selected = selectedRefreshHz,
                        onSelect = { selectedRefreshHz = it; formError = null },
                        modifier = Modifier.fillMaxWidth(),
                    )
                    PrecisionTextField(
                        value = profileNameInput,
                        onValueChange = { profileNameInput = it; formError = null },
                        label = "Profile name (optional)",
                    )
                    PrecisionSwitchRow(
                        title = "Persist after disconnect",
                        subtitle = "Keep this virtual display attached on the host when disconnected.",
                        checked = persistAfterDisconnect,
                        onCheckedChange = { persistAfterDisconnect = it },
                    )
                    formError?.let { message ->
                        Text(
                            text = message,
                            color = p.dangerText,
                            fontSize = 12.sp,
                        )
                    }
                    Row(
                        modifier = Modifier.fillMaxWidth(),
                        horizontalArrangement = Arrangement.spacedBy(8.dp, Alignment.End),
                        verticalAlignment = Alignment.CenterVertically,
                    ) {
                        PrecisionButton(
                            text = tr("Save"),
                            leadingSymbol = MobileSymbol.Check,
                            style = PrecisionButtonStyle.PRIMARY,
                            compact = true,
                            onClick = {
                                val displayName = profileNameInput.trim().ifEmpty {
                                    "${selectedPreset.displayLabel} ${selectedPlacement.shortLabel}"
                                }
                                val profile = VirtualDisplayProfile.fromPreset(
                                    preset = selectedPreset,
                                    placement = selectedPlacement,
                                    name = displayName,
                                    refreshHz = selectedRefreshHz,
                                    persistAfterDisconnect = persistAfterDisconnect,
                                )
                                VirtualDisplayStore.add(appContext, profile)
                                    .onSuccess { updated ->
                                        profiles = updated
                                        onProfilesChanged?.invoke(updated)
                                        resetCreateForm()
                                        showCreateForm = false
                                    }
                                    .onFailure { failure ->
                                        formError = failure.message ?: "Invalid virtual display profile"
                                    }
                            },
                        )
                    }
                }
            }

            if (profiles.isEmpty()) {
                Box(
                    modifier = Modifier
                        .fillMaxWidth()
                        .clip(RoundedCornerShape(12.dp))
                        .background(p.surfaceSunken)
                        .border(BorderStroke(1.dp, p.borderMuted), RoundedCornerShape(12.dp))
                        .padding(14.dp),
                    contentAlignment = Alignment.Center,
                ) {
                    Text(
                        text = "No saved virtual display profiles yet.",
                        color = p.textSecondary,
                        fontSize = 12.5.sp,
                    )
                }
            } else {
                LazyColumn(
                    modifier = Modifier.heightIn(max = 260.dp),
                    verticalArrangement = Arrangement.spacedBy(8.dp),
                ) {
                    items(profiles, key = { it.id }) { profile ->
                        VirtualDisplayProfileRow(
                            profile = profile,
                            isActive = profile.id == activeProfileId,
                            onActivate = { onActivateProfile?.invoke(profile) },
                            onDelete = {
                                val updated = VirtualDisplayStore.remove(appContext, profile.id)
                                profiles = updated
                                onProfilesChanged?.invoke(updated)
                            },
                        )
                    }
                }
            }
        }
    }
}

@Composable
private fun VirtualDisplayProfileRow(
    profile: VirtualDisplayProfile,
    isActive: Boolean,
    onActivate: () -> Unit,
    onDelete: () -> Unit,
) {
    val p = WeaveloTokens.colors
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .clip(RoundedCornerShape(12.dp))
            .background(if (isActive) p.brandSoft else p.surface)
            .border(
                BorderStroke(1.dp, if (isActive) p.brandBorder else p.border),
                RoundedCornerShape(12.dp),
            )
            .padding(horizontal = 12.dp, vertical = 10.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        Column(
            modifier = Modifier.weight(1f),
            verticalArrangement = Arrangement.spacedBy(4.dp),
        ) {
            Row(
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(6.dp),
            ) {
                PrecisionPillBadge(
                    text = profile.preset.displayLabel,
                    tone = if (isActive) PillTone.EMERALD else PillTone.BRAND,
                    showDot = isActive,
                    monospace = true,
                )
                PrecisionPillBadge(
                    text = profile.placement.shortLabel,
                    tone = PillTone.CYAN,
                )
                Text(
                    text = profile.name.ifBlank { profile.preset.displayLabel },
                    color = p.textPrimary,
                    fontSize = 13.sp,
                    fontWeight = FontWeight.SemiBold,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
            }
            Text(
                text = buildString {
                    append(profile.resolutionLabel())
                    append(" · ")
                    append(formatDpiScale(profile.dpiScale))
                    if (profile.persistAfterDisconnect) {
                        append(" · Persistent")
                    }
                },
                color = p.textTertiary,
                fontSize = 11.5.sp,
                fontFamily = FontFamily.Monospace,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
        }
        PrecisionButton(
            text = if (isActive) "Active" else "Use",
            style = if (isActive) PrecisionButtonStyle.TONAL else PrecisionButtonStyle.SUBTLE,
            compact = true,
            enabled = !isActive,
            onClick = onActivate,
        )
        MobileAction(
            symbol = MobileSymbol.Close,
            label = tr("Delete"),
            destructive = true,
            modifier = Modifier.size(34.dp),
            onClick = onDelete,
        )
    }
}

private fun buildDeviceMatchedProfile(
    context: Context,
    landscape: Boolean,
    placement: DisplayPlacement,
    persistAfterDisconnect: Boolean,
): VirtualDisplayProfile {
    val metrics = context.resources.displayMetrics
    val refreshHz = runCatching {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
            context.display?.refreshRate?.roundToInt() ?: 60
        } else {
            60
        }
    }.getOrDefault(60)
    return VirtualDisplayProfile.fromDeviceMetrics(
        widthPx = metrics.widthPixels,
        heightPx = metrics.heightPixels,
        densityDpi = metrics.densityDpi,
        refreshHz = refreshHz,
        landscape = landscape,
        placement = placement,
        persistAfterDisconnect = persistAfterDisconnect,
    )
}

private fun formatDpiScale(scale: Float): String {
    val formatted = String.format(Locale.US, "%.2f", scale)
        .trimEnd('0')
        .trimEnd('.')
    return "${formatted}x"
}
