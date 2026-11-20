package `fun`.hackerlife.weavelo

import android.content.Context
import android.content.SharedPreferences
import org.json.JSONArray
import org.json.JSONObject
import java.util.UUID
import kotlin.math.roundToInt

enum class VirtualDisplayPreset(
    val wireValue: String,
    val displayLabel: String,
    val defaultWidth: Int,
    val defaultHeight: Int,
    val defaultRefreshHz: Int = 60,
    val defaultDpiScale: Float = 1.0f,
) {
    FHD_1080P("fhd_1080p", "1080p", 1920, 1080, 60, 1.0f),
    QHD_1440P("qhd_1440p", "2K", 2560, 1440, 60, 1.25f),
    UHD_4K("uhd_4k", "4K", 3840, 2160, 60, 2.0f),
    ULTRAWIDE_21_9("ultrawide_21_9", "21:9", 3440, 1440, 60, 1.0f),
    SUPER_ULTRAWIDE_32_9("super_ultrawide_32_9", "32:9", 5120, 1440, 60, 1.0f),
    MATCH_DEVICE("match_device", "Match Device", 1920, 1080, 60, 1.0f);

    companion object {
        fun fromWireValueOrNull(value: String?): VirtualDisplayPreset? {
            val normalized = value?.trim()?.lowercase().orEmpty()
            if (normalized.isEmpty()) return null
            return entries.firstOrNull {
                it.wireValue == normalized || it.name.equals(normalized, ignoreCase = true)
            }
        }

        fun fromWireValue(value: String?): VirtualDisplayPreset =
            fromWireValueOrNull(value) ?: FHD_1080P
    }
}

enum class DisplayPlacement(
    val wireValue: String,
    val displayLabel: String,
    val shortLabel: String,
) {
    RIGHT_OF("right_of", "Right of Primary", "Right"),
    LEFT_OF("left_of", "Left of Primary", "Left"),
    ABOVE("above", "Above Primary", "Above"),
    BELOW("below", "Below Primary", "Below"),
    MIRROR("mirror", "Mirror Primary", "Mirror"),
    STANDALONE("standalone", "Standalone", "Standalone");

    companion object {
        fun fromWireValueOrNull(value: String?): DisplayPlacement? {
            val normalized = value?.trim()?.lowercase().orEmpty()
            if (normalized.isEmpty()) return null
            return entries.firstOrNull {
                it.wireValue == normalized || it.name.equals(normalized, ignoreCase = true)
            }
        }

        fun fromWireValue(value: String?): DisplayPlacement =
            fromWireValueOrNull(value) ?: RIGHT_OF
    }
}

