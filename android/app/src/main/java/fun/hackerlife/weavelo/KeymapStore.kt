package `fun`.hackerlife.weavelo

import android.content.Context
import android.content.SharedPreferences
import org.json.JSONArray
import org.json.JSONObject
import java.util.Locale
import java.util.UUID

enum class KeymapCategory(
    val wireValue: String,
    val displayLabel: String,
) {
    FPS("fps", "FPS"),
    MOBA("moba", "MOBA"),
    RPG("rpg", "RPG"),
    ACTION("action", "Action"),
    RACING("racing", "Racing"),
    SIMULATOR("simulator", "Simulator"),
    PRODUCTIVITY("productivity", "Productivity"),
    MEDIA("media", "Media"),
    CUSTOM("custom", "Custom");

    companion object {
        fun fromWireValueOrNull(value: String?): KeymapCategory? {
            val normalized = value?.trim()?.lowercase(Locale.ROOT).orEmpty()
            if (normalized.isEmpty()) return null
            return entries.firstOrNull {
                it.wireValue == normalized || it.name.equals(normalized, ignoreCase = true)
            }
        }

        fun fromWireValue(value: String?): KeymapCategory =
            fromWireValueOrNull(value) ?: CUSTOM
    }
}

enum class VirtualControlKind(
    val wireValue: String,
    val displayLabel: String,
) {
    JOYSTICK_WASD("joystick_wasd", "WASD Stick"),
    LOOK_PAD("look_pad", "Look Pad"),
    KEY_BUTTON("key_button", "Key"),
    COMBO_BUTTON("combo_button", "Combo"),
    MACRO_BURST("macro_burst", "Burst"),
    SCROLL_WHEEL("scroll_wheel", "Scroll"),
    MOUSE_BUTTON("mouse_button", "Mouse");

    companion object {
        fun fromWireValueOrNull(value: String?): VirtualControlKind? {
            val normalized = value?.trim()?.lowercase(Locale.ROOT).orEmpty()
            if (normalized.isEmpty()) return null
            return entries.firstOrNull {
                it.wireValue == normalized || it.name.equals(normalized, ignoreCase = true)
            }
        }

        fun fromWireValue(value: String?): VirtualControlKind =
            fromWireValueOrNull(value) ?: KEY_BUTTON
    }
}

data class KeymapControl(
    val id: String,
    val kind: VirtualControlKind,
    val label: String,
    val x: Float,
    val y: Float,
    val size: Float = DEFAULT_BUTTON_SIZE,
    val keys: List<String> = emptyList(),
    val opacity: Float = DEFAULT_OPACITY,
    val toggle: Boolean = false,
    val burstIntervalMs: Int = DEFAULT_BURST_INTERVAL_MS,
    val sensitivity: Float = DEFAULT_SENSITIVITY,
) {
    fun normalized(): KeymapControl = copy(
        id = id.trim(),
        label = label.trim(),
        keys = keys.map { it.trim() }.filter { it.isNotEmpty() },
    )

    fun validate(): String? {
        if (id.isBlank()) return "Control ID must not be blank"
        if (label.isBlank()) return "Control label must not be blank"
        if (!x.isFinite() || x < MIN_COORD || x > MAX_COORD) {
            return "Control x must be between $MIN_COORD and $MAX_COORD"
        }
        if (!y.isFinite() || y < MIN_COORD || y > MAX_COORD) {
            return "Control y must be between $MIN_COORD and $MAX_COORD"
        }
        if (!size.isFinite() || size < MIN_SIZE || size > MAX_SIZE) {
            return "Control size must be between $MIN_SIZE and $MAX_SIZE"
        }
        if (!opacity.isFinite() || opacity < MIN_OPACITY || opacity > MAX_OPACITY) {
            return "Control opacity must be between $MIN_OPACITY and $MAX_OPACITY"
        }
        if (burstIntervalMs !in MIN_BURST_INTERVAL_MS..MAX_BURST_INTERVAL_MS) {
            return "Burst interval must be between $MIN_BURST_INTERVAL_MS and $MAX_BURST_INTERVAL_MS ms"
        }
        if (!sensitivity.isFinite() || sensitivity < MIN_SENSITIVITY || sensitivity > MAX_SENSITIVITY) {
            return "Sensitivity must be between $MIN_SENSITIVITY and $MAX_SENSITIVITY"
        }
        if (keys.any { it.isBlank() }) {
            return "Control keys must not contain blank entries"
        }
        when (kind) {
            VirtualControlKind.KEY_BUTTON,
            VirtualControlKind.COMBO_BUTTON,
            VirtualControlKind.MACRO_BURST,
            VirtualControlKind.MOUSE_BUTTON -> {
                if (keys.isEmpty()) return "Control $id (${kind.wireValue}) requires at least one key"
            }
            VirtualControlKind.JOYSTICK_WASD -> {
                if (keys.isNotEmpty() && keys.size != 4) {
                    return "Joystick $id must specify either 0 (default WASD) or 4 directional keys"
                }
            }
            VirtualControlKind.LOOK_PAD,
            VirtualControlKind.SCROLL_WHEEL -> Unit
        }
        return null
    }

    fun isValid(): Boolean = validate() == null

    fun effectiveDirectionalKeys(): List<String> =
        if (keys.size == 4) keys else DEFAULT_WASD_KEYS

    fun toJsonObject(): JSONObject = JSONObject()
        .put(KEY_ID, id)
        .put(KEY_KIND, kind.wireValue)
        .put(KEY_LABEL, label)
        .put(KEY_X, x.toDouble())
        .put(KEY_Y, y.toDouble())
        .put(KEY_SIZE, size.toDouble())
        .put(KEY_KEYS, JSONArray(keys))
        .put(KEY_OPACITY, opacity.toDouble())
        .put(KEY_TOGGLE, toggle)
        .put(KEY_BURST_INTERVAL_MS, burstIntervalMs)
        .put(KEY_SENSITIVITY, sensitivity.toDouble())

    fun toJson(): String = toJsonObject().toString()

    companion object {
        const val MIN_COORD = 0.0f
        const val MAX_COORD = 1.0f
        const val MIN_SIZE = 0.03f
        const val MAX_SIZE = 0.45f
        const val MIN_OPACITY = 0.10f
        const val MAX_OPACITY = 1.0f
        const val MIN_BURST_INTERVAL_MS = 10
        const val MAX_BURST_INTERVAL_MS = 2000
        const val MIN_SENSITIVITY = 0.1f
        const val MAX_SENSITIVITY = 5.0f

        const val DEFAULT_BUTTON_SIZE = 0.085f
        const val DEFAULT_JOYSTICK_SIZE = 0.22f
        const val DEFAULT_OPACITY = 0.78f
        const val DEFAULT_BURST_INTERVAL_MS = 60
        const val DEFAULT_SENSITIVITY = 1.0f

        val DEFAULT_WASD_KEYS = listOf("w", "a", "s", "d")

        private const val KEY_ID = "id"
        private const val KEY_KIND = "kind"
        private const val KEY_LABEL = "label"
        private const val KEY_X = "x"
        private const val KEY_Y = "y"
        private const val KEY_SIZE = "size"
        private const val KEY_KEYS = "keys"
        private const val KEY_OPACITY = "opacity"
        private const val KEY_TOGGLE = "toggle"
        private const val KEY_BURST_INTERVAL_MS = "burst_interval_ms"
        private const val KEY_SENSITIVITY = "sensitivity"

        fun fromJsonObject(json: JSONObject?): KeymapControl? {
            if (json == null) return null
            val id = json.optString(KEY_ID, "").trim()
            val kind = VirtualControlKind.fromWireValueOrNull(json.optString(KEY_KIND, "")) ?: return null
            val label = json.optString(KEY_LABEL, "").trim()
            if (!json.has(KEY_X) || !json.has(KEY_Y)) return null
            val x = json.optDouble(KEY_X, Double.NaN).toFloat()
            val y = json.optDouble(KEY_Y, Double.NaN).toFloat()
            val size = if (json.has(KEY_SIZE)) {
                json.optDouble(KEY_SIZE, Double.NaN).toFloat()
            } else {
                DEFAULT_BUTTON_SIZE
            }
            val opacity = if (json.has(KEY_OPACITY)) {
                json.optDouble(KEY_OPACITY, Double.NaN).toFloat()
            } else {
                DEFAULT_OPACITY
            }
            val burstIntervalMs = json.optInt(KEY_BURST_INTERVAL_MS, DEFAULT_BURST_INTERVAL_MS)
            val sensitivity = if (json.has(KEY_SENSITIVITY)) {
                json.optDouble(KEY_SENSITIVITY, Double.NaN).toFloat()
            } else {
                DEFAULT_SENSITIVITY
            }
            val keysArray = json.optJSONArray(KEY_KEYS)
            val keys = buildList {
                if (keysArray != null) {
                    for (i in 0 until keysArray.length()) {
                        val item = keysArray.optString(i, "").trim()
                        if (item.isNotEmpty()) add(item)
                    }
                }
            }
            val control = KeymapControl(
                id = id,
                kind = kind,
                label = label,
                x = x,
                y = y,
                size = size,
                keys = keys,
                opacity = opacity,
                toggle = json.optBoolean(KEY_TOGGLE, false),
                burstIntervalMs = burstIntervalMs,
                sensitivity = sensitivity,
            )
            return control.takeIf { it.isValid() }
        }

        fun fromJson(raw: String?): KeymapControl? {
            if (raw.isNullOrBlank()) return null
            return runCatching { fromJsonObject(JSONObject(raw)) }.getOrNull()
        }
    }
}

data class KeymapProfile(
    val id: String = UUID.randomUUID().toString(),
    val name: String,
    val category: KeymapCategory = KeymapCategory.CUSTOM,
    val controls: List<KeymapControl>,
    val processNames: List<String> = emptyList(),
    val tags: List<String> = emptyList(),
    val opacity: Float = DEFAULT_PROFILE_OPACITY,
    val builtIn: Boolean = false,
    val description: String = "",
    val schemaVersion: Int = CURRENT_SCHEMA_VERSION,
) {
    fun normalized(): KeymapProfile = copy(
        id = id.trim(),
        name = name.trim(),
        controls = controls.map { it.normalized() },
        processNames = processNames.map { it.trim().lowercase(Locale.ROOT) }.filter { it.isNotEmpty() }.distinct(),
        tags = tags.map { it.trim() }.filter { it.isNotEmpty() }.distinct(),
        description = description.trim(),
    )

    fun validate(): String? {
        if (schemaVersion != CURRENT_SCHEMA_VERSION) {
            return "Unsupported keymap schema version: $schemaVersion"
        }
        if (id.isBlank()) return "Profile ID must not be blank"
        if (name.isBlank()) return "Profile name must not be blank"
        if (!opacity.isFinite() || opacity < KeymapControl.MIN_OPACITY || opacity > KeymapControl.MAX_OPACITY) {
            return "Profile opacity must be between ${KeymapControl.MIN_OPACITY} and ${KeymapControl.MAX_OPACITY}"
        }
        if (controls.isEmpty()) return "Profile must contain at least one control"
        val seenControlIds = HashSet<String>(controls.size)
        for (control in controls) {
            control.validate()?.let { return it }
            if (!seenControlIds.add(control.id)) {
                return "Duplicate control ID in profile: ${control.id}"
            }
        }
        return null
    }

    fun isValid(): Boolean = validate() == null

    fun toJsonObject(): JSONObject {
        val controlsArray = JSONArray()
        controls.forEach { controlsArray.put(it.toJsonObject()) }
        return JSONObject()
            .put(KEY_SCHEMA_VERSION, schemaVersion)
            .put(KEY_ID, id)
            .put(KEY_NAME, name)
            .put(KEY_CATEGORY, category.wireValue)
            .put(KEY_CONTROLS, controlsArray)
            .put(KEY_PROCESS_NAMES, JSONArray(processNames))
            .put(KEY_TAGS, JSONArray(tags))
            .put(KEY_OPACITY, opacity.toDouble())
            .put(KEY_BUILT_IN, builtIn)
            .put(KEY_DESCRIPTION, description)
    }

    fun toJson(): String = toJsonObject().toString()

    companion object {
        const val CURRENT_SCHEMA_VERSION = 1
        const val DEFAULT_PROFILE_OPACITY = 0.78f

        private const val KEY_SCHEMA_VERSION = "schema_version"
        private const val KEY_ID = "id"
        private const val KEY_NAME = "name"
        private const val KEY_CATEGORY = "category"
        private const val KEY_CONTROLS = "controls"
        private const val KEY_PROCESS_NAMES = "process_names"
        private const val KEY_TAGS = "tags"
        private const val KEY_OPACITY = "opacity"
        private const val KEY_BUILT_IN = "built_in"
        private const val KEY_DESCRIPTION = "description"

        fun fromJsonObject(json: JSONObject?): KeymapProfile? {
            if (json == null) return null
            val schemaVersion = json.optInt(KEY_SCHEMA_VERSION, CURRENT_SCHEMA_VERSION)
            if (schemaVersion != CURRENT_SCHEMA_VERSION) return null
            val id = json.optString(KEY_ID, "").trim()
            val name = json.optString(KEY_NAME, "").trim()
            val category = KeymapCategory.fromWireValueOrNull(json.optString(KEY_CATEGORY, "")) ?: return null
            val opacity = if (json.has(KEY_OPACITY)) {
                json.optDouble(KEY_OPACITY, Double.NaN).toFloat()
            } else {
                DEFAULT_PROFILE_OPACITY
            }
            val controlsJson = json.optJSONArray(KEY_CONTROLS) ?: return null
            val controls = buildList {
                for (i in 0 until controlsJson.length()) {
                    val control = KeymapControl.fromJsonObject(controlsJson.optJSONObject(i)) ?: return null
                    add(control)
                }
            }
            val processNames = readStringList(json.optJSONArray(KEY_PROCESS_NAMES))
            val tags = readStringList(json.optJSONArray(KEY_TAGS))
            val profile = KeymapProfile(
                id = id,
                name = name,
                category = category,
                controls = controls,
                processNames = processNames,
                tags = tags,
                opacity = opacity,
                builtIn = json.optBoolean(KEY_BUILT_IN, false),
                description = json.optString(KEY_DESCRIPTION, "").trim(),
                schemaVersion = schemaVersion,
            )
            return profile.takeIf { it.isValid() }
        }

        fun fromJson(raw: String?): KeymapProfile? {
            if (raw.isNullOrBlank()) return null
            return runCatching { fromJsonObject(JSONObject(raw)) }.getOrNull()
        }

        private fun readStringList(array: JSONArray?): List<String> {
            if (array == null) return emptyList()
            return buildList {
                for (i in 0 until array.length()) {
                    val item = array.optString(i, "").trim()
                    if (item.isNotEmpty()) add(item)
                }
            }
        }
    }
}

object KeymapStore {
    private const val PREFS_NAME = "weavelo_keymap_store"
    private const val KEY_CUSTOM_PROFILES_JSON = "custom_keymap_profiles_json"
    private const val KEY_ACTIVE_PROFILE_ID = "active_keymap_profile_id"
    private const val KEY_HUD_ENABLED = "keymap_hud_enabled"

    fun encodeProfiles(profiles: List<KeymapProfile>): String {
        val array = JSONArray()
        profiles.forEach { array.put(it.toJsonObject()) }
        return array.toString()
    }

    fun decodeProfiles(raw: String?): List<KeymapProfile> {
        if (raw.isNullOrBlank()) return emptyList()
        return runCatching {
            val array = JSONArray(raw)
            val accepted = mutableListOf<KeymapProfile>()
            val seenIds = mutableSetOf<String>()
            for (index in 0 until array.length()) {
                val profile = KeymapProfile.fromJsonObject(array.optJSONObject(index)) ?: continue
                if (seenIds.add(profile.id)) {
                    accepted += profile
                }
            }
            accepted.toList()
        }.getOrDefault(emptyList())
    }

    fun listCustom(context: Context): List<KeymapProfile> =
        listCustom(context.getSharedPreferences(PREFS_NAME, Context.MODE_PRIVATE))

    fun listCustom(prefs: SharedPreferences): List<KeymapProfile> =
        decodeProfiles(prefs.getString(KEY_CUSTOM_PROFILES_JSON, null))

    fun saveCustomAll(context: Context, profiles: List<KeymapProfile>) {
        saveCustomAll(context.getSharedPreferences(PREFS_NAME, Context.MODE_PRIVATE), profiles)
    }

    fun saveCustomAll(prefs: SharedPreferences, profiles: List<KeymapProfile>) {
        prefs.edit().putString(KEY_CUSTOM_PROFILES_JSON, encodeProfiles(profiles)).apply()
    }

    fun upsertCustom(context: Context, profile: KeymapProfile): Result<List<KeymapProfile>> =
        upsertCustom(context.getSharedPreferences(PREFS_NAME, Context.MODE_PRIVATE), profile)

    fun upsertCustom(prefs: SharedPreferences, profile: KeymapProfile): Result<List<KeymapProfile>> {
        val normalized = profile.normalized().copy(builtIn = false)
        normalized.validate()?.let { error ->
            return Result.failure(IllegalArgumentException(error))
        }
        val existing = listCustom(prefs)
        val idx = existing.indexOfFirst { it.id == normalized.id }
        val updated = if (idx >= 0) {
            existing.toMutableList().apply { set(idx, normalized) }
        } else {
            existing + normalized
        }
        saveCustomAll(prefs, updated)
        return Result.success(updated)
    }

    fun removeCustom(context: Context, id: String): List<KeymapProfile> =
        removeCustom(context.getSharedPreferences(PREFS_NAME, Context.MODE_PRIVATE), id)

    fun removeCustom(prefs: SharedPreferences, id: String): List<KeymapProfile> {
        val updated = listCustom(prefs).filterNot { it.id == id }
        saveCustomAll(prefs, updated)
        if (getActiveProfileId(prefs) == id) {
            setActiveProfileId(prefs, null)
        }
        return updated
    }

    fun getActiveProfileId(context: Context): String? =
        getActiveProfileId(context.getSharedPreferences(PREFS_NAME, Context.MODE_PRIVATE))

    fun getActiveProfileId(prefs: SharedPreferences): String? =
        prefs.getString(KEY_ACTIVE_PROFILE_ID, null)?.trim()?.ifEmpty { null }

