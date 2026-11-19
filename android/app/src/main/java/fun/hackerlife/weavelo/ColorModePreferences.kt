package `fun`.hackerlife.weavelo

import android.content.Context
import android.content.SharedPreferences
import org.json.JSONObject

enum class ChromaMode(val wireValue: String, val displayLabel: String) {
    YUV420("yuv420", "YUV 4:2:0"),
    YUV444("yuv444", "YUV 4:4:4");

    companion object {
        fun fromWireValue(value: String?): ChromaMode {
            val normalized = value?.trim()?.lowercase().orEmpty()
            if (normalized.isEmpty()) return YUV420
            return entries.firstOrNull {
                it.wireValue == normalized || it.name.equals(normalized, ignoreCase = true)
            } ?: YUV420
        }
    }
}

enum class HdrMode(val wireValue: String, val displayLabel: String) {
    SDR("sdr", "SDR"),
    HDR10("hdr10", "HDR10");

    companion object {
        fun fromWireValue(value: String?): HdrMode {
            val normalized = value?.trim()?.lowercase().orEmpty()
            if (normalized.isEmpty()) return SDR
            return entries.firstOrNull {
                it.wireValue == normalized || it.name.equals(normalized, ignoreCase = true)
            } ?: SDR
        }
    }
}

data class ColorProfilePreference(
    val chroma: ChromaMode = ChromaMode.YUV420,
    val hdr: HdrMode = HdrMode.SDR,
) {
    fun toJsonObject(): JSONObject = JSONObject()
        .put(KEY_CHROMA, chroma.wireValue)
        .put(KEY_HDR, hdr.wireValue)

    fun toJson(): String = toJsonObject().toString()

    companion object {
        const val KEY_CHROMA = "chroma"
        const val KEY_HDR = "hdr"

        fun fromJsonObject(json: JSONObject?): ColorProfilePreference {
            if (json == null) return ColorProfilePreference()
            return ColorProfilePreference(
                chroma = ChromaMode.fromWireValue(json.optString(KEY_CHROMA, "")),
                hdr = HdrMode.fromWireValue(json.optString(KEY_HDR, "")),
            )
        }

        fun fromJson(raw: String?): ColorProfilePreference {
            if (raw.isNullOrBlank()) return ColorProfilePreference()
            return runCatching { fromJsonObject(JSONObject(raw)) }
                .getOrDefault(ColorProfilePreference())
        }
    }
}

object ColorModePreferences {
    private const val PREFS_NAME = "weavelo_color_mode_preferences"
    private const val KEY_PROFILE_JSON = "color_profile_json"

    fun load(context: Context): ColorProfilePreference =
        load(context.getSharedPreferences(PREFS_NAME, Context.MODE_PRIVATE))

    fun load(prefs: SharedPreferences): ColorProfilePreference =
        ColorProfilePreference.fromJson(prefs.getString(KEY_PROFILE_JSON, null))

    fun save(context: Context, preference: ColorProfilePreference) {
        save(context.getSharedPreferences(PREFS_NAME, Context.MODE_PRIVATE), preference)
    }

    fun save(prefs: SharedPreferences, preference: ColorProfilePreference) {
        prefs.edit().putString(KEY_PROFILE_JSON, preference.toJson()).apply()
    }
}