data class VirtualDisplayProfile(
    val id: String = UUID.randomUUID().toString(),
    val name: String,
    val width: Int,
    val height: Int,
    val refreshHz: Int = 60,
    val dpiScale: Float = 1.0f,
    val placement: DisplayPlacement = DisplayPlacement.RIGHT_OF,
    val preset: VirtualDisplayPreset = VirtualDisplayPreset.FHD_1080P,
    val persistAfterDisconnect: Boolean = false,
) {
    fun normalized(): VirtualDisplayProfile = copy(
        id = id.trim(),
        name = name.trim(),
    )

    fun validate(): String? {
        if (id.isBlank()) return "Profile ID must not be blank"
        if (width !in MIN_WIDTH..MAX_WIDTH) {
            return "Width must be between $MIN_WIDTH and $MAX_WIDTH"
        }
        if (width % 2 != 0) {
            return "Width must be an even number"
        }
        if (height !in MIN_HEIGHT..MAX_HEIGHT) {
            return "Height must be between $MIN_HEIGHT and $MAX_HEIGHT"
        }
        if (height % 2 != 0) {
            return "Height must be an even number"
        }
        if (refreshHz !in MIN_REFRESH_HZ..MAX_REFRESH_HZ) {
            return "Refresh rate must be between $MIN_REFRESH_HZ and $MAX_REFRESH_HZ Hz"
        }
        if (!dpiScale.isFinite() || dpiScale < MIN_DPI_SCALE || dpiScale > MAX_DPI_SCALE) {
            return "DPI scale must be between $MIN_DPI_SCALE and $MAX_DPI_SCALE"
        }
        return null
    }

    fun isValid(): Boolean = validate() == null

    fun resolutionLabel(): String = "${width}x${height} @ ${refreshHz}Hz"

    fun toJsonObject(): JSONObject = JSONObject()
        .put(KEY_ID, id)
        .put(KEY_NAME, name)
        .put(KEY_WIDTH, width)
        .put(KEY_HEIGHT, height)
        .put(KEY_REFRESH_HZ, refreshHz)
        .put(KEY_DPI_SCALE, dpiScale.toDouble())
        .put(KEY_PLACEMENT, placement.wireValue)
        .put(KEY_PRESET, preset.wireValue)
        .put(KEY_PERSIST_AFTER_DISCONNECT, persistAfterDisconnect)

    fun toJson(): String = toJsonObject().toString()

    companion object {
        const val MIN_WIDTH = 320
        const val MAX_WIDTH = 7680
        const val MIN_HEIGHT = 240
        const val MAX_HEIGHT = 4320
        const val MIN_REFRESH_HZ = 24
        const val MAX_REFRESH_HZ = 240
        const val MIN_DPI_SCALE = 0.5f
        const val MAX_DPI_SCALE = 4.0f

        private const val BASELINE_DENSITY_DPI = 160f

        private const val KEY_ID = "id"
        private const val KEY_NAME = "name"
        private const val KEY_WIDTH = "width"
        private const val KEY_HEIGHT = "height"
        private const val KEY_REFRESH_HZ = "refresh_hz"
        private const val KEY_DPI_SCALE = "dpi_scale"
        private const val KEY_PLACEMENT = "placement"
        private const val KEY_PRESET = "preset"
        private const val KEY_PERSIST_AFTER_DISCONNECT = "persist_after_disconnect"

        fun fromPreset(
            preset: VirtualDisplayPreset,
            placement: DisplayPlacement = DisplayPlacement.RIGHT_OF,
            name: String = preset.displayLabel,
            refreshHz: Int = preset.defaultRefreshHz,
            dpiScale: Float = preset.defaultDpiScale,
            persistAfterDisconnect: Boolean = false,
            id: String = UUID.randomUUID().toString(),
        ): VirtualDisplayProfile = VirtualDisplayProfile(
            id = id,
            name = name,
            width = preset.defaultWidth,
            height = preset.defaultHeight,
            refreshHz = refreshHz.coerceIn(MIN_REFRESH_HZ, MAX_REFRESH_HZ),
            dpiScale = dpiScale.coerceIn(MIN_DPI_SCALE, MAX_DPI_SCALE),
            placement = placement,
            preset = preset,
            persistAfterDisconnect = persistAfterDisconnect,
        )

        fun fromDeviceMetrics(
            widthPx: Int,
            heightPx: Int,
            densityDpi: Int,
            refreshHz: Int = 60,
            landscape: Boolean = true,
            name: String = "Wireless Secondary Display",
            placement: DisplayPlacement = DisplayPlacement.RIGHT_OF,
            persistAfterDisconnect: Boolean = false,
            id: String = UUID.randomUUID().toString(),
        ): VirtualDisplayProfile {
            val longSide = maxOf(widthPx, heightPx)
            val shortSide = minOf(widthPx, heightPx)
            val rawWidth = if (landscape) longSide else shortSide
            val rawHeight = if (landscape) shortSide else longSide

            val clampedWidth = alignEven(rawWidth.coerceIn(MIN_WIDTH, MAX_WIDTH), MIN_WIDTH)
            val clampedHeight = alignEven(rawHeight.coerceIn(MIN_HEIGHT, MAX_HEIGHT), MIN_HEIGHT)
            val clampedRefresh = refreshHz.coerceIn(MIN_REFRESH_HZ, MAX_REFRESH_HZ)
            val rawScale = if (densityDpi > 0) densityDpi / BASELINE_DENSITY_DPI else 1.0f
            val quantizedScale = ((rawScale * 4f).roundToInt() / 4f)
                .coerceIn(MIN_DPI_SCALE, MAX_DPI_SCALE)

            return VirtualDisplayProfile(
                id = id,
                name = name,
                width = clampedWidth,
                height = clampedHeight,
                refreshHz = clampedRefresh,
                dpiScale = quantizedScale,
                placement = placement,
                preset = VirtualDisplayPreset.MATCH_DEVICE,
                persistAfterDisconnect = persistAfterDisconnect,
            )
        }

        private fun alignEven(value: Int, minEven: Int): Int {
            val even = value and 1.inv()
            return if (even < minEven) minEven else even
        }

        fun fromJsonObject(json: JSONObject?): VirtualDisplayProfile? {
            if (json == null) return null
            val id = json.optString(KEY_ID, "").trim()
            if (!json.has(KEY_WIDTH) || !json.has(KEY_HEIGHT) || !json.has(KEY_REFRESH_HZ)) {
                return null
            }
            val width = json.optInt(KEY_WIDTH, -1)
            val height = json.optInt(KEY_HEIGHT, -1)
            val refreshHz = json.optInt(KEY_REFRESH_HZ, -1)
            val dpiScale = if (json.has(KEY_DPI_SCALE)) {
                json.optDouble(KEY_DPI_SCALE, Double.NaN).toFloat()
            } else {
                1.0f
            }
            val placement = DisplayPlacement.fromWireValueOrNull(json.optString(KEY_PLACEMENT, "")) ?: return null
            val preset = VirtualDisplayPreset.fromWireValueOrNull(json.optString(KEY_PRESET, "")) ?: return null
            val profile = VirtualDisplayProfile(
                id = id,
                name = json.optString(KEY_NAME, "").trim(),
                width = width,
                height = height,
                refreshHz = refreshHz,
                dpiScale = dpiScale,
                placement = placement,
                preset = preset,
                persistAfterDisconnect = json.optBoolean(KEY_PERSIST_AFTER_DISCONNECT, false),
            )
            return profile.takeIf { it.isValid() }
        }

        fun fromJson(raw: String?): VirtualDisplayProfile? {
            if (raw.isNullOrBlank()) return null
            return runCatching { fromJsonObject(JSONObject(raw)) }.getOrNull()
        }
    }
}