    fun setActiveProfileId(context: Context, id: String?) {
        setActiveProfileId(context.getSharedPreferences(PREFS_NAME, Context.MODE_PRIVATE), id)
    }

    fun setActiveProfileId(prefs: SharedPreferences, id: String?) {
        val normalized = id?.trim()?.ifEmpty { null }
        val editor = prefs.edit()
        if (normalized == null) {
            editor.remove(KEY_ACTIVE_PROFILE_ID)
        } else {
            editor.putString(KEY_ACTIVE_PROFILE_ID, normalized)
        }
        editor.apply()
    }

    fun isHudEnabled(context: Context): Boolean =
        isHudEnabled(context.getSharedPreferences(PREFS_NAME, Context.MODE_PRIVATE))

    fun isHudEnabled(prefs: SharedPreferences): Boolean =
        prefs.getBoolean(KEY_HUD_ENABLED, false)

    fun setHudEnabled(context: Context, enabled: Boolean) {
        setHudEnabled(context.getSharedPreferences(PREFS_NAME, Context.MODE_PRIVATE), enabled)
    }

    fun setHudEnabled(prefs: SharedPreferences, enabled: Boolean) {
        prefs.edit().putBoolean(KEY_HUD_ENABLED, enabled).apply()
    }

    fun resolveProfile(context: Context, id: String?): KeymapProfile? =
        resolveProfile(context.getSharedPreferences(PREFS_NAME, Context.MODE_PRIVATE), id)

    fun resolveProfile(prefs: SharedPreferences, id: String?): KeymapProfile? {
        val target = id?.trim()?.ifEmpty { null } ?: return null
        return listCustom(prefs).firstOrNull { it.id == target }
            ?: KeymapCatalog.findById(target)
    }

    fun getActiveProfile(context: Context): KeymapProfile? =
        getActiveProfile(context.getSharedPreferences(PREFS_NAME, Context.MODE_PRIVATE))

    fun getActiveProfile(prefs: SharedPreferences): KeymapProfile? =
        resolveProfile(prefs, getActiveProfileId(prefs))

    fun allProfiles(context: Context): List<KeymapProfile> =
        allProfiles(context.getSharedPreferences(PREFS_NAME, Context.MODE_PRIVATE))

    fun allProfiles(prefs: SharedPreferences): List<KeymapProfile> {
        val custom = listCustom(prefs)
        val customIds = custom.mapTo(HashSet()) { it.id }
        return custom + KeymapCatalog.all().filterNot { it.id in customIds }
    }
}

object KeymapCatalog {
    private enum class Archetype {
        FPS_TACTICAL,
        FPS_BATTLE_ROYALE,
        FPS_HERO,
        FPS_ARENA,
        MOBA_CLASSIC,
        MOBA_BRAWLER,
        RPG_OPEN_WORLD,
        RPG_ARPG,
        RPG_MMO,
        RPG_TURN_BASED,
        ACTION_SOULS,
        ACTION_HACK_SLASH,
        ACTION_PLATFORMER,
        ACTION_STEALTH,
        ACTION_FIGHTING,
        RACING_ARCADE,
        RACING_SIM,
        SIM_FLIGHT,
        SIM_STRATEGY,
        SIM_SANDBOX,
        PROD_IDE,
        PROD_CREATIVE,
        PROD_OFFICE,
        PROD_TERMINAL,
        MEDIA_PLAYER,
        MEDIA_EDITOR,
    }

    private data class CatalogEntry(
        val slug: String,
        val name: String,
        val category: KeymapCategory,
        val archetype: Archetype,
        val processes: List<String>,
        val tags: List<String> = emptyList(),
    )

