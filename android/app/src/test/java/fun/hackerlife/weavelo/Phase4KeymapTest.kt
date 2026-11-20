package `fun`.hackerlife.weavelo

import android.content.SharedPreferences
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class Phase4KeymapTest {

    @Test
    fun catalogProvidesAtLeast500UniqueValidPresetsAcrossCategories() {
        val all = KeymapCatalog.all()
        assertTrue("Expected at least 500 presets, got ${all.size}", all.size >= 500)

        val seenIds = HashSet<String>(all.size)
        for (profile in all) {
            assertTrue("Duplicate preset ID: ${profile.id}", seenIds.add(profile.id))
            assertEquals("Expected schemaVersion 1 for ${profile.id}", 1, profile.schemaVersion)
            assertTrue("Expected builtIn=true for ${profile.id}", profile.builtIn)
            assertNull("Preset ${profile.id} failed validation: ${profile.validate()}", profile.validate())
            assertTrue("Preset ${profile.id} must have controls", profile.controls.isNotEmpty())

            val seenControlIds = HashSet<String>(profile.controls.size)
            for (control in profile.controls) {
                assertTrue("Duplicate control ID ${control.id} in ${profile.id}", seenControlIds.add(control.id))
                assertTrue("Control x out of range in ${profile.id}/${control.id}", control.x in 0.0f..1.0f)
                assertTrue("Control y out of range in ${profile.id}/${control.id}", control.y in 0.0f..1.0f)
                assertTrue(
                    "Control size out of range in ${profile.id}/${control.id}",
                    control.size in KeymapControl.MIN_SIZE..KeymapControl.MAX_SIZE,
                )
                assertTrue(
                    "Control opacity out of range in ${profile.id}/${control.id}",
                    control.opacity in KeymapControl.MIN_OPACITY..KeymapControl.MAX_OPACITY,
                )
            }
        }

        val builtInCategories = listOf(
            KeymapCategory.FPS,
            KeymapCategory.MOBA,
            KeymapCategory.RPG,
            KeymapCategory.ACTION,
            KeymapCategory.RACING,
            KeymapCategory.SIMULATOR,
            KeymapCategory.PRODUCTIVITY,
            KeymapCategory.MEDIA,
        )
        for (category in builtInCategories) {
            val items = KeymapCatalog.byCategory(category)
            assertTrue("Expected non-empty catalog for $category", items.isNotEmpty())
            assertTrue("All items in $category must match category", items.all { it.category == category })
        }
    }

    @Test
    fun catalogMatchProcessAndSearchWorkAcrossFormats() {
        val cs2 = KeymapCatalog.matchProcess("cs2.exe")
        assertNotNull(cs2)
        assertEquals("Counter-Strike 2", cs2!!.name)
        assertEquals(KeymapCategory.FPS, cs2.category)

        // Case-insensitive and full path normalization
        val valorant = KeymapCatalog.matchProcess("""C:\Riot Games\VALORANT\live\VALORANT-Win64-Shipping.EXE""")
        assertNotNull(valorant)
        assertEquals("VALORANT", valorant!!.name)

        // Unix path & extensionless binary
        val vscode = KeymapCatalog.matchProcess("/usr/bin/code")
        assertNotNull(vscode)
        assertEquals("Visual Studio Code", vscode!!.name)
        assertEquals(KeymapCategory.PRODUCTIVITY, vscode.category)

        // Extensionless match against .exe catalog entry
        val genshin = KeymapCatalog.matchProcess("GenshinImpact")
        assertNotNull(genshin)
        assertEquals("Genshin Impact", genshin!!.name)

        assertNull(KeymapCatalog.matchProcess(""))
        assertNull(KeymapCatalog.matchProcess("   "))
        assertNull(KeymapCatalog.matchProcess("nonexistent_app_12345.exe"))

        // Search by title, tag, process, and category filter
        val searchLol = KeymapCatalog.search("lol", KeymapCategory.MOBA)
        assertTrue(searchLol.any { it.name == "League of Legends" })

        val searchVlc = KeymapCatalog.search("vlc.exe")
        assertTrue(searchVlc.any { it.name == "VLC Media Player" })

        val searchMultiToken = KeymapCatalog.search("black myth wukong")
        assertEquals(1, searchMultiToken.size)
        assertEquals("Black Myth: Wukong", searchMultiToken.first().name)

        assertEquals(KeymapCatalog.all().size, KeymapCatalog.search("").size)
        assertEquals(
            KeymapCatalog.byCategory(KeymapCategory.RACING).size,
            KeymapCatalog.search("   ", KeymapCategory.RACING).size,
        )
    }

    @Test
    fun validationRejectsInvalidControlsAndProfiles() {
        val validControl = KeymapControl(
            id = "jump",
            kind = VirtualControlKind.KEY_BUTTON,
            label = "Jump",
            x = 0.85f,
            y = 0.65f,
            keys = listOf("space"),
        )
        assertTrue(validControl.isValid())

        val invalidControls = listOf(
            validControl.copy(id = " "),
            validControl.copy(label = ""),
            validControl.copy(x = -0.01f),
            validControl.copy(x = 1.01f),
            validControl.copy(y = Float.NaN),
            validControl.copy(size = 0.01f),
            validControl.copy(size = 0.90f),
            validControl.copy(opacity = 0.05f),
            validControl.copy(opacity = 1.5f),
            validControl.copy(burstIntervalMs = 5),
            validControl.copy(sensitivity = 0.0f),
            validControl.copy(keys = emptyList()),
            KeymapControl("stick", VirtualControlKind.JOYSTICK_WASD, "Move", 0.2f, 0.7f, keys = listOf("w", "a")),
        )
        for (control in invalidControls) {
            assertFalse("Expected invalid control: $control", control.isValid())
            assertNotNull(control.validate())
        }

        val validProfile = KeymapProfile(
            id = "custom-1",
            name = "My Layout",
            category = KeymapCategory.CUSTOM,
            controls = listOf(validControl),
        )
        assertTrue(validProfile.isValid())

        val invalidProfiles = listOf(
            validProfile.copy(schemaVersion = 2),
            validProfile.copy(id = "  "),
            validProfile.copy(name = ""),
            validProfile.copy(opacity = 0.05f),
            validProfile.copy(controls = emptyList()),
            validProfile.copy(controls = listOf(validControl, validControl.copy(label = "Dup"))),
        )
        for (profile in invalidProfiles) {
            assertFalse("Expected invalid profile: $profile", profile.isValid())
            assertNotNull(profile.validate())
        }
    }

    @Test
    fun profileJsonRoundtripPreservesAllControlKindsAndStoreOperations() {
        val allKindsProfile = KeymapProfile(
            id = "custom-all-kinds",
            name = "All Control Kinds",
            category = KeymapCategory.CUSTOM,
            processNames = listOf("testgame.exe"),
            tags = listOf("custom", "test"),
            opacity = 0.65f,
            builtIn = false,
            description = "Covers every VirtualControlKind",
            controls = listOf(
                KeymapControl("c-joy", VirtualControlKind.JOYSTICK_WASD, "WASD", 0.15f, 0.75f, 0.22f, listOf("w", "a", "s", "d")),
                KeymapControl("c-look", VirtualControlKind.LOOK_PAD, "Look", 0.55f, 0.45f, 0.26f, emptyList(), opacity = 0.35f, sensitivity = 1.5f),
                KeymapControl("c-key", VirtualControlKind.KEY_BUTTON, "Jump", 0.85f, 0.55f, 0.09f, listOf("space"), toggle = true),
                KeymapControl("c-combo", VirtualControlKind.COMBO_BUTTON, "Save", 0.25f, 0.85f, 0.09f, listOf("ctrl", "s")),
                KeymapControl("c-burst", VirtualControlKind.MACRO_BURST, "Burst", 0.88f, 0.72f, 0.10f, listOf("mouse_left"), burstIntervalMs = 80),
                KeymapControl("c-scroll", VirtualControlKind.SCROLL_WHEEL, "Zoom", 0.92f, 0.30f, 0.11f, emptyList(), sensitivity = 1.25f),
                KeymapControl("c-mouse", VirtualControlKind.MOUSE_BUTTON, "ADS", 0.75f, 0.60f, 0.085f, listOf("mouse_right")),
            ),
        )

        val encodedSingle = allKindsProfile.toJson()
        val decodedSingle = KeymapProfile.fromJson(encodedSingle)
        assertEquals(allKindsProfile, decodedSingle)
        assertNull(KeymapProfile.fromJson("not-json"))
        assertNull(KeymapProfile.fromJson("""{"schema_version":99,"id":"x","name":"x","category":"fps","controls":[]}"""))

        val prefs = InMemorySharedPreferences()
        assertEquals(emptyList<KeymapProfile>(), KeymapStore.listCustom(prefs))
        assertNull(KeymapStore.getActiveProfileId(prefs))
        assertFalse(KeymapStore.isHudEnabled(prefs))

        // Upsert custom profile & activate it
        assertTrue(KeymapStore.upsertCustom(prefs, allKindsProfile).isSuccess)
        assertEquals(listOf(allKindsProfile), KeymapStore.listCustom(prefs))

        KeymapStore.setActiveProfileId(prefs, allKindsProfile.id)
        KeymapStore.setHudEnabled(prefs, true)
        assertEquals(allKindsProfile.id, KeymapStore.getActiveProfileId(prefs))
        assertEquals(allKindsProfile, KeymapStore.getActiveProfile(prefs))
        assertTrue(KeymapStore.isHudEnabled(prefs))

        // Update existing custom profile in-place
        val updated = allKindsProfile.copy(name = "All Control Kinds v2", opacity = 0.85f)
        assertTrue(KeymapStore.upsertCustom(prefs, updated).isSuccess)
        assertEquals(1, KeymapStore.listCustom(prefs).size)
        assertEquals("All Control Kinds v2", KeymapStore.getActiveProfile(prefs)?.name)

        // Custom override takes precedence over built-in catalog ID
        val builtInCs2 = KeymapCatalog.matchProcess("cs2.exe")!!
        val customCs2 = builtInCs2.copy(name = "CS2 Custom", opacity = 0.55f)
        assertTrue(KeymapStore.upsertCustom(prefs, customCs2).isSuccess)
        val resolvedCs2 = KeymapStore.resolveProfile(prefs, builtInCs2.id)
        assertNotNull(resolvedCs2)
        assertEquals("CS2 Custom", resolvedCs2!!.name)
        assertFalse(resolvedCs2.builtIn)

        // allProfiles deduplicates custom overrides
        val combined = KeymapStore.allProfiles(prefs)
        assertEquals(KeymapCatalog.all().size + 1, combined.size)

        // Removing active custom profile clears activeProfileId
        KeymapStore.removeCustom(prefs, allKindsProfile.id)
        assertNull(KeymapStore.getActiveProfileId(prefs))
        assertEquals(listOf(customCs2.copy(builtIn = false)), KeymapStore.listCustom(prefs))

        // Built-in preset can also be resolved directly as active
        val builtInValorant = KeymapCatalog.matchProcess("valorant.exe")!!
        KeymapStore.setActiveProfileId(prefs, builtInValorant.id)
        assertEquals(builtInValorant, KeymapStore.getActiveProfile(prefs))
    }

    private class InMemorySharedPreferences : SharedPreferences {
        private val data = LinkedHashMap<String, Any?>()

        override fun getAll(): MutableMap<String, *> = LinkedHashMap(data)
        override fun getString(key: String?, defValue: String?): String? = data[key] as? String ?: defValue
        @Suppress("UNCHECKED_CAST")
        override fun getStringSet(key: String?, defValues: MutableSet<String>?): MutableSet<String>? =
            (data[key] as? Set<String>)?.toMutableSet() ?: defValues
        override fun getInt(key: String?, defValue: Int): Int = data[key] as? Int ?: defValue
        override fun getLong(key: String?, defValue: Long): Long = data[key] as? Long ?: defValue
        override fun getFloat(key: String?, defValue: Float): Float = data[key] as? Float ?: defValue
        override fun getBoolean(key: String?, defValue: Boolean): Boolean = data[key] as? Boolean ?: defValue
        override fun contains(key: String?): Boolean = data.containsKey(key)
        override fun edit(): SharedPreferences.Editor = EditorImpl()
        override fun registerOnSharedPreferenceChangeListener(listener: SharedPreferences.OnSharedPreferenceChangeListener?) = Unit
        override fun unregisterOnSharedPreferenceChangeListener(listener: SharedPreferences.OnSharedPreferenceChangeListener?) = Unit

        private inner class EditorImpl : SharedPreferences.Editor {
            private val pending = LinkedHashMap<String, Any?>()
            private var clearAll = false

            override fun putString(key: String?, value: String?): SharedPreferences.Editor = apply {
                if (key != null) pending[key] = value
            }
            override fun putStringSet(key: String?, values: MutableSet<String>?): SharedPreferences.Editor = apply {
                if (key != null) pending[key] = values?.toSet()
            }
            override fun putInt(key: String?, value: Int): SharedPreferences.Editor = apply {
                if (key != null) pending[key] = value
            }
            override fun putLong(key: String?, value: Long): SharedPreferences.Editor = apply {
                if (key != null) pending[key] = value
            }
            override fun putFloat(key: String?, value: Float): SharedPreferences.Editor = apply {
                if (key != null) pending[key] = value
            }
            override fun putBoolean(key: String?, value: Boolean): SharedPreferences.Editor = apply {
                if (key != null) pending[key] = value
            }
            override fun remove(key: String?): SharedPreferences.Editor = apply {
                if (key != null) pending[key] = null
            }
            override fun clear(): SharedPreferences.Editor = apply { clearAll = true }
            override fun commit(): Boolean {
                apply()
                return true
            }
            override fun apply() {
                if (clearAll) data.clear()
                pending.forEach { (k, v) ->
                    if (v == null) data.remove(k) else data[k] = v
                }
            }
        }
    }
}