object VirtualDisplayStore {
    private const val PREFS_NAME = "weavelo_virtual_display_store"
    private const val KEY_PROFILES_JSON = "virtual_display_profiles_json"

    fun encodeProfiles(profiles: List<VirtualDisplayProfile>): String {
        val array = JSONArray()
        profiles.forEach { array.put(it.toJsonObject()) }
        return array.toString()
    }

    fun decodeProfiles(raw: String?): List<VirtualDisplayProfile> {
        if (raw.isNullOrBlank()) return emptyList()
        return runCatching {
            val array = JSONArray(raw)
            val accepted = mutableListOf<VirtualDisplayProfile>()
            val seenIds = mutableSetOf<String>()
            for (index in 0 until array.length()) {
                val profile = VirtualDisplayProfile.fromJsonObject(array.optJSONObject(index)) ?: continue
                if (seenIds.add(profile.id)) {
                    accepted += profile
                }
            }
            accepted.toList()
        }.getOrDefault(emptyList())
    }

    fun validateForAdd(
        profile: VirtualDisplayProfile,
        existing: List<VirtualDisplayProfile>,
    ): String? {
        val normalized = profile.normalized()
        normalized.validate()?.let { return it }
        if (existing.any { it.id == normalized.id }) {
            return "A virtual display profile with this ID already exists"
        }
        return null
    }

    fun list(context: Context): List<VirtualDisplayProfile> =
        list(context.getSharedPreferences(PREFS_NAME, Context.MODE_PRIVATE))

    fun list(prefs: SharedPreferences): List<VirtualDisplayProfile> =
        decodeProfiles(prefs.getString(KEY_PROFILES_JSON, null))

    fun saveAll(context: Context, profiles: List<VirtualDisplayProfile>) {
        saveAll(context.getSharedPreferences(PREFS_NAME, Context.MODE_PRIVATE), profiles)
    }

    fun saveAll(prefs: SharedPreferences, profiles: List<VirtualDisplayProfile>) {
        prefs.edit().putString(KEY_PROFILES_JSON, encodeProfiles(profiles)).apply()
    }

    fun add(context: Context, profile: VirtualDisplayProfile): Result<List<VirtualDisplayProfile>> =
        add(context.getSharedPreferences(PREFS_NAME, Context.MODE_PRIVATE), profile)

    fun add(prefs: SharedPreferences, profile: VirtualDisplayProfile): Result<List<VirtualDisplayProfile>> {
        val normalized = profile.normalized()
        val existing = list(prefs)
        validateForAdd(normalized, existing)?.let { error ->
            return Result.failure(IllegalArgumentException(error))
        }
        val updated = existing + normalized
        saveAll(prefs, updated)
        return Result.success(updated)
    }

    fun remove(context: Context, id: String): List<VirtualDisplayProfile> =
        remove(context.getSharedPreferences(PREFS_NAME, Context.MODE_PRIVATE), id)

    fun remove(prefs: SharedPreferences, id: String): List<VirtualDisplayProfile> {
        val updated = list(prefs).filterNot { it.id == id }
        saveAll(prefs, updated)
        return updated
    }
}