    private val archetypeControls: Map<Archetype, List<KeymapControl>> by lazy {
        mapOf(
            Archetype.FPS_TACTICAL to listOf(
                KeymapControl("wasd", VirtualControlKind.JOYSTICK_WASD, "WASD", 0.14f, 0.74f, 0.22f, listOf("w", "a", "s", "d")),
                KeymapControl("look", VirtualControlKind.LOOK_PAD, "Aim", 0.56f, 0.48f, 0.28f, emptyList(), opacity = 0.32f, sensitivity = 1.15f),
                KeymapControl("fire", VirtualControlKind.MOUSE_BUTTON, "Fire", 0.88f, 0.72f, 0.11f, listOf("mouse_left")),
                KeymapControl("ads", VirtualControlKind.MOUSE_BUTTON, "ADS", 0.76f, 0.56f, 0.085f, listOf("mouse_right")),
                KeymapControl("jump", VirtualControlKind.KEY_BUTTON, "Jump", 0.89f, 0.50f, 0.08f, listOf("space")),
                KeymapControl("crouch", VirtualControlKind.KEY_BUTTON, "Crouch", 0.78f, 0.84f, 0.075f, listOf("ctrl"), toggle = true),
                KeymapControl("reload", VirtualControlKind.KEY_BUTTON, "R", 0.68f, 0.76f, 0.075f, listOf("r")),
                KeymapControl("use", VirtualControlKind.KEY_BUTTON, "E", 0.66f, 0.58f, 0.07f, listOf("e")),
                KeymapControl("shift", VirtualControlKind.KEY_BUTTON, "Walk", 0.12f, 0.48f, 0.075f, listOf("shift"), toggle = true),
                KeymapControl("tab", VirtualControlKind.KEY_BUTTON, "Tab", 0.50f, 0.10f, 0.07f, listOf("tab")),
            ),
            Archetype.FPS_BATTLE_ROYALE to listOf(
                KeymapControl("wasd", VirtualControlKind.JOYSTICK_WASD, "WASD", 0.14f, 0.74f, 0.22f, listOf("w", "a", "s", "d")),
                KeymapControl("look", VirtualControlKind.LOOK_PAD, "Aim", 0.56f, 0.46f, 0.28f, emptyList(), opacity = 0.30f, sensitivity = 1.2f),
                KeymapControl("fire", VirtualControlKind.MOUSE_BUTTON, "Fire", 0.88f, 0.72f, 0.11f, listOf("mouse_left")),
                KeymapControl("ads", VirtualControlKind.MOUSE_BUTTON, "Scope", 0.76f, 0.54f, 0.085f, listOf("mouse_right")),
                KeymapControl("jump", VirtualControlKind.KEY_BUTTON, "Jump", 0.89f, 0.48f, 0.08f, listOf("space")),
                KeymapControl("crouch", VirtualControlKind.KEY_BUTTON, "C", 0.78f, 0.84f, 0.075f, listOf("c")),
                KeymapControl("reload", VirtualControlKind.KEY_BUTTON, "R", 0.68f, 0.76f, 0.075f, listOf("r")),
                KeymapControl("interact", VirtualControlKind.KEY_BUTTON, "F", 0.64f, 0.58f, 0.075f, listOf("f")),
                KeymapControl("heal", VirtualControlKind.KEY_BUTTON, "4", 0.28f, 0.86f, 0.07f, listOf("4")),
                KeymapControl("map", VirtualControlKind.KEY_BUTTON, "M", 0.90f, 0.14f, 0.07f, listOf("m")),
            ),
            Archetype.FPS_HERO to listOf(
                KeymapControl("wasd", VirtualControlKind.JOYSTICK_WASD, "WASD", 0.14f, 0.74f, 0.22f, listOf("w", "a", "s", "d")),
                KeymapControl("look", VirtualControlKind.LOOK_PAD, "Look", 0.55f, 0.46f, 0.26f, emptyList(), opacity = 0.30f, sensitivity = 1.25f),
                KeymapControl("fire", VirtualControlKind.MOUSE_BUTTON, "LMB", 0.88f, 0.72f, 0.11f, listOf("mouse_left")),
                KeymapControl("alt_fire", VirtualControlKind.MOUSE_BUTTON, "RMB", 0.77f, 0.56f, 0.085f, listOf("mouse_right")),
                KeymapControl("ult", VirtualControlKind.KEY_BUTTON, "Q", 0.66f, 0.82f, 0.08f, listOf("q")),
                KeymapControl("skill_e", VirtualControlKind.KEY_BUTTON, "E", 0.74f, 0.76f, 0.075f, listOf("e")),
                KeymapControl("skill_shift", VirtualControlKind.KEY_BUTTON, "Shift", 0.12f, 0.48f, 0.075f, listOf("shift")),
                KeymapControl("jump", VirtualControlKind.KEY_BUTTON, "Jump", 0.89f, 0.48f, 0.08f, listOf("space")),
                KeymapControl("reload", VirtualControlKind.KEY_BUTTON, "R", 0.65f, 0.62f, 0.07f, listOf("r")),
            ),
            Archetype.FPS_ARENA to listOf(
                KeymapControl("wasd", VirtualControlKind.JOYSTICK_WASD, "WASD", 0.14f, 0.74f, 0.22f, listOf("w", "a", "s", "d")),
                KeymapControl("look", VirtualControlKind.LOOK_PAD, "Look", 0.56f, 0.46f, 0.28f, emptyList(), opacity = 0.30f, sensitivity = 1.35f),
                KeymapControl("fire", VirtualControlKind.MACRO_BURST, "Burst", 0.88f, 0.72f, 0.11f, listOf("mouse_left"), burstIntervalMs = 55),
                KeymapControl("alt", VirtualControlKind.MOUSE_BUTTON, "Alt", 0.76f, 0.56f, 0.085f, listOf("mouse_right")),
                KeymapControl("jump", VirtualControlKind.KEY_BUTTON, "Jump", 0.89f, 0.48f, 0.085f, listOf("space")),
                KeymapControl("dash", VirtualControlKind.KEY_BUTTON, "Dash", 0.76f, 0.84f, 0.075f, listOf("shift")),
                KeymapControl("weapon_wheel", VirtualControlKind.SCROLL_WHEEL, "Weap", 0.92f, 0.26f, 0.10f, emptyList()),
            ),
            Archetype.MOBA_CLASSIC to listOf(
                KeymapControl("move_pad", VirtualControlKind.MOUSE_BUTTON, "Move", 0.14f, 0.74f, 0.14f, listOf("mouse_right")),
                KeymapControl("atk", VirtualControlKind.KEY_BUTTON, "A", 0.28f, 0.82f, 0.08f, listOf("a")),
                KeymapControl("q", VirtualControlKind.KEY_BUTTON, "Q", 0.64f, 0.80f, 0.085f, listOf("q")),
                KeymapControl("w", VirtualControlKind.KEY_BUTTON, "W", 0.74f, 0.74f, 0.085f, listOf("w")),
                KeymapControl("e", VirtualControlKind.KEY_BUTTON, "E", 0.82f, 0.64f, 0.085f, listOf("e")),
                KeymapControl("r", VirtualControlKind.KEY_BUTTON, "R", 0.88f, 0.50f, 0.095f, listOf("r")),
                KeymapControl("sum_d", VirtualControlKind.KEY_BUTTON, "D", 0.66f, 0.60f, 0.07f, listOf("d")),
                KeymapControl("sum_f", VirtualControlKind.KEY_BUTTON, "F", 0.74f, 0.52f, 0.07f, listOf("f")),
                KeymapControl("recall", VirtualControlKind.KEY_BUTTON, "B", 0.52f, 0.86f, 0.07f, listOf("b")),
                KeymapControl("center", VirtualControlKind.KEY_BUTTON, "Cam", 0.14f, 0.52f, 0.075f, listOf("space")),
            ),
            Archetype.MOBA_BRAWLER to listOf(
                KeymapControl("wasd", VirtualControlKind.JOYSTICK_WASD, "Move", 0.14f, 0.74f, 0.22f, listOf("w", "a", "s", "d")),
                KeymapControl("look", VirtualControlKind.LOOK_PAD, "Aim", 0.55f, 0.48f, 0.26f, emptyList(), opacity = 0.30f),
                KeymapControl("primary", VirtualControlKind.MOUSE_BUTTON, "Atk", 0.88f, 0.74f, 0.105f, listOf("mouse_left")),
                KeymapControl("secondary", VirtualControlKind.MOUSE_BUTTON, "Alt", 0.76f, 0.62f, 0.085f, listOf("mouse_right")),
                KeymapControl("s1", VirtualControlKind.KEY_BUTTON, "Q", 0.68f, 0.82f, 0.08f, listOf("q")),
                KeymapControl("s2", VirtualControlKind.KEY_BUTTON, "E", 0.78f, 0.84f, 0.08f, listOf("e")),
                KeymapControl("s3", VirtualControlKind.KEY_BUTTON, "R", 0.89f, 0.52f, 0.085f, listOf("r")),
                KeymapControl("dodge", VirtualControlKind.KEY_BUTTON, "Shift", 0.64f, 0.64f, 0.075f, listOf("shift")),
            ),
            Archetype.RPG_OPEN_WORLD to listOf(
                KeymapControl("wasd", VirtualControlKind.JOYSTICK_WASD, "Move", 0.14f, 0.74f, 0.22f, listOf("w", "a", "s", "d")),
                KeymapControl("cam", VirtualControlKind.LOOK_PAD, "Cam", 0.56f, 0.46f, 0.28f, emptyList(), opacity = 0.28f),
                KeymapControl("atk", VirtualControlKind.MOUSE_BUTTON, "Atk", 0.88f, 0.74f, 0.105f, listOf("mouse_left")),
                KeymapControl("sprint", VirtualControlKind.MOUSE_BUTTON, "Dash", 0.76f, 0.84f, 0.085f, listOf("mouse_right")),
                KeymapControl("skill", VirtualControlKind.KEY_BUTTON, "E", 0.75f, 0.62f, 0.085f, listOf("e")),
                KeymapControl("burst", VirtualControlKind.KEY_BUTTON, "Q", 0.86f, 0.52f, 0.09f, listOf("q")),
                KeymapControl("jump", VirtualControlKind.KEY_BUTTON, "Jump", 0.64f, 0.76f, 0.075f, listOf("space")),
                KeymapControl("interact", VirtualControlKind.KEY_BUTTON, "F", 0.62f, 0.58f, 0.075f, listOf("f")),
                KeymapControl("map", VirtualControlKind.KEY_BUTTON, "M", 0.90f, 0.14f, 0.07f, listOf("m")),
            ),
            Archetype.RPG_ARPG to listOf(
                KeymapControl("wasd", VirtualControlKind.JOYSTICK_WASD, "Move", 0.14f, 0.74f, 0.22f, listOf("w", "a", "s", "d")),
                KeymapControl("atk", VirtualControlKind.MOUSE_BUTTON, "LMB", 0.88f, 0.74f, 0.105f, listOf("mouse_left")),
                KeymapControl("rmb", VirtualControlKind.MOUSE_BUTTON, "RMB", 0.77f, 0.60f, 0.085f, listOf("mouse_right")),
                KeymapControl("s1", VirtualControlKind.KEY_BUTTON, "1", 0.56f, 0.84f, 0.075f, listOf("1")),
                KeymapControl("s2", VirtualControlKind.KEY_BUTTON, "2", 0.65f, 0.84f, 0.075f, listOf("2")),
                KeymapControl("s3", VirtualControlKind.KEY_BUTTON, "3", 0.74f, 0.84f, 0.075f, listOf("3")),
                KeymapControl("s4", VirtualControlKind.KEY_BUTTON, "4", 0.83f, 0.84f, 0.075f, listOf("4")),
                KeymapControl("dodge", VirtualControlKind.KEY_BUTTON, "Space", 0.89f, 0.52f, 0.085f, listOf("space")),
                KeymapControl("potion", VirtualControlKind.KEY_BUTTON, "Q", 0.28f, 0.84f, 0.075f, listOf("q")),
            ),
            Archetype.RPG_MMO to listOf(
                KeymapControl("wasd", VirtualControlKind.JOYSTICK_WASD, "Move", 0.14f, 0.74f, 0.22f, listOf("w", "a", "s", "d")),
                KeymapControl("cam", VirtualControlKind.LOOK_PAD, "Cam", 0.52f, 0.44f, 0.26f, emptyList(), opacity = 0.28f),
                KeymapControl("s1", VirtualControlKind.KEY_BUTTON, "1", 0.58f, 0.84f, 0.075f, listOf("1")),
                KeymapControl("s2", VirtualControlKind.KEY_BUTTON, "2", 0.67f, 0.84f, 0.075f, listOf("2")),
                KeymapControl("s3", VirtualControlKind.KEY_BUTTON, "3", 0.76f, 0.84f, 0.075f, listOf("3")),
                KeymapControl("s4", VirtualControlKind.KEY_BUTTON, "4", 0.85f, 0.84f, 0.075f, listOf("4")),
                KeymapControl("s5", VirtualControlKind.KEY_BUTTON, "5", 0.76f, 0.68f, 0.075f, listOf("5")),
                KeymapControl("jump", VirtualControlKind.KEY_BUTTON, "Jump", 0.88f, 0.66f, 0.08f, listOf("space")),
                KeymapControl("target", VirtualControlKind.KEY_BUTTON, "Tab", 0.88f, 0.48f, 0.075f, listOf("tab")),
                KeymapControl("zoom", VirtualControlKind.SCROLL_WHEEL, "Zoom", 0.92f, 0.24f, 0.095f, emptyList()),
            ),
            Archetype.RPG_TURN_BASED to listOf(
                KeymapControl("wasd", VirtualControlKind.JOYSTICK_WASD, "Move", 0.14f, 0.74f, 0.21f, listOf("w", "a", "s", "d")),
                KeymapControl("confirm", VirtualControlKind.KEY_BUTTON, "OK", 0.88f, 0.74f, 0.10f, listOf("enter")),
                KeymapControl("cancel", VirtualControlKind.KEY_BUTTON, "Esc", 0.76f, 0.82f, 0.085f, listOf("escape")),
                KeymapControl("skill", VirtualControlKind.KEY_BUTTON, "E", 0.76f, 0.60f, 0.085f, listOf("e")),
                KeymapControl("ult", VirtualControlKind.KEY_BUTTON, "Q", 0.88f, 0.54f, 0.085f, listOf("q")),
                KeymapControl("space", VirtualControlKind.KEY_BUTTON, "Space", 0.64f, 0.74f, 0.08f, listOf("space")),
                KeymapControl("tab", VirtualControlKind.KEY_BUTTON, "Tab", 0.90f, 0.16f, 0.075f, listOf("tab")),
            ),
            Archetype.ACTION_SOULS to listOf(
                KeymapControl("wasd", VirtualControlKind.JOYSTICK_WASD, "Move", 0.14f, 0.74f, 0.22f, listOf("w", "a", "s", "d")),
                KeymapControl("cam", VirtualControlKind.LOOK_PAD, "Cam", 0.54f, 0.46f, 0.26f, emptyList(), opacity = 0.28f),
                KeymapControl("light", VirtualControlKind.MOUSE_BUTTON, "Light", 0.88f, 0.72f, 0.105f, listOf("mouse_left")),
                KeymapControl("heavy", VirtualControlKind.COMBO_BUTTON, "Heavy", 0.88f, 0.52f, 0.085f, listOf("shift", "mouse_left")),
                KeymapControl("block", VirtualControlKind.MOUSE_BUTTON, "Guard", 0.75f, 0.58f, 0.085f, listOf("mouse_right")),
                KeymapControl("roll", VirtualControlKind.KEY_BUTTON, "Roll", 0.76f, 0.82f, 0.09f, listOf("space")),
                KeymapControl("heal", VirtualControlKind.KEY_BUTTON, "Heal", 0.64f, 0.78f, 0.075f, listOf("r")),
                KeymapControl("lock", VirtualControlKind.KEY_BUTTON, "Lock", 0.64f, 0.58f, 0.075f, listOf("q")),
            ),
            Archetype.ACTION_HACK_SLASH to listOf(
                KeymapControl("wasd", VirtualControlKind.JOYSTICK_WASD, "Move", 0.14f, 0.74f, 0.22f, listOf("w", "a", "s", "d")),
                KeymapControl("cam", VirtualControlKind.LOOK_PAD, "Cam", 0.54f, 0.46f, 0.26f, emptyList(), opacity = 0.28f),
                KeymapControl("atk", VirtualControlKind.MACRO_BURST, "Combo", 0.88f, 0.74f, 0.11f, listOf("mouse_left"), burstIntervalMs = 95),
                KeymapControl("special", VirtualControlKind.MOUSE_BUTTON, "Spec", 0.76f, 0.58f, 0.085f, listOf("mouse_right")),
                KeymapControl("dodge", VirtualControlKind.KEY_BUTTON, "Dodge", 0.76f, 0.84f, 0.085f, listOf("shift")),
                KeymapControl("jump", VirtualControlKind.KEY_BUTTON, "Jump", 0.88f, 0.52f, 0.085f, listOf("space")),
                KeymapControl("interact", VirtualControlKind.KEY_BUTTON, "E", 0.64f, 0.72f, 0.075f, listOf("e")),
            ),
            Archetype.ACTION_PLATFORMER to listOf(
                KeymapControl("dpad", VirtualControlKind.JOYSTICK_WASD, "Move", 0.15f, 0.74f, 0.22f, listOf("up", "left", "down", "right")),
                KeymapControl("jump", VirtualControlKind.KEY_BUTTON, "Jump", 0.88f, 0.74f, 0.105f, listOf("z")),
                KeymapControl("atk", VirtualControlKind.KEY_BUTTON, "Atk", 0.76f, 0.80f, 0.095f, listOf("x")),
                KeymapControl("dash", VirtualControlKind.KEY_BUTTON, "Dash", 0.88f, 0.54f, 0.085f, listOf("c")),
                KeymapControl("grab", VirtualControlKind.KEY_BUTTON, "Grab", 0.74f, 0.58f, 0.08f, listOf("shift")),
                KeymapControl("pause", VirtualControlKind.KEY_BUTTON, "Esc", 0.50f, 0.10f, 0.07f, listOf("escape")),
            ),
            Archetype.ACTION_STEALTH to listOf(
                KeymapControl("wasd", VirtualControlKind.JOYSTICK_WASD, "Move", 0.14f, 0.74f, 0.22f, listOf("w", "a", "s", "d")),
                KeymapControl("cam", VirtualControlKind.LOOK_PAD, "Look", 0.55f, 0.46f, 0.26f, emptyList(), opacity = 0.28f),
                KeymapControl("action", VirtualControlKind.MOUSE_BUTTON, "Act", 0.88f, 0.74f, 0.10f, listOf("mouse_left")),
                KeymapControl("aim", VirtualControlKind.MOUSE_BUTTON, "Aim", 0.76f, 0.58f, 0.085f, listOf("mouse_right")),
                KeymapControl("crouch", VirtualControlKind.KEY_BUTTON, "Sneak", 0.76f, 0.84f, 0.085f, listOf("c"), toggle = true),
                KeymapControl("interact", VirtualControlKind.KEY_BUTTON, "E", 0.88f, 0.52f, 0.085f, listOf("e")),
                KeymapControl("vision", VirtualControlKind.KEY_BUTTON, "Scan", 0.64f, 0.74f, 0.075f, listOf("q")),
            ),
            Archetype.ACTION_FIGHTING to listOf(
                KeymapControl("stick", VirtualControlKind.JOYSTICK_WASD, "Stick", 0.15f, 0.74f, 0.23f, listOf("w", "a", "s", "d")),
                KeymapControl("lp", VirtualControlKind.KEY_BUTTON, "LP", 0.66f, 0.60f, 0.085f, listOf("u")),
                KeymapControl("hp", VirtualControlKind.KEY_BUTTON, "HP", 0.78f, 0.56f, 0.085f, listOf("i")),
                KeymapControl("lk", VirtualControlKind.KEY_BUTTON, "LK", 0.66f, 0.80f, 0.085f, listOf("j")),
                KeymapControl("hk", VirtualControlKind.KEY_BUTTON, "HK", 0.78f, 0.78f, 0.085f, listOf("k")),
                KeymapControl("sp", VirtualControlKind.KEY_BUTTON, "SP", 0.90f, 0.66f, 0.09f, listOf("l")),
                KeymapControl("throw", VirtualControlKind.COMBO_BUTTON, "Grab", 0.90f, 0.46f, 0.08f, listOf("u", "j")),
            ),
            Archetype.RACING_ARCADE to listOf(
                KeymapControl("steer", VirtualControlKind.JOYSTICK_WASD, "Steer", 0.16f, 0.74f, 0.24f, listOf("w", "a", "s", "d")),
                KeymapControl("gas", VirtualControlKind.KEY_BUTTON, "Gas", 0.88f, 0.72f, 0.115f, listOf("w")),
                KeymapControl("brake", VirtualControlKind.KEY_BUTTON, "Brake", 0.74f, 0.80f, 0.10f, listOf("s")),
                KeymapControl("drift", VirtualControlKind.KEY_BUTTON, "Drift", 0.75f, 0.58f, 0.09f, listOf("space")),
                KeymapControl("nitro", VirtualControlKind.KEY_BUTTON, "Nitro", 0.89f, 0.48f, 0.095f, listOf("shift")),
                KeymapControl("cam", VirtualControlKind.KEY_BUTTON, "Cam", 0.90f, 0.16f, 0.07f, listOf("c")),
                KeymapControl("reset", VirtualControlKind.KEY_BUTTON, "Reset", 0.12f, 0.16f, 0.07f, listOf("r")),
            ),
            Archetype.RACING_SIM to listOf(
                KeymapControl("steer", VirtualControlKind.JOYSTICK_WASD, "Wheel", 0.16f, 0.74f, 0.24f, listOf("up", "left", "down", "right")),
                KeymapControl("throttle", VirtualControlKind.KEY_BUTTON, "Gas", 0.88f, 0.72f, 0.115f, listOf("up")),
                KeymapControl("brake", VirtualControlKind.KEY_BUTTON, "Brake", 0.74f, 0.80f, 0.10f, listOf("down")),
                KeymapControl("shift_up", VirtualControlKind.KEY_BUTTON, "Gear+", 0.89f, 0.48f, 0.085f, listOf("e")),
                KeymapControl("shift_dn", VirtualControlKind.KEY_BUTTON, "Gear-", 0.76f, 0.54f, 0.085f, listOf("q")),
                KeymapControl("handbrake", VirtualControlKind.KEY_BUTTON, "HB", 0.63f, 0.78f, 0.08f, listOf("space")),
                KeymapControl("pit", VirtualControlKind.KEY_BUTTON, "Cam", 0.90f, 0.16f, 0.07f, listOf("c")),
            ),
            Archetype.SIM_FLIGHT to listOf(
                KeymapControl("yoke", VirtualControlKind.JOYSTICK_WASD, "Pitch", 0.16f, 0.72f, 0.24f, listOf("w", "a", "s", "d")),
                KeymapControl("look", VirtualControlKind.LOOK_PAD, "Cockpit", 0.54f, 0.44f, 0.26f, emptyList(), opacity = 0.26f),
                KeymapControl("thr_up", VirtualControlKind.KEY_BUTTON, "Thr+", 0.88f, 0.52f, 0.09f, listOf("f3")),
                KeymapControl("thr_dn", VirtualControlKind.KEY_BUTTON, "Thr-", 0.88f, 0.74f, 0.09f, listOf("f2")),
                KeymapControl("gear", VirtualControlKind.KEY_BUTTON, "Gear", 0.74f, 0.78f, 0.08f, listOf("g")),
                KeymapControl("flaps", VirtualControlKind.KEY_BUTTON, "Flap", 0.74f, 0.58f, 0.08f, listOf("f7")),
                KeymapControl("brake", VirtualControlKind.KEY_BUTTON, "Brake", 0.62f, 0.80f, 0.075f, listOf("b")),
            ),
            Archetype.SIM_STRATEGY to listOf(
                KeymapControl("pan", VirtualControlKind.JOYSTICK_WASD, "Pan", 0.14f, 0.74f, 0.21f, listOf("w", "a", "s", "d")),
                KeymapControl("select", VirtualControlKind.MOUSE_BUTTON, "Sel", 0.88f, 0.74f, 0.10f, listOf("mouse_left")),
                KeymapControl("order", VirtualControlKind.MOUSE_BUTTON, "Order", 0.76f, 0.74f, 0.095f, listOf("mouse_right")),
                KeymapControl("pause", VirtualControlKind.KEY_BUTTON, "Pause", 0.50f, 0.12f, 0.08f, listOf("space")),
                KeymapControl("zoom", VirtualControlKind.SCROLL_WHEEL, "Zoom", 0.92f, 0.36f, 0.11f, emptyList()),
                KeymapControl("grp1", VirtualControlKind.COMBO_BUTTON, "Grp1", 0.64f, 0.84f, 0.075f, listOf("ctrl", "1")),
                KeymapControl("esc", VirtualControlKind.KEY_BUTTON, "Esc", 0.10f, 0.12f, 0.07f, listOf("escape")),
            ),
            Archetype.SIM_SANDBOX to listOf(
                KeymapControl("wasd", VirtualControlKind.JOYSTICK_WASD, "Move", 0.14f, 0.74f, 0.22f, listOf("w", "a", "s", "d")),
                KeymapControl("look", VirtualControlKind.LOOK_PAD, "Look", 0.55f, 0.46f, 0.26f, emptyList(), opacity = 0.28f),
                KeymapControl("use", VirtualControlKind.MOUSE_BUTTON, "Act", 0.88f, 0.74f, 0.10f, listOf("mouse_left")),
                KeymapControl("alt", VirtualControlKind.MOUSE_BUTTON, "Build", 0.76f, 0.62f, 0.085f, listOf("mouse_right")),
                KeymapControl("jump", VirtualControlKind.KEY_BUTTON, "Jump", 0.88f, 0.52f, 0.085f, listOf("space")),
                KeymapControl("inv", VirtualControlKind.KEY_BUTTON, "Bag", 0.76f, 0.84f, 0.08f, listOf("e")),
                KeymapControl("rotate", VirtualControlKind.KEY_BUTTON, "Rot", 0.64f, 0.76f, 0.075f, listOf("r")),
            ),
            Archetype.PROD_IDE to listOf(
                KeymapControl("palette", VirtualControlKind.COMBO_BUTTON, "Cmd+P", 0.14f, 0.84f, 0.09f, listOf("ctrl", "shift", "p")),
                KeymapControl("save", VirtualControlKind.COMBO_BUTTON, "Save", 0.26f, 0.84f, 0.085f, listOf("ctrl", "s")),
                KeymapControl("find", VirtualControlKind.COMBO_BUTTON, "Find", 0.38f, 0.84f, 0.085f, listOf("ctrl", "f")),
                KeymapControl("undo", VirtualControlKind.COMBO_BUTTON, "Undo", 0.62f, 0.84f, 0.085f, listOf("ctrl", "z")),
                KeymapControl("copy", VirtualControlKind.COMBO_BUTTON, "Copy", 0.74f, 0.84f, 0.085f, listOf("ctrl", "c")),
                KeymapControl("paste", VirtualControlKind.COMBO_BUTTON, "Paste", 0.86f, 0.84f, 0.085f, listOf("ctrl", "v")),
                KeymapControl("scroll", VirtualControlKind.SCROLL_WHEEL, "Scroll", 0.92f, 0.46f, 0.12f, emptyList()),
            ),
            Archetype.PROD_CREATIVE to listOf(
                KeymapControl("undo", VirtualControlKind.COMBO_BUTTON, "Undo", 0.12f, 0.84f, 0.09f, listOf("ctrl", "z")),
                KeymapControl("redo", VirtualControlKind.COMBO_BUTTON, "Redo", 0.24f, 0.84f, 0.09f, listOf("ctrl", "shift", "z")),
                KeymapControl("pan", VirtualControlKind.KEY_BUTTON, "Hand", 0.12f, 0.64f, 0.09f, listOf("space")),
                KeymapControl("save", VirtualControlKind.COMBO_BUTTON, "Save", 0.88f, 0.84f, 0.085f, listOf("ctrl", "s")),
                KeymapControl("brush_inc", VirtualControlKind.KEY_BUTTON, "]", 0.88f, 0.64f, 0.08f, listOf("]")),
                KeymapControl("brush_dec", VirtualControlKind.KEY_BUTTON, "[", 0.78f, 0.64f, 0.08f, listOf("[")),
                KeymapControl("zoom", VirtualControlKind.SCROLL_WHEEL, "Zoom", 0.92f, 0.36f, 0.12f, emptyList()),
            ),
            Archetype.PROD_OFFICE to listOf(
                KeymapControl("copy", VirtualControlKind.COMBO_BUTTON, "Copy", 0.14f, 0.84f, 0.085f, listOf("ctrl", "c")),
                KeymapControl("paste", VirtualControlKind.COMBO_BUTTON, "Paste", 0.26f, 0.84f, 0.085f, listOf("ctrl", "v")),
                KeymapControl("undo", VirtualControlKind.COMBO_BUTTON, "Undo", 0.38f, 0.84f, 0.085f, listOf("ctrl", "z")),
                KeymapControl("save", VirtualControlKind.COMBO_BUTTON, "Save", 0.66f, 0.84f, 0.085f, listOf("ctrl", "s")),
                KeymapControl("find", VirtualControlKind.COMBO_BUTTON, "Find", 0.78f, 0.84f, 0.085f, listOf("ctrl", "f")),
                KeymapControl("enter", VirtualControlKind.KEY_BUTTON, "Enter", 0.89f, 0.84f, 0.085f, listOf("enter")),
                KeymapControl("scroll", VirtualControlKind.SCROLL_WHEEL, "Scroll", 0.92f, 0.46f, 0.12f, emptyList()),
            ),
            Archetype.PROD_TERMINAL to listOf(
                KeymapControl("nav", VirtualControlKind.JOYSTICK_WASD, "Arrows", 0.14f, 0.74f, 0.20f, listOf("up", "left", "down", "right")),
                KeymapControl("ctrl_c", VirtualControlKind.COMBO_BUTTON, "^C", 0.64f, 0.84f, 0.085f, listOf("ctrl", "c")),
                KeymapControl("ctrl_d", VirtualControlKind.COMBO_BUTTON, "^D", 0.76f, 0.84f, 0.085f, listOf("ctrl", "d")),
                KeymapControl("tab", VirtualControlKind.KEY_BUTTON, "Tab", 0.88f, 0.84f, 0.085f, listOf("tab")),
                KeymapControl("esc", VirtualControlKind.KEY_BUTTON, "Esc", 0.76f, 0.66f, 0.08f, listOf("escape")),
                KeymapControl("enter", VirtualControlKind.KEY_BUTTON, "Enter", 0.88f, 0.66f, 0.09f, listOf("enter")),
                KeymapControl("scroll", VirtualControlKind.SCROLL_WHEEL, "Scroll", 0.92f, 0.36f, 0.11f, emptyList()),
            ),
            Archetype.MEDIA_PLAYER to listOf(
                KeymapControl("play", VirtualControlKind.KEY_BUTTON, "Play", 0.50f, 0.82f, 0.105f, listOf("space")),
                KeymapControl("back", VirtualControlKind.KEY_BUTTON, "-5s", 0.34f, 0.82f, 0.09f, listOf("left")),
                KeymapControl("fwd", VirtualControlKind.KEY_BUTTON, "+5s", 0.66f, 0.82f, 0.09f, listOf("right")),
                KeymapControl("vol_up", VirtualControlKind.KEY_BUTTON, "Vol+", 0.88f, 0.64f, 0.085f, listOf("up")),
                KeymapControl("vol_dn", VirtualControlKind.KEY_BUTTON, "Vol-", 0.88f, 0.82f, 0.085f, listOf("down")),
                KeymapControl("mute", VirtualControlKind.KEY_BUTTON, "Mute", 0.14f, 0.82f, 0.085f, listOf("m")),
                KeymapControl("full", VirtualControlKind.KEY_BUTTON, "Full", 0.14f, 0.64f, 0.085f, listOf("f")),
            ),
            Archetype.MEDIA_EDITOR to listOf(
                KeymapControl("play", VirtualControlKind.KEY_BUTTON, "Play", 0.50f, 0.84f, 0.10f, listOf("space")),
                KeymapControl("rev", VirtualControlKind.KEY_BUTTON, "J", 0.36f, 0.84f, 0.085f, listOf("j")),
                KeymapControl("stop", VirtualControlKind.KEY_BUTTON, "K", 0.64f, 0.84f, 0.085f, listOf("k")),
                KeymapControl("ffwd", VirtualControlKind.KEY_BUTTON, "L", 0.76f, 0.84f, 0.085f, listOf("l")),
                KeymapControl("undo", VirtualControlKind.COMBO_BUTTON, "Undo", 0.14f, 0.84f, 0.085f, listOf("ctrl", "z")),
                KeymapControl("cut", VirtualControlKind.COMBO_BUTTON, "Cut", 0.25f, 0.84f, 0.085f, listOf("ctrl", "b")),
                KeymapControl("save", VirtualControlKind.COMBO_BUTTON, "Save", 0.88f, 0.84f, 0.085f, listOf("ctrl", "s")),
                KeymapControl("scrub", VirtualControlKind.SCROLL_WHEEL, "Time", 0.92f, 0.46f, 0.12f, emptyList()),
            ),
        )
    }

    private fun fps(slug: String, name: String, arch: Archetype, vararg procs: String, tags: List<String> = emptyList()) =
        CatalogEntry(slug, name, KeymapCategory.FPS, arch, procs.toList(), tags)

    private fun moba(slug: String, name: String, arch: Archetype, vararg procs: String, tags: List<String> = emptyList()) =
        CatalogEntry(slug, name, KeymapCategory.MOBA, arch, procs.toList(), tags)

    private fun rpg(slug: String, name: String, arch: Archetype, vararg procs: String, tags: List<String> = emptyList()) =
        CatalogEntry(slug, name, KeymapCategory.RPG, arch, procs.toList(), tags)

    private fun action(slug: String, name: String, arch: Archetype, vararg procs: String, tags: List<String> = emptyList()) =
        CatalogEntry(slug, name, KeymapCategory.ACTION, arch, procs.toList(), tags)

    private fun racing(slug: String, name: String, arch: Archetype, vararg procs: String, tags: List<String> = emptyList()) =
        CatalogEntry(slug, name, KeymapCategory.RACING, arch, procs.toList(), tags)

    private fun sim(slug: String, name: String, arch: Archetype, vararg procs: String, tags: List<String> = emptyList()) =
        CatalogEntry(slug, name, KeymapCategory.SIMULATOR, arch, procs.toList(), tags)

    private fun prod(slug: String, name: String, arch: Archetype, vararg procs: String, tags: List<String> = emptyList()) =
        CatalogEntry(slug, name, KeymapCategory.PRODUCTIVITY, arch, procs.toList(), tags)

    private fun media(slug: String, name: String, arch: Archetype, vararg procs: String, tags: List<String> = emptyList()) =
        CatalogEntry(slug, name, KeymapCategory.MEDIA, arch, procs.toList(), tags)

    private val entries: List<CatalogEntry> by lazy {
        listOf(
            // === FPS (94) ===
            fps("cs2", "Counter-Strike 2", Archetype.FPS_TACTICAL, "cs2.exe", "csgo.exe", tags = listOf("cs", "csgo", "valve", "shooter")),
            fps("valorant", "VALORANT", Archetype.FPS_TACTICAL, "valorant-win64-shipping.exe", "valorant.exe", tags = listOf("riot", "val", "tactical")),
            fps("apex-legends", "Apex Legends", Archetype.FPS_BATTLE_ROYALE, "r5apex.exe", "r5apex_dx12.exe", tags = listOf("apex", "br", "respawn")),
            fps("overwatch-2", "Overwatch 2", Archetype.FPS_HERO, "overwatch.exe", tags = listOf("ow2", "blizzard", "hero")),
            fps("cod-warzone", "Call of Duty: Warzone", Archetype.FPS_BATTLE_ROYALE, "cod.exe", "modernwarfare.exe", tags = listOf("cod", "wz", "br")),
            fps("cod-mw3", "Call of Duty: Modern Warfare III", Archetype.FPS_TACTICAL, "cod-mw3.exe", tags = listOf("cod", "mw3")),
            fps("cod-bo6", "Call of Duty: Black Ops 6", Archetype.FPS_TACTICAL, "cod-bo6.exe", tags = listOf("cod", "bo6")),
            fps("pubg", "PUBG: Battlegrounds", Archetype.FPS_BATTLE_ROYALE, "tslgame.exe", tags = listOf("pubg", "br")),
            fps("rainbow-six-siege", "Rainbow Six Siege", Archetype.FPS_TACTICAL, "rainbowsix.exe", "rainbowsix_vulkan.exe", tags = listOf("r6", "r6s", "ubisoft")),
            fps("fortnite", "Fortnite", Archetype.FPS_BATTLE_ROYALE, "fortniteclient-win64-shipping.exe", tags = listOf("fn", "epic", "br")),
            fps("battlefield-2042", "Battlefield 2042", Archetype.FPS_TACTICAL, "bf2042.exe", tags = listOf("bf", "ea")),
            fps("battlefield-v", "Battlefield V", Archetype.FPS_TACTICAL, "bfv.exe", tags = listOf("bf5", "ea")),
            fps("battlefield-1", "Battlefield 1", Archetype.FPS_TACTICAL, "bf1.exe", tags = listOf("bf1", "ea")),
            fps("escape-from-tarkov", "Escape from Tarkov", Archetype.FPS_TACTICAL, "escapefromtarkov.exe", tags = listOf("eft", "tarkov", "extraction")),
            fps("destiny-2", "Destiny 2", Archetype.FPS_HERO, "destiny2.exe", tags = listOf("d2", "bungie")),
            fps("doom-eternal", "DOOM Eternal", Archetype.FPS_ARENA, "doometernalx64vk.exe", tags = listOf("doom", "id")),
            fps("doom-2016", "DOOM (2016)", Archetype.FPS_ARENA, "doomx64.exe", "doomx64vk.exe", tags = listOf("doom")),
            fps("halo-infinite", "Halo Infinite", Archetype.FPS_ARENA, "haloinfinite.exe", tags = listOf("halo")),
            fps("halo-mcc", "Halo: The Master Chief Collection", Archetype.FPS_ARENA, "mcc-win64-shipping.exe", tags = listOf("halo", "mcc")),
            fps("titanfall-2", "Titanfall 2", Archetype.FPS_ARENA, "titanfall2.exe", tags = listOf("tf2", "respawn")),
            fps("team-fortress-2", "Team Fortress 2", Archetype.FPS_HERO, "tf_win64.exe", "hl2.exe", tags = listOf("tf2", "valve")),
            fps("left-4-dead-2", "Left 4 Dead 2", Archetype.FPS_TACTICAL, "left4dead2.exe", tags = listOf("l4d2", "coop")),
            fps("half-life-2", "Half-Life 2", Archetype.FPS_TACTICAL, "hl2_win64.exe", tags = listOf("hl2", "valve")),
            fps("portal-2", "Portal 2", Archetype.FPS_TACTICAL, "portal2.exe", tags = listOf("portal", "puzzle")),
            fps("rust", "Rust", Archetype.FPS_BATTLE_ROYALE, "rustclient.exe", tags = listOf("survival")),
            fps("dayz", "DayZ", Archetype.FPS_BATTLE_ROYALE, "dayz_x64.exe", tags = listOf("survival")),
            fps("hunt-showdown", "Hunt: Showdown 1896", Archetype.FPS_TACTICAL, "huntgame.exe", tags = listOf("hunt", "extraction")),
            fps("ready-or-not", "Ready or Not", Archetype.FPS_TACTICAL, "readyornot-win64-shipping.exe", tags = listOf("ron", "swat")),
            fps("squad", "Squad", Archetype.FPS_TACTICAL, "squadgame.exe", tags = listOf("milsim")),
            fps("hell-let-loose", "Hell Let Loose", Archetype.FPS_TACTICAL, "hll-win64-shipping.exe", tags = listOf("hll", "ww2")),
            fps("insurgency-sandstorm", "Insurgency: Sandstorm", Archetype.FPS_TACTICAL, "insurgencyclient-win64-shipping.exe", tags = listOf("insurgency")),
            fps("the-finals", "THE FINALS", Archetype.FPS_HERO, "discovery.exe", tags = listOf("finals", "embark")),
            fps("delta-force", "Delta Force", Archetype.FPS_TACTICAL, "deltaforceclient-win64-shipping.exe", tags = listOf("df", "extraction")),
            fps("arena-breakout", "Arena Breakout: Infinite", Archetype.FPS_TACTICAL, "uagame.exe", tags = listOf("abi", "extraction")),
            fps("gray-zone-warfare", "Gray Zone Warfare", Archetype.FPS_TACTICAL, "gzwclientsteam-win64-shipping.exe", tags = listOf("gzw")),
            fps("payday-2", "PAYDAY 2", Archetype.FPS_TACTICAL, "payday2_win32_release.exe", tags = listOf("heist")),
            fps("payday-3", "PAYDAY 3", Archetype.FPS_TACTICAL, "payday3client-win64-shipping.exe", tags = listOf("heist")),
            fps("deep-rock-galactic", "Deep Rock Galactic", Archetype.FPS_TACTICAL, "fsd-win64-shipping.exe", tags = listOf("drg", "coop")),
            fps("borderlands-2", "Borderlands 2", Archetype.FPS_ARENA, "borderlands2.exe", tags = listOf("bl2", "looter")),
            fps("borderlands-3", "Borderlands 3", Archetype.FPS_ARENA, "borderlands3.exe", tags = listOf("bl3", "looter")),
            fps("tiny-tinas-wonderlands", "Tiny Tina's Wonderlands", Archetype.FPS_ARENA, "wonderlands.exe", tags = listOf("looter")),
            fps("bioshock-infinite", "BioShock Infinite", Archetype.FPS_TACTICAL, "bioshockinfinite.exe", tags = listOf("bioshock")),
            fps("metro-exodus", "Metro Exodus", Archetype.FPS_TACTICAL, "metroexodus.exe", tags = listOf("metro")),
            fps("stalker-2", "S.T.A.L.K.E.R. 2", Archetype.FPS_TACTICAL, "stalker2-win64-shipping.exe", tags = listOf("stalker")),
            fps("stalker-anomaly", "S.T.A.L.K.E.R. Anomaly", Archetype.FPS_TACTICAL, "anomalydx11.exe", tags = listOf("stalker")),
            fps("crysis-remastered", "Crysis Remastered", Archetype.FPS_TACTICAL, "crysisremastered.exe", tags = listOf("crysis")),
            fps("far-cry-5", "Far Cry 5", Archetype.FPS_BATTLE_ROYALE, "farcry5.exe", tags = listOf("fc5", "ubisoft")),
            fps("far-cry-6", "Far Cry 6", Archetype.FPS_BATTLE_ROYALE, "farcry6.exe", tags = listOf("fc6", "ubisoft")),
            fps("wolfenstein-2", "Wolfenstein II", Archetype.FPS_ARENA, "newcolossus_x64vk.exe", tags = listOf("wolfenstein")),
            fps("quake-champions", "Quake Champions", Archetype.FPS_ARENA, "quakechampions.exe", tags = listOf("quake")),
            fps("quake-live", "Quake Live", Archetype.FPS_ARENA, "quakelive_steam.exe", tags = listOf("quake")),
            fps("paladins", "Paladins", Archetype.FPS_HERO, "paladins.exe", tags = listOf("hero")),
            fps("splitgate", "Splitgate", Archetype.FPS_ARENA, "portalwars-win64-shipping.exe", tags = listOf("portal", "arena")),
            fps("enlisted", "Enlisted", Archetype.FPS_TACTICAL, "enlisted.exe", tags = listOf("ww2")),
            fps("warface", "Warface: Clutch", Archetype.FPS_TACTICAL, "game.exe", tags = listOf("wf")),
            fps("crossfire", "CrossFire", Archetype.FPS_TACTICAL, "crossfire.exe", tags = listOf("cf")),
            fps("point-blank", "Point Blank", Archetype.FPS_TACTICAL, "pointblank.exe", tags = listOf("pb")),
            fps("krunker", "Krunker.io", Archetype.FPS_ARENA, "krunker.exe", tags = listOf("browser", "arena")),
            fps("battlebit-remastered", "BattleBit Remastered", Archetype.FPS_TACTICAL, "battlebit.exe", tags = listOf("battlebit")),
            fps("arma-3", "Arma 3", Archetype.FPS_TACTICAL, "arma3_x64.exe", tags = listOf("arma", "milsim")),
            fps("arma-reforger", "Arma Reforger", Archetype.FPS_TACTICAL, "armareforgersteam.exe", tags = listOf("arma")),
            fps("rising-storm-2", "Rising Storm 2: Vietnam", Archetype.FPS_TACTICAL, "vngame.exe", tags = listOf("rs2")),
            fps("squad-44", "Squad 44", Archetype.FPS_TACTICAL, "postscriptum.exe", tags = listOf("ww2")),
            fps("isonzo", "Isonzo", Archetype.FPS_TACTICAL, "isonzo.exe", tags = listOf("ww1")),
            fps("verdun", "Verdun", Archetype.FPS_TACTICAL, "verdun.exe", tags = listOf("ww1")),
            fps("planetside-2", "PlanetSide 2", Archetype.FPS_TACTICAL, "planetside2_x64.exe", tags = listOf("ps2", "mmofps")),
            fps("sw-battlefront-2", "Star Wars Battlefront II", Archetype.FPS_HERO, "starwarsbattlefrontii.exe", tags = listOf("swbf2")),
            fps("ghost-recon-wildlands", "Ghost Recon Wildlands", Archetype.FPS_TACTICAL, "grw.exe", tags = listOf("grw")),
            fps("ghost-recon-breakpoint", "Ghost Recon Breakpoint", Archetype.FPS_TACTICAL, "grb.exe", tags = listOf("grb")),
            fps("division-2", "Tom Clancy's The Division 2", Archetype.FPS_TACTICAL, "thedivision2.exe", tags = listOf("div2")),
            fps("sniper-elite-5", "Sniper Elite 5", Archetype.FPS_TACTICAL, "sniper5_dx12.exe", tags = listOf("sniper")),
            fps("sniper-ghost-warrior", "Sniper Ghost Warrior Contracts 2", Archetype.FPS_TACTICAL, "sgwcontracts2.exe", tags = listOf("sniper")),
            fps("ultrakill", "ULTRAKILL", Archetype.FPS_ARENA, "ultrakill.exe", tags = listOf("boomer", "arena")),
            fps("dusk", "DUSK", Archetype.FPS_ARENA, "dusk.exe", tags = listOf("boomer")),
            fps("amid-evil", "AMID EVIL", Archetype.FPS_ARENA, "amidevil-win64-shipping.exe", tags = listOf("boomer")),
            fps("prodeus", "Prodeus", Archetype.FPS_ARENA, "prodeus.exe", tags = listOf("boomer")),
            fps("turbo-overkill", "Turbo Overkill", Archetype.FPS_ARENA, "turbooverkill.exe", tags = listOf("boomer")),
            fps("serious-sam-4", "Serious Sam 4", Archetype.FPS_ARENA, "sam4.exe", tags = listOf("sam")),
            fps("killing-floor-2", "Killing Floor 2", Archetype.FPS_ARENA, "kfgame.exe", tags = listOf("kf2", "coop")),
            fps("back-4-blood", "Back 4 Blood", Archetype.FPS_TACTICAL, "back4blood.exe", tags = listOf("b4b", "coop")),
            fps("world-war-z", "World War Z", Archetype.FPS_TACTICAL, "wwzretailegs.exe", tags = listOf("wwz", "coop")),
            fps("gtfo", "GTFO", Archetype.FPS_TACTICAL, "gtfo.exe", tags = listOf("coop", "horror")),
            fps("helldivers-2", "Helldivers 2", Archetype.FPS_TACTICAL, "helldivers2.exe", tags = listOf("hd2", "coop")),
            fps("risk-of-rain-2", "Risk of Rain 2", Archetype.FPS_ARENA, "risk of rain 2.exe", tags = listOf("ror2", "roguelike")),
            fps("roboquest", "Roboquest", Archetype.FPS_ARENA, "roboquest-win64-shipping.exe", tags = listOf("roguelike")),
            fps("gunfire-reborn", "Gunfire Reborn", Archetype.FPS_ARENA, "gunfire reborn.exe", tags = listOf("roguelike")),
            fps("neon-white", "Neon White", Archetype.FPS_ARENA, "neon white.exe", tags = listOf("speedrun")),
            fps("ghostrunner", "Ghostrunner", Archetype.FPS_ARENA, "ghostrunner-win64-shipping.exe", tags = listOf("parkour")),
            fps("superhot", "SUPERHOT", Archetype.FPS_TACTICAL, "superhot.exe", tags = listOf("time")),
            fps("marauders", "Marauders", Archetype.FPS_TACTICAL, "raidgame-win64-shipping.exe", tags = listOf("extraction")),
            fps("bodycam", "Bodycam", Archetype.FPS_TACTICAL, "bodycam-win64-shipping.exe", tags = listOf("realistic")),
            fps("six-days-fallujah", "Six Days in Fallujah", Archetype.FPS_TACTICAL, "sixdays-win64-shipping.exe", tags = listOf("tactical")),
            fps("chivalry-2", "Chivalry 2", Archetype.FPS_ARENA, "chivalry2-win64-shipping.exe", tags = listOf("medieval")),
            fps("strinova", "Strinova", Archetype.FPS_HERO, "strinova.exe", tags = listOf("calabiyau", "anime")),

            // === MOBA (45) ===
            moba("league-of-legends", "League of Legends", Archetype.MOBA_CLASSIC, "league of legends.exe", "leagueclient.exe", tags = listOf("lol", "riot")),
            moba("dota-2", "Dota 2", Archetype.MOBA_CLASSIC, "dota2.exe", tags = listOf("dota", "valve")),
            moba("smite", "SMITE", Archetype.MOBA_BRAWLER, "smite.exe", tags = listOf("hirez")),
            moba("smite-2", "SMITE 2", Archetype.MOBA_BRAWLER, "hemingway-win64-shipping.exe", tags = listOf("smite2")),
            moba("heroes-of-the-storm", "Heroes of the Storm", Archetype.MOBA_CLASSIC, "heroesofthestorm_x64.exe", tags = listOf("hots", "blizzard")),
            moba("deadlock", "Deadlock", Archetype.MOBA_BRAWLER, "project8.exe", "deadlock.exe", tags = listOf("valve", "shooter")),
            moba("marvel-rivals", "Marvel Rivals", Archetype.MOBA_BRAWLER, "marvel-win64-shipping.exe", tags = listOf("marvel", "hero")),
            moba("eternal-return", "Eternal Return", Archetype.MOBA_CLASSIC, "eternalreturn.exe", tags = listOf("erbs")),
            moba("naraka-bladepoint", "NARAKA: BLADEPOINT", Archetype.MOBA_BRAWLER, "narakabladepoint.exe", tags = listOf("naraka", "yongjie")),
            moba("predecessor", "Predecessor", Archetype.MOBA_BRAWLER, "predecessorclient-win64-shipping.exe", tags = listOf("paragon")),
            moba("supervive", "SUPERVIVE", Archetype.MOBA_BRAWLER, "supervive.exe", tags = listOf("theorycraft")),
            moba("omega-strikers", "Omega Strikers", Archetype.MOBA_BRAWLER, "omegastrikers-win64-shipping.exe", tags = listOf("striker")),
            moba("battlerite", "Battlerite", Archetype.MOBA_BRAWLER, "battlerite.exe", tags = listOf("arena")),
            moba("gigantic-rampage", "Gigantic: Rampage Edition", Archetype.MOBA_BRAWLER, "rxgamesteam-win64-shipping.exe", tags = listOf("gigantic")),
            moba("fragpunk", "FragPunk", Archetype.MOBA_BRAWLER, "fragpunk.exe", tags = listOf("card", "hero")),
            moba("mecha-break", "Mecha BREAK", Archetype.MOBA_BRAWLER, "mechabreak.exe", tags = listOf("mecha")),
            moba("awesomenauts", "Awesomenauts", Archetype.MOBA_BRAWLER, "awesomenauts.exe", tags = listOf("2d")),
            moba("heroes-of-newerth", "Heroes of Newerth", Archetype.MOBA_CLASSIC, "hon_x64.exe", tags = listOf("hon")),
            moba("vainglory", "Vainglory PC", Archetype.MOBA_CLASSIC, "vainglory.exe", tags = listOf("vg")),
            moba("honor-of-kings", "Honor of Kings PC", Archetype.MOBA_CLASSIC, "honorofkings.exe", tags = listOf("hok", "wzry")),
            moba("arena-of-valor", "Arena of Valor PC", Archetype.MOBA_CLASSIC, "arenaofvalor.exe", tags = listOf("aov")),
            moba("mobile-legends", "Mobile Legends PC", Archetype.MOBA_CLASSIC, "mlbb.exe", tags = listOf("mlbb")),
            moba("wild-rift-pc", "LoL: Wild Rift (Emulator)", Archetype.MOBA_CLASSIC, "wildrift.exe", tags = listOf("wr", "lolm")),
            moba("pokemon-unite", "Pokemon UNITE (PC)", Archetype.MOBA_BRAWLER, "pokemonunite.exe", tags = listOf("pokemon")),
            moba("onmyoji-arena", "Onmyoji Arena PC", Archetype.MOBA_CLASSIC, "onmyojiarena.exe", tags = listOf("oa")),
            moba("300-heroes", "300 Heroes", Archetype.MOBA_CLASSIC, "300.exe", tags = listOf("anime")),
            moba("heroes-evolved", "Heroes Evolved", Archetype.MOBA_CLASSIC, "heroesevolved.exe", tags = listOf("he")),
            moba("brawl-stars-pc", "Brawl Stars (PC)", Archetype.MOBA_BRAWLER, "brawlstars.exe", tags = listOf("bs")),
            moba("bloodline-champions", "Bloodline Champions", Archetype.MOBA_BRAWLER, "bloodlinechampions.exe", tags = listOf("blc")),
            moba("strife-moba", "Strife", Archetype.MOBA_CLASSIC, "strife.exe", tags = listOf("s2")),
            moba("demigod", "Demigod", Archetype.MOBA_CLASSIC, "demigod.exe", tags = listOf("classic")),
            moba("monday-night-combat", "Monday Night Combat", Archetype.MOBA_BRAWLER, "mnc.exe", tags = listOf("mnc")),
            moba("paragon-overprime", "Paragon: The Overprime", Archetype.MOBA_BRAWLER, "overprime.exe", tags = listOf("paragon")),
            moba("fault-moba", "Fault", Archetype.MOBA_BRAWLER, "fault.exe", tags = listOf("paragon")),
            moba("albion-arena", "Albion Online Arena", Archetype.MOBA_CLASSIC, "albion-online-arena.exe", tags = listOf("albion")),
            moba("lost-ark-pvp", "Lost Ark PvP Arena", Archetype.MOBA_BRAWLER, "lostark-pvp.exe", tags = listOf("lostark")),
            moba("bleeding-edge", "Bleeding Edge", Archetype.MOBA_BRAWLER, "bleedingedge.exe", tags = listOf("brawler")),
            moba("spellbreak", "Spellbreak", Archetype.MOBA_BRAWLER, "g3-win64-shipping.exe", tags = listOf("magic")),
            moba("kurtzpel", "KurtzPel", Archetype.MOBA_BRAWLER, "thechase-win64-shipping.exe", tags = listOf("anime")),
            moba("smash-legends", "SMASH LEGENDS", Archetype.MOBA_BRAWLER, "smashlegends.exe", tags = listOf("brawler")),
            moba("my-hero-ultra-rumble", "My Hero Ultra Rumble", Archetype.MOBA_BRAWLER, "mhur.exe", tags = listOf("anime")),
            moba("gundam-evolution", "Gundam Evolution", Archetype.MOBA_BRAWLER, "evogame.exe", tags = listOf("gundam")),
            moba("hyper-universe", "Hyper Universe", Archetype.MOBA_BRAWLER, "hyperuniverse.exe", tags = listOf("2d")),
            moba("mxm-arena", "Master X Master", Archetype.MOBA_CLASSIC, "mxm.exe", tags = listOf("mxm")),
            moba("corepunk", "Corepunk", Archetype.MOBA_CLASSIC, "corepunk.exe", tags = listOf("mmoba")),

            // === RPG (96) ===
            rpg("genshin-impact", "Genshin Impact", Archetype.RPG_OPEN_WORLD, "genshinimpact.exe", "yuanshen.exe", tags = listOf("genshin", "mihoyo", "hoyoverse", "ys")),
            rpg("honkai-star-rail", "Honkai: Star Rail", Archetype.RPG_TURN_BASED, "starrail.exe", tags = listOf("hsr", "hoyoverse")),
            rpg("zenless-zone-zero", "Zenless Zone Zero", Archetype.RPG_OPEN_WORLD, "zenlesszonezero.exe", tags = listOf("zzz", "hoyoverse")),
            rpg("wuthering-waves", "Wuthering Waves", Archetype.RPG_OPEN_WORLD, "client-win64-shipping.exe", "wutheringwaves.exe", tags = listOf("wuwa", "kuro")),
            rpg("world-of-warcraft", "World of Warcraft", Archetype.RPG_MMO, "wow.exe", "wowclassic.exe", tags = listOf("wow", "blizzard", "mmo")),
            rpg("ff14", "Final Fantasy XIV", Archetype.RPG_MMO, "ffxiv_dx11.exe", tags = listOf("ffxiv", "ff14", "mmo")),
            rpg("guild-wars-2", "Guild Wars 2", Archetype.RPG_MMO, "gw2-64.exe", tags = listOf("gw2", "mmo")),
            rpg("elder-scrolls-online", "The Elder Scrolls Online", Archetype.RPG_MMO, "eso64.exe", tags = listOf("eso", "mmo")),
            rpg("lost-ark", "Lost Ark", Archetype.RPG_ARPG, "lostark.exe", tags = listOf("la", "mmo")),
            rpg("black-desert", "Black Desert Online", Archetype.RPG_MMO, "blackdesert64.exe", tags = listOf("bdo", "mmo")),
            rpg("maplestory", "MapleStory", Archetype.RPG_MMO, "maplestory.exe", tags = listOf("maple", "mmo")),
            rpg("runescape", "RuneScape", Archetype.RPG_MMO, "rs2client.exe", tags = listOf("rs3", "mmo")),
            rpg("osrs", "Old School RuneScape", Archetype.RPG_MMO, "runelite.exe", "osclient.exe", tags = listOf("osrs", "mmo")),
            rpg("path-of-exile", "Path of Exile", Archetype.RPG_ARPG, "pathofexile.exe", "pathofexilesteam.exe", tags = listOf("poe", "arpg")),
            rpg("path-of-exile-2", "Path of Exile 2", Archetype.RPG_ARPG, "pathofexile2.exe", "pathofexile2steam.exe", tags = listOf("poe2", "arpg")),
            rpg("diablo-4", "Diablo IV", Archetype.RPG_ARPG, "diablo iv.exe", tags = listOf("d4", "blizzard", "arpg")),
            rpg("diablo-2-resurrected", "Diablo II: Resurrected", Archetype.RPG_ARPG, "d2r.exe", tags = listOf("d2r", "arpg")),
            rpg("diablo-3", "Diablo III", Archetype.RPG_ARPG, "diablo iii64.exe", tags = listOf("d3", "arpg")),
            rpg("last-epoch", "Last Epoch", Archetype.RPG_ARPG, "last epoch.exe", tags = listOf("le", "arpg")),
            rpg("grim-dawn", "Grim Dawn", Archetype.RPG_ARPG, "grim dawn.exe", tags = listOf("gd", "arpg")),
            rpg("torchlight-infinite", "Torchlight: Infinite", Archetype.RPG_ARPG, "torchlight_infinite.exe", tags = listOf("tli", "arpg")),
            rpg("baldurs-gate-3", "Baldur's Gate 3", Archetype.RPG_TURN_BASED, "bg3.exe", "bg3_dx11.exe", tags = listOf("bg3", "larian", "crpg")),
            rpg("divinity-os2", "Divinity: Original Sin 2", Archetype.RPG_TURN_BASED, "eocapp.exe", tags = listOf("dos2", "crpg")),
            rpg("elden-ring", "Elden Ring", Archetype.RPG_OPEN_WORLD, "eldenring.exe", tags = listOf("er", "fromsoft")),
            rpg("cyberpunk-2077", "Cyberpunk 2077", Archetype.RPG_OPEN_WORLD, "cyberpunk2077.exe", tags = listOf("cp2077", "cdpr")),
            rpg("witcher-3", "The Witcher 3: Wild Hunt", Archetype.RPG_OPEN_WORLD, "witcher3.exe", tags = listOf("w3", "cdpr")),
            rpg("skyrim", "The Elder Scrolls V: Skyrim", Archetype.RPG_OPEN_WORLD, "skyrimse.exe", "tesv.exe", tags = listOf("tes5", "bethesda")),
            rpg("fallout-4", "Fallout 4", Archetype.RPG_OPEN_WORLD, "fallout4.exe", tags = listOf("fo4", "bethesda")),
            rpg("fallout-new-vegas", "Fallout: New Vegas", Archetype.RPG_OPEN_WORLD, "falloutnv.exe", tags = listOf("fnv", "obsidian")),
            rpg("fallout-76", "Fallout 76", Archetype.RPG_OPEN_WORLD, "fallout76.exe", tags = listOf("fo76")),
            rpg("starfield", "Starfield", Archetype.RPG_OPEN_WORLD, "starfield.exe", tags = listOf("bethesda")),
            rpg("mass-effect-le", "Mass Effect Legendary Edition", Archetype.RPG_OPEN_WORLD, "masseffect1.exe", "masseffect2.exe", "masseffect3.exe", tags = listOf("me")),
            rpg("dragon-age-inquisition", "Dragon Age: Inquisition", Archetype.RPG_OPEN_WORLD, "dragonageinquisition.exe", tags = listOf("dai")),
            rpg("dragon-age-veilguard", "Dragon Age: The Veilguard", Archetype.RPG_OPEN_WORLD, "dragon age the veilguard.exe", tags = listOf("dav")),
            rpg("persona-5-royal", "Persona 5 Royal", Archetype.RPG_TURN_BASED, "p5r.exe", tags = listOf("p5r", "atlus", "jrpg")),
            rpg("persona-3-reload", "Persona 3 Reload", Archetype.RPG_TURN_BASED, "p3r.exe", tags = listOf("p3r", "atlus", "jrpg")),
            rpg("metaphor-refantazio", "Metaphor: ReFantazio", Archetype.RPG_TURN_BASED, "metaphor.exe", tags = listOf("atlus", "jrpg")),
            rpg("yakuza-lad", "Yakuza: Like a Dragon", Archetype.RPG_TURN_BASED, "yakuzalikeadragon.exe", tags = listOf("lad", "jrpg")),
            rpg("lad-infinite-wealth", "Like a Dragon: Infinite Wealth", Archetype.RPG_TURN_BASED, "likeadragon8.exe", tags = listOf("lad8", "jrpg")),
            rpg("ff7-remake", "Final Fantasy VII Remake", Archetype.RPG_OPEN_WORLD, "ff7remake_.exe", tags = listOf("ff7r", "jrpg")),
            rpg("ff16", "Final Fantasy XVI", Archetype.RPG_OPEN_WORLD, "ffxvi.exe", tags = listOf("ff16", "jrpg")),
            rpg("dragons-dogma-2", "Dragon's Dogma 2", Archetype.RPG_OPEN_WORLD, "dd2.exe", tags = listOf("dd2", "capcom")),
            rpg("monster-hunter-world", "Monster Hunter: World", Archetype.RPG_OPEN_WORLD, "monsterhunterworld.exe", tags = listOf("mhw", "capcom")),
            rpg("monster-hunter-rise", "Monster Hunter Rise", Archetype.RPG_OPEN_WORLD, "monsterhunterrise.exe", tags = listOf("mhr", "capcom")),
            rpg("monster-hunter-wilds", "Monster Hunter Wilds", Archetype.RPG_OPEN_WORLD, "monsterhunterwilds.exe", tags = listOf("mhwilds", "capcom")),
            rpg("kingdom-come", "Kingdom Come: Deliverance", Archetype.RPG_OPEN_WORLD, "kingdomcome.exe", tags = listOf("kcd")),
            rpg("kingdom-come-2", "Kingdom Come: Deliverance II", Archetype.RPG_OPEN_WORLD, "kingdomcome2.exe", tags = listOf("kcd2")),
            rpg("bannerlord", "Mount & Blade II: Bannerlord", Archetype.RPG_OPEN_WORLD, "bannerlord.exe", tags = listOf("mb2")),
            rpg("kenshi", "Kenshi", Archetype.RPG_ARPG, "kenshi_x64.exe", tags = listOf("sandbox")),
            rpg("disco-elysium", "Disco Elysium", Archetype.RPG_TURN_BASED, "disco.exe", tags = listOf("crpg")),
            rpg("pillars-of-eternity-2", "Pillars of Eternity II", Archetype.RPG_TURN_BASED, "pillarsii.exe", tags = listOf("poe2", "crpg")),
            rpg("pathfinder-wotr", "Pathfinder: Wrath of the Righteous", Archetype.RPG_TURN_BASED, "wrath.exe", tags = listOf("wotr", "crpg")),
            rpg("rogue-trader", "Warhammer 40,000: Rogue Trader", Archetype.RPG_TURN_BASED, "wh40krt.exe", tags = listOf("crpg")),
            rpg("wasteland-3", "Wasteland 3", Archetype.RPG_TURN_BASED, "wl3.exe", tags = listOf("crpg")),
            rpg("octopath-traveler-2", "Octopath Traveler II", Archetype.RPG_TURN_BASED, "octopath_traveler2-win64-shipping.exe", tags = listOf("jrpg")),
            rpg("triangle-strategy", "Triangle Strategy", Archetype.RPG_TURN_BASED, "trianglestrategy-win64-shipping.exe", tags = listOf("srpg")),
            rpg("tales-of-arise", "Tales of Arise", Archetype.RPG_OPEN_WORLD, "tales of arise.exe", tags = listOf("jrpg")),
            rpg("granblue-relink", "Granblue Fantasy: Relink", Archetype.RPG_OPEN_WORLD, "granblue_fantasy_relink.exe", tags = listOf("gbf", "jrpg")),
            rpg("nier-automata", "NieR: Automata", Archetype.RPG_OPEN_WORLD, "nierautomata.exe", tags = listOf("nier")),
            rpg("nier-replicant", "NieR Replicant", Archetype.RPG_OPEN_WORLD, "nier replicant ver.1.22474487139.exe", tags = listOf("nier")),
            rpg("kingdom-hearts-3", "Kingdom Hearts III", Archetype.RPG_OPEN_WORLD, "kingdom hearts iii.exe", tags = listOf("kh3")),
            rpg("scarlet-nexus", "SCARLET NEXUS", Archetype.RPG_OPEN_WORLD, "scarletnexus-win64-shipping.exe", tags = listOf("anime")),
            rpg("code-vein", "CODE VEIN", Archetype.RPG_OPEN_WORLD, "codevein-win64-shipping.exe", tags = listOf("souls")),
            rpg("lies-of-p", "Lies of P", Archetype.RPG_OPEN_WORLD, "lop-win64-shipping.exe", tags = listOf("souls")),
            rpg("lords-of-the-fallen", "Lords of the Fallen", Archetype.RPG_OPEN_WORLD, "lotf2-win64-shipping.exe", tags = listOf("souls")),
            rpg("remnant-2", "Remnant II", Archetype.RPG_OPEN_WORLD, "remnant2-win64-shipping.exe", tags = listOf("coop")),
            rpg("palworld", "Palworld", Archetype.RPG_OPEN_WORLD, "palworld-win64-shipping.exe", tags = listOf("survival")),
            rpg("enshrouded", "Enshrouded", Archetype.RPG_OPEN_WORLD, "enshrouded.exe", tags = listOf("survival")),
            rpg("valheim", "Valheim", Archetype.RPG_OPEN_WORLD, "valheim.exe", tags = listOf("survival")),
            rpg("v-rising", "V Rising", Archetype.RPG_ARPG, "vrising.exe", tags = listOf("survival")),
            rpg("terraria", "Terraria", Archetype.RPG_ARPG, "terraria.exe", tags = listOf("2d", "sandbox")),
            rpg("starbound", "Starbound", Archetype.RPG_ARPG, "starbound.exe", tags = listOf("2d", "sandbox")),
            rpg("core-keeper", "Core Keeper", Archetype.RPG_ARPG, "corekeeper.exe", tags = listOf("sandbox")),
            rpg("necesse", "Necesse", Archetype.RPG_ARPG, "necesse.exe", tags = listOf("sandbox")),
            rpg("stardew-valley", "Stardew Valley", Archetype.RPG_TURN_BASED, "stardew valley.exe", tags = listOf("farming", "cozy")),
            rpg("sun-haven", "Sun Haven", Archetype.RPG_TURN_BASED, "sun haven.exe", tags = listOf("farming")),
            rpg("my-time-at-sandrock", "My Time at Sandrock", Archetype.RPG_OPEN_WORLD, "sandrock.exe", tags = listOf("cozy")),
            rpg("rune-factory-5", "Rune Factory 5", Archetype.RPG_OPEN_WORLD, "rune factory 5.exe", tags = listOf("jrpg")),
            rpg("undertale", "UNDERTALE", Archetype.RPG_TURN_BASED, "undertale.exe", tags = listOf("indie")),
            rpg("deltarune", "DELTARUNE", Archetype.RPG_TURN_BASED, "deltarune.exe", tags = listOf("indie")),
            rpg("omori", "OMORI", Archetype.RPG_TURN_BASED, "omori.exe", tags = listOf("indie")),
            rpg("sea-of-stars", "Sea of Stars", Archetype.RPG_TURN_BASED, "seaofstars.exe", tags = listOf("jrpg")),
            rpg("chained-echoes", "Chained Echoes", Archetype.RPG_TURN_BASED, "chained echoes.exe", tags = listOf("jrpg")),
            rpg("chrono-trigger", "Chrono Trigger", Archetype.RPG_TURN_BASED, "chrono trigger.exe", tags = listOf("jrpg")),
            rpg("dragon-quest-11", "Dragon Quest XI S", Archetype.RPG_TURN_BASED, "dragon quest xi s.exe", tags = listOf("dq11", "jrpg")),
            rpg("smt-v-vengeance", "Shin Megami Tensei V: Vengeance", Archetype.RPG_TURN_BASED, "smt5v-win64-shipping.exe", tags = listOf("smtv", "jrpg")),
            rpg("new-world", "New World: Aeternum", Archetype.RPG_MMO, "newworld.exe", tags = listOf("mmo")),
            rpg("throne-and-liberty", "THRONE AND LIBERTY", Archetype.RPG_MMO, "tl.exe", tags = listOf("tl", "mmo")),
            rpg("pso2-ngs", "Phantasy Star Online 2 NGS", Archetype.RPG_MMO, "pso2.exe", tags = listOf("pso2", "mmo")),
            rpg("warframe", "Warframe", Archetype.RPG_OPEN_WORLD, "warframe.x64.exe", tags = listOf("wf", "looter")),
            rpg("tower-of-fantasy", "Tower of Fantasy", Archetype.RPG_OPEN_WORLD, "qrsl.exe", tags = listOf("tof", "mmo")),
            rpg("once-human", "Once Human", Archetype.RPG_OPEN_WORLD, "once_human.exe", tags = listOf("survival", "mmo")),
            rpg("dark-and-darker", "Dark and Darker", Archetype.RPG_OPEN_WORLD, "dungeoncrawler.exe", tags = listOf("dad", "extraction")),
            rpg("albion-online", "Albion Online", Archetype.RPG_MMO, "albion-online.exe", tags = listOf("mmo")),
            rpg("everquest-2", "EverQuest II", Archetype.RPG_MMO, "everquest2.exe", tags = listOf("eq2", "mmo")),
            rpg("star-wars-tor", "Star Wars: The Old Republic", Archetype.RPG_MMO, "swtor.exe", tags = listOf("swtor", "mmo")),

            // === ACTION (82) ===
            action("black-myth-wukong", "Black Myth: Wukong", Archetype.ACTION_SOULS, "b1-win64-shipping.exe", tags = listOf("wukong", "bmw", "heishenhua")),
            action("sekiro", "Sekiro: Shadows Die Twice", Archetype.ACTION_SOULS, "sekiro.exe", tags = listOf("fromsoft")),
            action("dark-souls-3", "Dark Souls III", Archetype.ACTION_SOULS, "darksoulsiii.exe", tags = listOf("ds3", "fromsoft")),
            action("dark-souls-remastered", "Dark Souls: Remastered", Archetype.ACTION_SOULS, "darksoulsremastered.exe", tags = listOf("ds1", "fromsoft")),
            action("armored-core-6", "Armored Core VI", Archetype.ACTION_HACK_SLASH, "armoredcore6.exe", tags = listOf("ac6", "mecha")),
            action("dmc-5", "Devil May Cry 5", Archetype.ACTION_HACK_SLASH, "devilmaycry5.exe", tags = listOf("dmc5", "capcom")),
            action("bayonetta", "Bayonetta", Archetype.ACTION_HACK_SLASH, "bayonetta.exe", tags = listOf("platinum")),
            action("metal-gear-rising", "Metal Gear Rising: Revengeance", Archetype.ACTION_HACK_SLASH, "metal gear rising revengeance.exe", tags = listOf("mgr")),
            action("god-of-war", "God of War", Archetype.ACTION_HACK_SLASH, "gow.exe", tags = listOf("gow")),
            action("god-of-war-ragnarok", "God of War Ragnarok", Archetype.ACTION_HACK_SLASH, "gowr.exe", tags = listOf("gowr")),
            action("ghost-of-tsushima", "Ghost of Tsushima", Archetype.ACTION_HACK_SLASH, "ghostoftsushima.exe", tags = listOf("got")),
            action("horizon-zero-dawn", "Horizon Zero Dawn", Archetype.ACTION_STEALTH, "horizonzerodawn.exe", tags = listOf("hzd")),
            action("horizon-forbidden-west", "Horizon Forbidden West", Archetype.ACTION_STEALTH, "horizonforbiddenwest.exe", tags = listOf("hfw")),
            action("spider-man-remastered", "Marvel's Spider-Man Remastered", Archetype.ACTION_HACK_SLASH, "spider-man.exe", tags = listOf("spiderman")),
            action("spider-man-miles", "Spider-Man: Miles Morales", Archetype.ACTION_HACK_SLASH, "milesmorales.exe", tags = listOf("spiderman")),
            action("batman-arkham-knight", "Batman: Arkham Knight", Archetype.ACTION_HACK_SLASH, "batmanak.exe", tags = listOf("batman")),
            action("batman-arkham-city", "Batman: Arkham City", Archetype.ACTION_HACK_SLASH, "batmanac.exe", tags = listOf("batman")),
            action("shadow-of-war", "Middle-earth: Shadow of War", Archetype.ACTION_HACK_SLASH, "shadowofwar.exe", tags = listOf("lotr")),
            action("ac-valhalla", "Assassin's Creed Valhalla", Archetype.ACTION_STEALTH, "acvalhalla.exe", tags = listOf("ac", "ubisoft")),
            action("ac-mirage", "Assassin's Creed Mirage", Archetype.ACTION_STEALTH, "acmirage.exe", tags = listOf("ac", "ubisoft")),
            action("ac-odyssey", "Assassin's Creed Odyssey", Archetype.ACTION_STEALTH, "acodyssey.exe", tags = listOf("ac", "ubisoft")),
            action("ac-shadows", "Assassin's Creed Shadows", Archetype.ACTION_STEALTH, "acshadows.exe", tags = listOf("ac", "ubisoft")),
            action("rdr2", "Red Dead Redemption 2", Archetype.ACTION_STEALTH, "rdr2.exe", tags = listOf("rockstar")),
            action("gta-5", "Grand Theft Auto V", Archetype.ACTION_HACK_SLASH, "gta5.exe", "playgtav.exe", tags = listOf("gtav", "rockstar")),
            action("watch-dogs-legion", "Watch Dogs: Legion", Archetype.ACTION_STEALTH, "watchdogslegion.exe", tags = listOf("wdl")),
            action("sleeping-dogs", "Sleeping Dogs", Archetype.ACTION_HACK_SLASH, "sdhdship.exe", tags = listOf("openworld")),
            action("mafia-definitive", "Mafia: Definitive Edition", Archetype.ACTION_STEALTH, "mafiadefinitiveedition.exe", tags = listOf("mafia")),
            action("hitman-woa", "HITMAN World of Assassination", Archetype.ACTION_STEALTH, "hitman3.exe", tags = listOf("hitman", "stealth")),
            action("mgsv", "Metal Gear Solid V", Archetype.ACTION_STEALTH, "mgsvtpp.exe", tags = listOf("mgs", "stealth")),
            action("splinter-cell-blacklist", "Splinter Cell Blacklist", Archetype.ACTION_STEALTH, "blacklist_dx11_game.exe", tags = listOf("stealth")),
            action("dishonored-2", "Dishonored 2", Archetype.ACTION_STEALTH, "dishonored2.exe", tags = listOf("arkane")),
            action("deathloop", "DEATHLOOP", Archetype.ACTION_STEALTH, "deathloop.exe", tags = listOf("arkane")),
            action("control", "Control", Archetype.ACTION_HACK_SLASH, "control_dx12.exe", tags = listOf("remedy")),
            action("alan-wake-2", "Alan Wake 2", Archetype.ACTION_STEALTH, "alanwake2.exe", tags = listOf("remedy", "horror")),
            action("quantum-break", "Quantum Break", Archetype.ACTION_HACK_SLASH, "quantumbreak.exe", tags = listOf("remedy")),
            action("re4-remake", "Resident Evil 4 Remake", Archetype.ACTION_STEALTH, "re4.exe", tags = listOf("re4", "capcom")),
            action("re2-remake", "Resident Evil 2 Remake", Archetype.ACTION_STEALTH, "re2.exe", tags = listOf("re2", "capcom")),
            action("re-village", "Resident Evil Village", Archetype.ACTION_STEALTH, "re8.exe", tags = listOf("re8", "capcom")),
            action("silent-hill-2", "SILENT HILL 2 Remake", Archetype.ACTION_STEALTH, "shproto-win64-shipping.exe", tags = listOf("sh2", "horror")),
            action("dead-space-remake", "Dead Space Remake", Archetype.ACTION_STEALTH, "dead space.exe", tags = listOf("horror")),
            action("dying-light-2", "Dying Light 2", Archetype.ACTION_HACK_SLASH, "dyinglightgame_x64_rwdi.exe", tags = listOf("dl2", "parkour")),
            action("dead-island-2", "Dead Island 2", Archetype.ACTION_HACK_SLASH, "deadisland-win64-shipping.exe", tags = listOf("zombie")),
            action("tlou-part-1", "The Last of Us Part I", Archetype.ACTION_STEALTH, "tlou-i.exe", tags = listOf("tlou")),
            action("uncharted-4", "UNCHARTED: Legacy of Thieves", Archetype.ACTION_STEALTH, "u4.exe", tags = listOf("uncharted")),
            action("tomb-raider", "Tomb Raider", Archetype.ACTION_STEALTH, "tombraider.exe", tags = listOf("lara")),
            action("shadow-tomb-raider", "Shadow of the Tomb Raider", Archetype.ACTION_STEALTH, "sottr.exe", tags = listOf("lara")),
            action("jedi-fallen-order", "Star Wars Jedi: Fallen Order", Archetype.ACTION_SOULS, "starwarsjedifallenorder.exe", tags = listOf("jedi")),
            action("jedi-survivor", "Star Wars Jedi: Survivor", Archetype.ACTION_SOULS, "jedisurvivor.exe", tags = listOf("jedi")),
            action("sifu", "Sifu", Archetype.ACTION_HACK_SLASH, "sifu-win64-shipping.exe", tags = listOf("kungfu")),
            action("hades", "Hades", Archetype.ACTION_HACK_SLASH, "hades.exe", tags = listOf("roguelike")),
            action("hades-2", "Hades II", Archetype.ACTION_HACK_SLASH, "hades2.exe", tags = listOf("roguelike")),
            action("dead-cells", "Dead Cells", Archetype.ACTION_PLATFORMER, "deadcells.exe", tags = listOf("roguelike")),
            action("hollow-knight", "Hollow Knight", Archetype.ACTION_PLATFORMER, "hollow_knight.exe", tags = listOf("metroidvania")),
            action("celeste", "Celeste", Archetype.ACTION_PLATFORMER, "celeste.exe", tags = listOf("platformer")),
            action("cuphead", "Cuphead", Archetype.ACTION_PLATFORMER, "cuphead.exe", tags = listOf("runandgun")),
            action("ori-wotw", "Ori and the Will of the Wisps", Archetype.ACTION_PLATFORMER, "oriwotw.exe", tags = listOf("metroidvania")),
            action("nine-sols", "Nine Sols", Archetype.ACTION_PLATFORMER, "ninesols.exe", tags = listOf("metroidvania")),
            action("blasphemous-2", "Blasphemous 2", Archetype.ACTION_PLATFORMER, "blasphemous 2.exe", tags = listOf("metroidvania")),
            action("katana-zero", "Katana ZERO", Archetype.ACTION_PLATFORMER, "katana zero.exe", tags = listOf("indie")),
            action("hotline-miami", "Hotline Miami", Archetype.ACTION_HACK_SLASH, "hotlinegl.exe", tags = listOf("indie")),
            action("enter-the-gungeon", "Enter the Gungeon", Archetype.ACTION_HACK_SLASH, "etg.exe", tags = listOf("roguelike")),
            action("binding-of-isaac", "The Binding of Isaac", Archetype.ACTION_PLATFORMER, "isaac-ng.exe", tags = listOf("roguelike")),
            action("vampire-survivors", "Vampire Survivors", Archetype.ACTION_PLATFORMER, "vampiresurvivors.exe", tags = listOf("bulletheaven")),
            action("brotato", "Brotato", Archetype.ACTION_PLATFORMER, "brotato.exe", tags = listOf("roguelike")),
            action("cult-of-the-lamb", "Cult of the Lamb", Archetype.ACTION_HACK_SLASH, "cult of the lamb.exe", tags = listOf("indie")),
            action("dave-the-diver", "DAVE THE DIVER", Archetype.ACTION_PLATFORMER, "davethediver.exe", tags = listOf("indie")),
            action("subnautica", "Subnautica", Archetype.ACTION_STEALTH, "subnautica.exe", tags = listOf("survival")),
            action("no-mans-sky", "No Man's Sky", Archetype.ACTION_HACK_SLASH, "nms.exe", tags = listOf("nms", "space")),
            action("minecraft", "Minecraft", Archetype.ACTION_HACK_SLASH, "minecraft.windows.exe", "javaw.exe", tags = listOf("mc", "sandbox")),
            action("roblox", "Roblox", Archetype.ACTION_HACK_SLASH, "robloxplayerbeta.exe", tags = listOf("sandbox")),
            action("fall-guys", "Fall Guys", Archetype.ACTION_PLATFORMER, "fallguys_client_game.exe", tags = listOf("party")),
            action("rocket-league", "Rocket League", Archetype.ACTION_PLATFORMER, "rocketleague.exe", tags = listOf("rl", "sports")),
            action("street-fighter-6", "Street Fighter 6", Archetype.ACTION_FIGHTING, "streetfighter6.exe", tags = listOf("sf6", "fighting")),
            action("tekken-8", "TEKKEN 8", Archetype.ACTION_FIGHTING, "polaris-win64-shipping.exe", tags = listOf("t8", "fighting")),
            action("guilty-gear-strive", "Guilty Gear -Strive-", Archetype.ACTION_FIGHTING, "ggst-win64-shipping.exe", tags = listOf("ggst", "fighting")),
            action("mortal-kombat-1", "Mortal Kombat 1", Archetype.ACTION_FIGHTING, "mk12.exe", tags = listOf("mk1", "fighting")),
            action("dragon-ball-sparking", "DRAGON BALL: Sparking! ZERO", Archetype.ACTION_FIGHTING, "sparkingzero-win64-shipping.exe", tags = listOf("dbz", "fighting")),
            action("brawlhalla", "Brawlhalla", Archetype.ACTION_FIGHTING, "brawlhalla.exe", tags = listOf("fighting")),
            action("for-honor", "For Honor", Archetype.ACTION_SOULS, "forhonor.exe", tags = listOf("melee")),
            action("sea-of-thieves", "Sea of Thieves", Archetype.ACTION_HACK_SLASH, "sotgame.exe", tags = listOf("sot", "pirate")),
            action("stellar-blade", "Stellar Blade", Archetype.ACTION_SOULS, "sb-win64-shipping.exe", tags = listOf("action")),
            action("warhammer-space-marine-2", "Warhammer 40,000: Space Marine 2", Archetype.ACTION_HACK_SLASH, "warhammer 40000 space marine 2 - retail.exe", tags = listOf("sm2")),

            // === RACING (45) ===
            racing("forza-horizon-5", "Forza Horizon 5", Archetype.RACING_ARCADE, "forzahorizon5.exe", tags = listOf("fh5", "cars")),
            racing("forza-horizon-4", "Forza Horizon 4", Archetype.RACING_ARCADE, "forzahorizon4.exe", tags = listOf("fh4")),
            racing("forza-motorsport", "Forza Motorsport", Archetype.RACING_SIM, "forza_gaming.desktop.x64_release_final.exe", tags = listOf("fm")),
            racing("assetto-corsa", "Assetto Corsa", Archetype.RACING_SIM, "acs.exe", tags = listOf("ac", "simracing")),
            racing("assetto-corsa-comp", "Assetto Corsa Competizione", Archetype.RACING_SIM, "ac2-win64-shipping.exe", tags = listOf("acc", "simracing")),
            racing("iracing", "iRacing", Archetype.RACING_SIM, "iracingsim64dx11.exe", tags = listOf("simracing")),
            racing("f1-24", "EA SPORTS F1 24", Archetype.RACING_SIM, "f1_24.exe", tags = listOf("f1", "formula")),
            racing("f1-23", "EA SPORTS F1 23", Archetype.RACING_SIM, "f1_23.exe", tags = listOf("f1", "formula")),
            racing("ea-wrc", "EA SPORTS WRC", Archetype.RACING_SIM, "wrc.exe", tags = listOf("rally")),
            racing("dirt-rally-2", "DiRT Rally 2.0", Archetype.RACING_SIM, "dirtrally2.exe", tags = listOf("rally")),
            racing("project-cars-2", "Project CARS 2", Archetype.RACING_SIM, "pcars2avx.exe", tags = listOf("pcars")),
            racing("automobilista-2", "Automobilista 2", Archetype.RACING_SIM, "ams2avx.exe", tags = listOf("ams2")),
            racing("rfactor-2", "rFactor 2", Archetype.RACING_SIM, "rfactor2.exe", tags = listOf("rf2")),
            racing("beamng-drive", "BeamNG.drive", Archetype.RACING_SIM, "beamng.drive.x64.exe", tags = listOf("beamng", "physics")),
            racing("nfs-unbound", "Need for Speed Unbound", Archetype.RACING_ARCADE, "needforspeedunbound.exe", tags = listOf("nfs")),
            racing("nfs-heat", "Need for Speed Heat", Archetype.RACING_ARCADE, "needforspeedheat.exe", tags = listOf("nfs")),
            racing("nfs-most-wanted", "Need for Speed: Most Wanted", Archetype.RACING_ARCADE, "nfs13.exe", tags = listOf("nfs")),
            racing("burnout-paradise", "Burnout Paradise Remastered", Archetype.RACING_ARCADE, "burnoutpr.exe", tags = listOf("burnout")),
            racing("the-crew-motorfest", "The Crew Motorfest", Archetype.RACING_ARCADE, "thecrewmotorfest.exe", tags = listOf("tcm")),
            racing("the-crew-2", "The Crew 2", Archetype.RACING_ARCADE, "thecrew2.exe", tags = listOf("tc2")),
            racing("tdu-solar-crown", "Test Drive Unlimited Solar Crown", Archetype.RACING_ARCADE, "tdusc.exe", tags = listOf("tdu")),
            racing("trackmania", "Trackmania", Archetype.RACING_ARCADE, "trackmania.exe", tags = listOf("tm")),
            racing("wreckfest", "Wreckfest", Archetype.RACING_ARCADE, "wreckfest_x64.exe", tags = listOf("derby")),
            racing("snowrunner", "SnowRunner", Archetype.RACING_SIM, "snowrunner.exe", tags = listOf("offroad")),
            racing("mudrunner", "MudRunner", Archetype.RACING_SIM, "mudrunner.exe", tags = listOf("offroad")),
            racing("expeditions-mudrunner", "Expeditions: A MudRunner Game", Archetype.RACING_SIM, "expeditions.exe", tags = listOf("offroad")),
            racing("ets2", "Euro Truck Simulator 2", Archetype.RACING_SIM, "eurotrucks2.exe", tags = listOf("ets2", "truck")),
            racing("ats", "American Truck Simulator", Archetype.RACING_SIM, "amtrucks.exe", tags = listOf("ats", "truck")),
            racing("carx-drift", "CarX Drift Racing Online", Archetype.RACING_ARCADE, "drift racing online.exe", tags = listOf("drift")),
            racing("carx-street", "CarX Street", Archetype.RACING_ARCADE, "carx street.exe", tags = listOf("street")),
            racing("drift-ce", "DRIFT CE", Archetype.RACING_SIM, "driftce.exe", tags = listOf("drift")),
            racing("art-of-rally", "art of rally", Archetype.RACING_ARCADE, "artofrally.exe", tags = listOf("rally")),
            racing("inertial-drift", "Inertial Drift", Archetype.RACING_ARCADE, "inertialdrift.exe", tags = listOf("drift")),
            racing("horizon-chase-turbo", "Horizon Chase Turbo", Archetype.RACING_ARCADE, "horizonchaseturbo.exe", tags = listOf("retro")),
            racing("hotshot-racing", "Hotshot Racing", Archetype.RACING_ARCADE, "hotshotracing.exe", tags = listOf("retro")),
            racing("redout-2", "Redout 2", Archetype.RACING_ARCADE, "redout2-win64-shipping.exe", tags = listOf("ag")),
            racing("distance", "Distance", Archetype.RACING_ARCADE, "distance.exe", tags = listOf("arcade")),
            racing("grip-combat", "GRIP: Combat Racing", Archetype.RACING_ARCADE, "grip-win64-shipping.exe", tags = listOf("combat")),
            racing("motogp-24", "MotoGP 24", Archetype.RACING_SIM, "motogp24-win64-shipping.exe", tags = listOf("moto")),
            racing("ride-5", "RIDE 5", Archetype.RACING_SIM, "ride5-win64-shipping.exe", tags = listOf("moto")),
            racing("mx-bikes", "MX Bikes", Archetype.RACING_SIM, "mxbikes.exe", tags = listOf("moto")),
            racing("descenders", "Descenders", Archetype.RACING_ARCADE, "descenders.exe", tags = listOf("mtb")),
            racing("riders-republic", "Riders Republic", Archetype.RACING_ARCADE, "ridersrepublic.exe", tags = listOf("extreme")),
            racing("steep", "Steep", Archetype.RACING_ARCADE, "steep.exe", tags = listOf("snow")),
            racing("le-mans-ultimate", "Le Mans Ultimate", Archetype.RACING_SIM, "lemansultimate.exe", tags = listOf("lmu", "simracing")),

            // === SIMULATOR (66) ===
            sim("msfs-2024", "Microsoft Flight Simulator 2024", Archetype.SIM_FLIGHT, "flightsimulator2024.exe", tags = listOf("msfs", "flight")),
            sim("msfs-2020", "Microsoft Flight Simulator 2020", Archetype.SIM_FLIGHT, "flightsimulator.exe", tags = listOf("msfs", "flight")),
            sim("x-plane-12", "X-Plane 12", Archetype.SIM_FLIGHT, "x-plane.exe", tags = listOf("xp12", "flight")),
            sim("dcs-world", "DCS World", Archetype.SIM_FLIGHT, "dcs.exe", tags = listOf("dcs", "flight")),
            sim("war-thunder", "War Thunder", Archetype.SIM_FLIGHT, "aces.exe", tags = listOf("wt", "flight", "tank")),
            sim("il2-sturmovik", "IL-2 Sturmovik", Archetype.SIM_FLIGHT, "il-2.exe", tags = listOf("il2", "flight")),
            sim("kerbal-space-program", "Kerbal Space Program", Archetype.SIM_FLIGHT, "ksp_x64.exe", tags = listOf("ksp", "space")),
            sim("elite-dangerous", "Elite Dangerous", Archetype.SIM_FLIGHT, "elitedangerous64.exe", tags = listOf("ed", "space")),
            sim("star-citizen", "Star Citizen", Archetype.SIM_FLIGHT, "starcitizen.exe", tags = listOf("sc", "space")),
            sim("x4-foundations", "X4: Foundations", Archetype.SIM_FLIGHT, "x4.exe", tags = listOf("x4", "space")),
            sim("eve-online", "EVE Online", Archetype.SIM_STRATEGY, "exefile.exe", tags = listOf("eve", "space")),
            sim("farming-sim-25", "Farming Simulator 25", Archetype.SIM_SANDBOX, "farmingsimulator2025game.exe", tags = listOf("fs25")),
            sim("farming-sim-22", "Farming Simulator 22", Archetype.SIM_SANDBOX, "farmingsimulator2022game.exe", tags = listOf("fs22")),
            sim("cities-skylines-2", "Cities: Skylines II", Archetype.SIM_STRATEGY, "cities2.exe", tags = listOf("cs2", "city")),
            sim("cities-skylines", "Cities: Skylines", Archetype.SIM_STRATEGY, "cities.exe", tags = listOf("city")),
            sim("factorio", "Factorio", Archetype.SIM_SANDBOX, "factorio.exe", tags = listOf("factory")),
            sim("satisfactory", "Satisfactory", Archetype.SIM_SANDBOX, "factorygame-win64-shipping.exe", tags = listOf("factory")),
            sim("dyson-sphere-program", "Dyson Sphere Program", Archetype.SIM_SANDBOX, "dspgame.exe", tags = listOf("dsp", "factory")),
            sim("shapez-2", "shapez 2", Archetype.SIM_STRATEGY, "shapez 2.exe", tags = listOf("factory")),
            sim("rimworld", "RimWorld", Archetype.SIM_STRATEGY, "rimworldwin64.exe", tags = listOf("colony")),
            sim("dwarf-fortress", "Dwarf Fortress", Archetype.SIM_STRATEGY, "dwarf fortress.exe", tags = listOf("df", "colony")),
            sim("oxygen-not-included", "Oxygen Not Included", Archetype.SIM_STRATEGY, "oxygennotincluded.exe", tags = listOf("oni", "colony")),
            sim("frostpunk-2", "Frostpunk 2", Archetype.SIM_STRATEGY, "frostpunk2-win64-shipping.exe", tags = listOf("fp2", "city")),
            sim("frostpunk", "Frostpunk", Archetype.SIM_STRATEGY, "frostpunk.exe", tags = listOf("city")),
            sim("manor-lords", "Manor Lords", Archetype.SIM_STRATEGY, "manorlords-win64-shipping.exe", tags = listOf("city")),
            sim("anno-1800", "Anno 1800", Archetype.SIM_STRATEGY, "anno1800.exe", tags = listOf("anno", "city")),
            sim("tropico-6", "Tropico 6", Archetype.SIM_STRATEGY, "tropico6-win64-shipping.exe", tags = listOf("city")),
            sim("planet-coaster-2", "Planet Coaster 2", Archetype.SIM_STRATEGY, "planetcoaster2.exe", tags = listOf("park")),
            sim("planet-zoo", "Planet Zoo", Archetype.SIM_STRATEGY, "planetzoo.exe", tags = listOf("zoo")),
            sim("jurassic-world-evo-2", "Jurassic World Evolution 2", Archetype.SIM_STRATEGY, "jwe2.exe", tags = listOf("park")),
            sim("the-sims-4", "The Sims 4", Archetype.SIM_STRATEGY, "ts4_x64.exe", tags = listOf("sims")),
            sim("inzoi", "inZOI", Archetype.SIM_SANDBOX, "inzoi-win64-shipping.exe", tags = listOf("life")),
            sim("house-flipper-2", "House Flipper 2", Archetype.SIM_SANDBOX, "houseflipper2.exe", tags = listOf("design")),
            sim("powerwash-sim", "PowerWash Simulator", Archetype.SIM_SANDBOX, "powerwashsimulator.exe", tags = listOf("relax")),
            sim("pc-building-sim-2", "PC Building Simulator 2", Archetype.SIM_SANDBOX, "pcbs2.exe", tags = listOf("pcbs")),
            sim("supermarket-sim", "Supermarket Simulator", Archetype.SIM_SANDBOX, "supermarket simulator.exe", tags = listOf("shop")),
            sim("tcg-card-shop-sim", "TCG Card Shop Simulator", Archetype.SIM_SANDBOX, "card shop simulator.exe", tags = listOf("shop")),
            sim("cooking-sim", "Cooking Simulator", Archetype.SIM_SANDBOX, "cookingsimulator.exe", tags = listOf("cooking")),
            sim("car-mechanic-sim-21", "Car Mechanic Simulator 2021", Archetype.SIM_SANDBOX, "car mechanicsimulator 2021.exe", tags = listOf("cms")),
            sim("train-sim-world-5", "Train Sim World 5", Archetype.SIM_FLIGHT, "ts2prototype-win64-shipping.exe", tags = listOf("train")),
            sim("derail-valley", "Derail Valley", Archetype.SIM_FLIGHT, "derailvalley.exe", tags = listOf("train")),
            sim("bus-sim-21", "Bus Simulator 21", Archetype.SIM_SANDBOX, "bussimulator21-win64-shipping.exe", tags = listOf("bus")),
            sim("construction-sim", "Construction Simulator", Archetype.SIM_SANDBOX, "constructionsimulator.exe", tags = listOf("construction")),
            sim("firefighting-sim", "Firefighting Simulator", Archetype.SIM_SANDBOX, "firefightingsim-win64-shipping.exe", tags = listOf("fire")),
            sim("police-sim", "Police Simulator: Patrol Officers", Archetype.SIM_SANDBOX, "boston-win64-shipping.exe", tags = listOf("police")),
            sim("civ-6", "Sid Meier's Civilization VI", Archetype.SIM_STRATEGY, "civilizationvi_dx12.exe", "civilizationvi.exe", tags = listOf("civ6", "4x")),
            sim("civ-7", "Sid Meier's Civilization VII", Archetype.SIM_STRATEGY, "civ7.exe", tags = listOf("civ7", "4x")),
            sim("stellaris", "Stellaris", Archetype.SIM_STRATEGY, "stellaris.exe", tags = listOf("paradox", "4x")),
            sim("eu4", "Europa Universalis IV", Archetype.SIM_STRATEGY, "eu4.exe", tags = listOf("paradox")),
            sim("hoi4", "Hearts of Iron IV", Archetype.SIM_STRATEGY, "hoi4.exe", tags = listOf("paradox", "ww2")),
            sim("ck3", "Crusader Kings III", Archetype.SIM_STRATEGY, "ck3.exe", tags = listOf("paradox")),
            sim("victoria-3", "Victoria 3", Archetype.SIM_STRATEGY, "victoria3.exe", tags = listOf("paradox")),
            sim("total-war-wh3", "Total War: WARHAMMER III", Archetype.SIM_STRATEGY, "warhammer3.exe", tags = listOf("tw", "rts")),
            sim("aoe-4", "Age of Empires IV", Archetype.SIM_STRATEGY, "reliccardinal.exe", tags = listOf("aoe4", "rts")),
            sim("aoe-2-de", "Age of Empires II: DE", Archetype.SIM_STRATEGY, "aoe2de_s.exe", tags = listOf("aoe2", "rts")),
            sim("starcraft-2", "StarCraft II", Archetype.SIM_STRATEGY, "sc2_x64.exe", tags = listOf("sc2", "rts")),
            sim("warcraft-3", "Warcraft III: Reforged", Archetype.SIM_STRATEGY, "warcraft iii.exe", tags = listOf("wc3", "rts")),
            sim("cnc-remastered", "Command & Conquer Remastered", Archetype.SIM_STRATEGY, "clientg.exe", tags = listOf("cnc", "rts")),
            sim("company-of-heroes-3", "Company of Heroes 3", Archetype.SIM_STRATEGY, "reliccoh3.exe", tags = listOf("coh3", "rts")),
            sim("football-manager-24", "Football Manager 2024", Archetype.SIM_STRATEGY, "fm.exe", tags = listOf("fm24", "sports")),
            sim("tabletop-simulator", "Tabletop Simulator", Archetype.SIM_STRATEGY, "tabletop simulator.exe", tags = listOf("tts", "board")),
            sim("world-of-tanks", "World of Tanks", Archetype.SIM_SANDBOX, "worldoftanks.exe", tags = listOf("wot", "tank")),
            sim("world-of-warships", "World of Warships", Archetype.SIM_SANDBOX, "worldofwarships64.exe", tags = listOf("wows", "ship")),
            sim("timberborn", "Timberborn", Archetype.SIM_STRATEGY, "timberborn.exe", tags = listOf("city")),
            sim("against-the-storm", "Against the Storm", Archetype.SIM_STRATEGY, "against the storm.exe", tags = listOf("city")),
            sim("balatro", "Balatro", Archetype.SIM_STRATEGY, "balatro.exe", tags = listOf("card", "roguelike")),

            // === PRODUCTIVITY (75) ===
            prod("vscode", "Visual Studio Code", Archetype.PROD_IDE, "code.exe", "code", tags = listOf("vs", "ide", "editor")),
            prod("intellij-idea", "IntelliJ IDEA", Archetype.PROD_IDE, "idea64.exe", "idea", tags = listOf("jetbrains", "java", "kotlin")),
            prod("android-studio", "Android Studio", Archetype.PROD_IDE, "studio64.exe", "studio", tags = listOf("jetbrains", "android")),
            prod("pycharm", "PyCharm", Archetype.PROD_IDE, "pycharm64.exe", "pycharm", tags = listOf("jetbrains", "python")),
            prod("webstorm", "WebStorm", Archetype.PROD_IDE, "webstorm64.exe", "webstorm", tags = listOf("jetbrains", "js")),
            prod("clion", "CLion", Archetype.PROD_IDE, "clion64.exe", "clion", tags = listOf("jetbrains", "cpp")),
            prod("rustrover", "RustRover", Archetype.PROD_IDE, "rustrover64.exe", "rustrover", tags = listOf("jetbrains", "rust")),
            prod("goland", "GoLand", Archetype.PROD_IDE, "goland64.exe", "goland", tags = listOf("jetbrains", "go")),
            prod("rider", "JetBrains Rider", Archetype.PROD_IDE, "rider64.exe", "rider", tags = listOf("jetbrains", "dotnet")),
            prod("xcode-remote", "Xcode", Archetype.PROD_IDE, "xcode", tags = listOf("apple", "swift")),
            prod("visual-studio", "Visual Studio", Archetype.PROD_IDE, "devenv.exe", tags = listOf("msvc", "dotnet")),
            prod("cursor-ide", "Cursor", Archetype.PROD_IDE, "cursor.exe", "cursor", tags = listOf("ai", "ide")),
            prod("windsurf-ide", "Windsurf", Archetype.PROD_IDE, "windsurf.exe", "windsurf", tags = listOf("ai", "ide")),
            prod("zed-editor", "Zed", Archetype.PROD_IDE, "zed.exe", "zed", tags = listOf("editor")),
            prod("neovim", "Neovim", Archetype.PROD_TERMINAL, "nvim.exe", "nvim", tags = listOf("vim", "editor")),
            prod("vim", "Vim", Archetype.PROD_TERMINAL, "vim.exe", "vim", tags = listOf("editor")),
            prod("emacs", "GNU Emacs", Archetype.PROD_IDE, "emacs.exe", "emacs", tags = listOf("editor")),
            prod("sublime-text", "Sublime Text", Archetype.PROD_IDE, "sublime_text.exe", "subl", tags = listOf("editor")),
            prod("notepad-pp", "Notepad++", Archetype.PROD_IDE, "notepad++.exe", tags = listOf("npp", "editor")),
            prod("windows-terminal", "Windows Terminal", Archetype.PROD_TERMINAL, "windowsterminal.exe", "wt.exe", tags = listOf("shell", "cli")),
            prod("iterm2", "iTerm2", Archetype.PROD_TERMINAL, "iterm2", tags = listOf("mac", "shell")),
            prod("warp-terminal", "Warp Terminal", Archetype.PROD_TERMINAL, "warp.exe", "warp", tags = listOf("shell")),
            prod("alacritty", "Alacritty", Archetype.PROD_TERMINAL, "alacritty.exe", "alacritty", tags = listOf("shell")),
            prod("kitty-terminal", "Kitty", Archetype.PROD_TERMINAL, "kitty", tags = listOf("shell")),
            prod("wezterm", "WezTerm", Archetype.PROD_TERMINAL, "wezterm-gui.exe", "wezterm", tags = listOf("shell")),
            prod("gitkraken", "GitKraken", Archetype.PROD_IDE, "gitkraken.exe", tags = listOf("git")),
            prod("fork-git", "Fork", Archetype.PROD_IDE, "fork.exe", tags = listOf("git")),
            prod("sourcetree", "Sourcetree", Archetype.PROD_IDE, "sourcetree.exe", tags = listOf("git")),
            prod("postman", "Postman", Archetype.PROD_IDE, "postman.exe", tags = listOf("api", "http")),
            prod("insomnia", "Insomnia", Archetype.PROD_IDE, "insomnia.exe", tags = listOf("api", "http")),
            prod("docker-desktop", "Docker Desktop", Archetype.PROD_TERMINAL, "docker desktop.exe", tags = listOf("containers")),
            prod("datagrip", "DataGrip", Archetype.PROD_IDE, "datagrip64.exe", tags = listOf("sql", "db")),
            prod("dbeaver", "DBeaver", Archetype.PROD_IDE, "dbeaver.exe", tags = listOf("sql", "db")),
            prod("tableplus", "TablePlus", Archetype.PROD_IDE, "tableplus.exe", tags = listOf("sql", "db")),
            prod("figma", "Figma", Archetype.PROD_CREATIVE, "figma.exe", "figma", tags = listOf("ui", "design")),
            prod("sketch", "Sketch", Archetype.PROD_CREATIVE, "sketch", tags = listOf("ui", "design")),
            prod("photoshop", "Adobe Photoshop", Archetype.PROD_CREATIVE, "photoshop.exe", tags = listOf("ps", "adobe")),
            prod("illustrator", "Adobe Illustrator", Archetype.PROD_CREATIVE, "illustrator.exe", tags = listOf("ai", "adobe")),
            prod("indesign", "Adobe InDesign", Archetype.PROD_CREATIVE, "indesign.exe", tags = listOf("id", "adobe")),
            prod("adobe-xd", "Adobe XD", Archetype.PROD_CREATIVE, "xd.exe", tags = listOf("xd", "adobe")),
            prod("lightroom", "Adobe Lightroom", Archetype.PROD_CREATIVE, "lightroom.exe", tags = listOf("lr", "photo")),
            prod("affinity-photo", "Affinity Photo", Archetype.PROD_CREATIVE, "photo.exe", tags = listOf("photo")),
            prod("affinity-designer", "Affinity Designer", Archetype.PROD_CREATIVE, "designer.exe", tags = listOf("vector")),
            prod("gimp", "GIMP", Archetype.PROD_CREATIVE, "gimp-2.10.exe", "gimp", tags = listOf("image")),
            prod("krita", "Krita", Archetype.PROD_CREATIVE, "krita.exe", "krita", tags = listOf("draw", "paint")),
            prod("inkscape", "Inkscape", Archetype.PROD_CREATIVE, "inkscape.exe", "inkscape", tags = listOf("vector", "svg")),
            prod("blender", "Blender", Archetype.PROD_CREATIVE, "blender.exe", "blender", tags = listOf("3d", "modeling")),
            prod("cinema-4d", "Cinema 4D", Archetype.PROD_CREATIVE, "cinema 4d.exe", tags = listOf("c4d", "3d")),
            prod("maya", "Autodesk Maya", Archetype.PROD_CREATIVE, "maya.exe", tags = listOf("3d")),
            prod("3ds-max", "Autodesk 3ds Max", Archetype.PROD_CREATIVE, "3dsmax.exe", tags = listOf("3d")),
            prod("houdini", "SideFX Houdini", Archetype.PROD_CREATIVE, "houdinifx.exe", tags = listOf("3d", "vfx")),
            prod("zbrush", "ZBrush", Archetype.PROD_CREATIVE, "zbrush.exe", tags = listOf("sculpt", "3d")),
            prod("unreal-editor", "Unreal Engine Editor", Archetype.PROD_CREATIVE, "unrealeditor.exe", tags = listOf("ue5", "gamedev")),
            prod("unity-editor", "Unity Editor", Archetype.PROD_CREATIVE, "unity.exe", tags = listOf("gamedev")),
            prod("godot-engine", "Godot Engine", Archetype.PROD_CREATIVE, "godot.exe", "godot", tags = listOf("gamedev")),
            prod("autocad", "AutoCAD", Archetype.PROD_CREATIVE, "acad.exe", tags = listOf("cad")),
            prod("solidworks", "SOLIDWORKS", Archetype.PROD_CREATIVE, "sldworks.exe", tags = listOf("cad")),
            prod("fusion-360", "Autodesk Fusion", Archetype.PROD_CREATIVE, "fusion360.exe", tags = listOf("cad")),
            prod("freecad", "FreeCAD", Archetype.PROD_CREATIVE, "freecad.exe", tags = listOf("cad")),
            prod("kicad", "KiCad", Archetype.PROD_CREATIVE, "kicad.exe", tags = listOf("pcb", "eda")),
            prod("ms-word", "Microsoft Word", Archetype.PROD_OFFICE, "winword.exe", tags = listOf("office", "doc")),
            prod("ms-excel", "Microsoft Excel", Archetype.PROD_OFFICE, "excel.exe", tags = listOf("office", "spreadsheet")),
            prod("ms-powerpoint", "Microsoft PowerPoint", Archetype.PROD_OFFICE, "powerpnt.exe", tags = listOf("office", "slides")),
            prod("ms-onenote", "Microsoft OneNote", Archetype.PROD_OFFICE, "onenote.exe", tags = listOf("notes")),
            prod("notion", "Notion", Archetype.PROD_OFFICE, "notion.exe", tags = listOf("notes", "docs")),
            prod("obsidian", "Obsidian", Archetype.PROD_OFFICE, "obsidian.exe", tags = listOf("markdown", "notes")),
            prod("logseq", "Logseq", Archetype.PROD_OFFICE, "logseq.exe", tags = listOf("notes")),
            prod("slack", "Slack", Archetype.PROD_OFFICE, "slack.exe", tags = listOf("chat")),
            prod("ms-teams", "Microsoft Teams", Archetype.PROD_OFFICE, "ms-teams.exe", "teams.exe", tags = listOf("chat", "meeting")),
            prod("zoom", "Zoom Workplace", Archetype.PROD_OFFICE, "zoom.exe", tags = listOf("meeting")),
            prod("chrome", "Google Chrome", Archetype.PROD_OFFICE, "chrome.exe", "google chrome", tags = listOf("browser", "web")),
            prod("firefox", "Mozilla Firefox", Archetype.PROD_OFFICE, "firefox.exe", "firefox", tags = listOf("browser", "web")),
            prod("arc-browser", "Arc Browser", Archetype.PROD_OFFICE, "arc.exe", "arc", tags = listOf("browser", "web")),
            prod("ms-edge", "Microsoft Edge", Archetype.PROD_OFFICE, "msedge.exe", tags = listOf("browser", "web")),
            prod("brave-browser", "Brave Browser", Archetype.PROD_OFFICE, "brave.exe", tags = listOf("browser", "web")),

            // === MEDIA (48) ===
            media("vlc", "VLC Media Player", Archetype.MEDIA_PLAYER, "vlc.exe", "vlc", tags = listOf("video", "player")),
            media("mpv", "mpv Player", Archetype.MEDIA_PLAYER, "mpv.exe", "mpv", tags = listOf("video", "player")),
            media("potplayer", "PotPlayer", Archetype.MEDIA_PLAYER, "potplayermini64.exe", tags = listOf("video", "player")),
            media("iina", "IINA", Archetype.MEDIA_PLAYER, "iina", tags = listOf("mac", "video")),
            media("mpc-hc", "MPC-HC", Archetype.MEDIA_PLAYER, "mpc-hc64.exe", tags = listOf("video", "player")),
            media("kodi", "Kodi", Archetype.MEDIA_PLAYER, "kodi.exe", tags = listOf("htpc", "media")),
            media("plex-htpc", "Plex HTPC", Archetype.MEDIA_PLAYER, "plex htpc.exe", "plex.exe", tags = listOf("htpc", "media")),
            media("jellyfin", "Jellyfin Media Player", Archetype.MEDIA_PLAYER, "jellyfinmediaplayer.exe", tags = listOf("htpc", "media")),
            media("youtube-web", "YouTube", Archetype.MEDIA_PLAYER, "youtube", tags = listOf("yt", "video", "stream")),
            media("netflix-app", "Netflix", Archetype.MEDIA_PLAYER, "netflix.exe", tags = listOf("video", "stream")),
            media("bilibili", "Bilibili", Archetype.MEDIA_PLAYER, "bilibili.exe", tags = listOf("bili", "video")),
            media("twitch", "Twitch", Archetype.MEDIA_PLAYER, "twitch.exe", tags = listOf("live", "stream")),
            media("disney-plus", "Disney+", Archetype.MEDIA_PLAYER, "disneyplus.exe", tags = listOf("video", "stream")),
            media("prime-video", "Prime Video", Archetype.MEDIA_PLAYER, "primevideo.exe", tags = listOf("video", "stream")),
            media("spotify", "Spotify", Archetype.MEDIA_PLAYER, "spotify.exe", "spotify", tags = listOf("music", "audio")),
            media("apple-music", "Apple Music", Archetype.MEDIA_PLAYER, "applemusic.exe", "music", tags = listOf("music", "audio")),
            media("foobar2000", "foobar2000", Archetype.MEDIA_PLAYER, "foobar2000.exe", tags = listOf("music", "audio")),
            media("musicbee", "MusicBee", Archetype.MEDIA_PLAYER, "musicbee.exe", tags = listOf("music", "audio")),
            media("winamp", "Winamp", Archetype.MEDIA_PLAYER, "winamp.exe", tags = listOf("music", "audio")),
            media("audacity", "Audacity", Archetype.MEDIA_EDITOR, "audacity.exe", tags = listOf("audio", "daw")),
            media("ableton-live", "Ableton Live", Archetype.MEDIA_EDITOR, "ableton live 12 suite.exe", "ableton live.exe", tags = listOf("daw", "music")),
            media("fl-studio", "FL Studio", Archetype.MEDIA_EDITOR, "fl64.exe", tags = listOf("daw", "music")),
            media("logic-pro", "Logic Pro", Archetype.MEDIA_EDITOR, "logic pro", tags = listOf("daw", "mac")),
            media("cubase", "Steinberg Cubase", Archetype.MEDIA_EDITOR, "cubase13.exe", tags = listOf("daw", "music")),
            media("reaper", "REAPER", Archetype.MEDIA_EDITOR, "reaper.exe", tags = listOf("daw", "audio")),
            media("studio-one", "PreSonus Studio One", Archetype.MEDIA_EDITOR, "studio one.exe", tags = listOf("daw")),
            media("pro-tools", "Avid Pro Tools", Archetype.MEDIA_EDITOR, "protools.exe", tags = listOf("daw")),
            media("bitwig-studio", "Bitwig Studio", Archetype.MEDIA_EDITOR, "bitwig studio.exe", tags = listOf("daw")),
            media("premiere-pro", "Adobe Premiere Pro", Archetype.MEDIA_EDITOR, "adobe premiere pro.exe", tags = listOf("pr", "video", "nle")),
            media("davinci-resolve", "DaVinci Resolve", Archetype.MEDIA_EDITOR, "resolve.exe", tags = listOf("video", "color", "nle")),
            media("final-cut-pro", "Final Cut Pro", Archetype.MEDIA_EDITOR, "final cut pro", tags = listOf("fcp", "mac", "nle")),
            media("after-effects", "Adobe After Effects", Archetype.MEDIA_EDITOR, "afterfx.exe", tags = listOf("ae", "vfx")),
            media("capcut", "CapCut", Archetype.MEDIA_EDITOR, "capcut.exe", tags = listOf("video", "edit")),
            media("jianying", "Jianying Pro", Archetype.MEDIA_EDITOR, "jianyingpro.exe", tags = listOf("capcut", "video")),
            media("vegas-pro", "VEGAS Pro", Archetype.MEDIA_EDITOR, "vegas210.exe", tags = listOf("video", "nle")),
            media("obs-studio", "OBS Studio", Archetype.MEDIA_EDITOR, "obs64.exe", "obs", tags = listOf("stream", "record")),
            media("streamlabs", "Streamlabs Desktop", Archetype.MEDIA_EDITOR, "streamlabs obs.exe", tags = listOf("stream")),
            media("xsplit", "XSplit Broadcaster", Archetype.MEDIA_EDITOR, "xsplit.core.exe", tags = listOf("stream")),
            media("handbrake", "HandBrake", Archetype.MEDIA_EDITOR, "handbrake.exe", tags = listOf("transcode")),
            media("shutter-encoder", "Shutter Encoder", Archetype.MEDIA_EDITOR, "shutter encoder.exe", tags = listOf("transcode")),
            media("aseprite", "Aseprite", Archetype.MEDIA_EDITOR, "aseprite.exe", tags = listOf("pixel", "sprite")),
            media("live2d-cubism", "Live2D Cubism", Archetype.MEDIA_EDITOR, "cubismeditor5.exe", tags = listOf("vtuber")),
            media("spine-2d", "Spine 2D", Archetype.MEDIA_EDITOR, "spine.exe", tags = listOf("animation")),
            media("wallpaper-engine", "Wallpaper Engine", Archetype.MEDIA_PLAYER, "wallpaper64.exe", tags = listOf("wallpaper")),
            media("pureref", "PureRef", Archetype.MEDIA_PLAYER, "pureref.exe", tags = listOf("reference")),
            media("qq-music", "QQ Music", Archetype.MEDIA_PLAYER, "qqmusic.exe", tags = listOf("music")),
            media("netease-cloud-music", "NetEase Cloud Music", Archetype.MEDIA_PLAYER, "cloudmusic.exe", tags = listOf("music")),
            media("kdenlive", "Kdenlive", Archetype.MEDIA_EDITOR, "kdenlive.exe", "kdenlive", tags = listOf("video", "nle")),
        )
    }

    private val profiles: List<KeymapProfile> by lazy {
        entries.map { entry ->
            KeymapProfile(
                id = "preset-${entry.category.wireValue}-${entry.slug}",
                name = entry.name,
                category = entry.category,
                controls = archetypeControls.getValue(entry.archetype),
                processNames = entry.processes.map { it.lowercase(Locale.ROOT) },
                tags = entry.tags,
                opacity = KeymapProfile.DEFAULT_PROFILE_OPACITY,
                builtIn = true,
                description = "${entry.category.displayLabel} preset for ${entry.name}",
                schemaVersion = KeymapProfile.CURRENT_SCHEMA_VERSION,
            )
        }
    }

    private val profilesById: Map<String, KeymapProfile> by lazy {
        profiles.associateBy { it.id }
    }

    private val exactProcessIndex: Map<String, KeymapProfile> by lazy {
        buildMap {
            for (profile in profiles) {
                for (proc in profile.processNames) {
                    val key = normalizeProcessToken(proc)
                    if (key.isNotEmpty() && !containsKey(key)) {
                        put(key, profile)
                    }
                }
            }
        }
    }

    private val stemProcessIndex: Map<String, KeymapProfile> by lazy {
        buildMap {
            for (profile in profiles) {
                for (proc in profile.processNames) {
                    val stem = stripExecutableSuffix(normalizeProcessToken(proc))
                    if (stem.isNotEmpty() && !containsKey(stem)) {
                        put(stem, profile)
                    }
                }
            }
        }
    }

    fun all(): List<KeymapProfile> = profiles

    fun findById(id: String): KeymapProfile? = profilesById[id.trim()]

    fun byCategory(category: KeymapCategory): List<KeymapProfile> =
        profiles.filter { it.category == category }

    fun countByCategory(): Map<KeymapCategory, Int> =
        profiles.groupingBy { it.category }.eachCount()

    fun search(query: String, category: KeymapCategory? = null): List<KeymapProfile> {
        val base = if (category == null) profiles else byCategory(category)
        val tokens = query.trim().lowercase(Locale.ROOT)
            .split(Regex("\\s+"))
            .filter { it.isNotEmpty() }
        if (tokens.isEmpty()) return base
        return base.filter { profile ->
            val haystack = buildString {
                append(profile.name.lowercase(Locale.ROOT))
                append(' ')
                append(profile.id.lowercase(Locale.ROOT))
                append(' ')
                append(profile.category.displayLabel.lowercase(Locale.ROOT))
                for (tag in profile.tags) {
                    append(' ')
                    append(tag.lowercase(Locale.ROOT))
                }
                for (proc in profile.processNames) {
                    append(' ')
                    append(proc.lowercase(Locale.ROOT))
                }
            }
            tokens.all { token -> haystack.contains(token) }
        }
    }

    fun matchProcess(processName: String): KeymapProfile? {
        val normalized = normalizeProcessToken(processName)
        if (normalized.isEmpty()) return null
        exactProcessIndex[normalized]?.let { return it }
        val stem = stripExecutableSuffix(normalized)
        if (stem.isEmpty()) return null
        stemProcessIndex[stem]?.let { return it }
        return null
    }

    private fun normalizeProcessToken(raw: String): String {
        val trimmed = raw.trim().lowercase(Locale.ROOT)
        if (trimmed.isEmpty()) return ""
        val lastSlash = maxOf(trimmed.lastIndexOf('/'), trimmed.lastIndexOf('\\'))
        return if (lastSlash >= 0) trimmed.substring(lastSlash + 1) else trimmed
    }

    private fun stripExecutableSuffix(token: String): String = when {
        token.endsWith(".exe") -> token.removeSuffix(".exe")
        token.endsWith(".app") -> token.removeSuffix(".app")
        token.endsWith(".bin") -> token.removeSuffix(".bin")
        else -> token
    }
}
