package `fun`.hackerlife.weavelo

import android.content.SharedPreferences
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class Phase3VirtualDisplayTest {

    @Test
    fun fromDeviceMetricsRespectsOrientationEvenAlignmentAndClamping() {
        val landscape = VirtualDisplayProfile.fromDeviceMetrics(
            widthPx = 1080,
            heightPx = 2400,
            densityDpi = 420,
            refreshHz = 120,
            landscape = true,
        )
        assertTrue(landscape.isValid())
        assertEquals(2400, landscape.width)
        assertEquals(1080, landscape.height)
        assertEquals(120, landscape.refreshHz)
        assertEquals(2.75f, landscape.dpiScale, 0.0001f)
        assertEquals(VirtualDisplayPreset.MATCH_DEVICE, landscape.preset)

        val portrait = VirtualDisplayProfile.fromDeviceMetrics(
            widthPx = 2400,
            heightPx = 1080,
            densityDpi = 320,
            refreshHz = 90,
            landscape = false,
        )
        assertTrue(portrait.isValid())
        assertEquals(1080, portrait.width)
        assertEquals(2400, portrait.height)
        assertEquals(90, portrait.refreshHz)
        assertEquals(2.0f, portrait.dpiScale, 0.0001f)
        assertEquals(VirtualDisplayPreset.MATCH_DEVICE, portrait.preset)

        val oddDimensions = VirtualDisplayProfile.fromDeviceMetrics(
            widthPx = 1081,
            heightPx = 2341,
            densityDpi = 160,
            refreshHz = 60,
            landscape = true,
        )
        assertTrue(oddDimensions.isValid())
        assertEquals(2340, oddDimensions.width)
        assertEquals(1080, oddDimensions.height)
        assertEquals(0, oddDimensions.width % 2)
        assertEquals(0, oddDimensions.height % 2)
        assertEquals(1.0f, oddDimensions.dpiScale, 0.0001f)

        val tooSmall = VirtualDisplayProfile.fromDeviceMetrics(
            widthPx = 100,
            heightPx = 150,
            densityDpi = 40,
            refreshHz = 10,
            landscape = true,
        )
        assertTrue(tooSmall.isValid())
        assertEquals(VirtualDisplayProfile.MIN_WIDTH, tooSmall.width)
        assertEquals(VirtualDisplayProfile.MIN_HEIGHT, tooSmall.height)
        assertEquals(VirtualDisplayProfile.MIN_REFRESH_HZ, tooSmall.refreshHz)
        assertEquals(VirtualDisplayProfile.MIN_DPI_SCALE, tooSmall.dpiScale, 0.0001f)

        val tooLarge = VirtualDisplayProfile.fromDeviceMetrics(
            widthPx = 9999,
            heightPx = 8889,
            densityDpi = 960,
            refreshHz = 360,
            landscape = true,
        )
        assertTrue(tooLarge.isValid())
        assertEquals(VirtualDisplayProfile.MAX_WIDTH, tooLarge.width)
        assertEquals(VirtualDisplayProfile.MAX_HEIGHT, tooLarge.height)
        assertEquals(VirtualDisplayProfile.MAX_REFRESH_HZ, tooLarge.refreshHz)
        assertEquals(VirtualDisplayProfile.MAX_DPI_SCALE, tooLarge.dpiScale, 0.0001f)

        val zeroDensity = VirtualDisplayProfile.fromDeviceMetrics(
            widthPx = 1920,
            heightPx = 1080,
            densityDpi = 0,
            refreshHz = 60,
            landscape = true,
        )
        assertTrue(zeroDensity.isValid())
        assertEquals(1.0f, zeroDensity.dpiScale, 0.0001f)
    }

    @Test
    fun validationAcceptsBoundsAndRejectsOddOrOutOfRangeValues() {
        val minValid = VirtualDisplayProfile(
            id = "min",
            name = "Min",
            width = 320,
            height = 240,
            refreshHz = 24,
            dpiScale = 0.5f,
        )
        assertTrue(minValid.isValid())
        assertNull(minValid.validate())

        val maxValid = VirtualDisplayProfile(
            id = "max",
            name = "Max",
            width = 7680,
            height = 4320,
            refreshHz = 240,
            dpiScale = 4.0f,
        )
        assertTrue(maxValid.isValid())
        assertNull(maxValid.validate())

        val invalidProfiles = listOf(
            VirtualDisplayProfile(id = "  ", name = "Blank ID", width = 1920, height = 1080),
            VirtualDisplayProfile(id = "odd-w", name = "Odd W", width = 1921, height = 1080),
            VirtualDisplayProfile(id = "odd-h", name = "Odd H", width = 1920, height = 1081),
            VirtualDisplayProfile(id = "low-w", name = "Low W", width = 318, height = 1080),
            VirtualDisplayProfile(id = "hi-w", name = "High W", width = 7682, height = 1080),
            VirtualDisplayProfile(id = "low-h", name = "Low H", width = 1920, height = 238),
            VirtualDisplayProfile(id = "hi-h", name = "High H", width = 1920, height = 4322),
            VirtualDisplayProfile(id = "low-hz", name = "Low Hz", width = 1920, height = 1080, refreshHz = 23),
            VirtualDisplayProfile(id = "hi-hz", name = "High Hz", width = 1920, height = 1080, refreshHz = 241),
            VirtualDisplayProfile(id = "low-dpi", name = "Low DPI", width = 1920, height = 1080, dpiScale = 0.49f),
            VirtualDisplayProfile(id = "hi-dpi", name = "High DPI", width = 1920, height = 1080, dpiScale = 4.01f),
            VirtualDisplayProfile(id = "nan-dpi", name = "NaN DPI", width = 1920, height = 1080, dpiScale = Float.NaN),
            VirtualDisplayProfile(id = "inf-dpi", name = "Inf DPI", width = 1920, height = 1080, dpiScale = Float.POSITIVE_INFINITY),
        )
        for (profile in invalidProfiles) {
            assertFalse("Expected invalid: $profile", profile.isValid())
            assertNotNull("Expected validation error: $profile", profile.validate())
        }
    }

    @Test
    fun presetsAndPlacementsProduceValidProfilesAndParseWireValues() {
        assertEquals(6, VirtualDisplayPreset.entries.size)
        assertEquals(6, DisplayPlacement.entries.size)

        val expectedResolutions = mapOf(
            VirtualDisplayPreset.FHD_1080P to (1920 to 1080),
            VirtualDisplayPreset.QHD_1440P to (2560 to 1440),
            VirtualDisplayPreset.UHD_4K to (3840 to 2160),
            VirtualDisplayPreset.ULTRAWIDE_21_9 to (3440 to 1440),
            VirtualDisplayPreset.SUPER_ULTRAWIDE_32_9 to (5120 to 1440),
            VirtualDisplayPreset.MATCH_DEVICE to (1920 to 1080),
        )
        for (preset in VirtualDisplayPreset.entries) {
            val profile = VirtualDisplayProfile.fromPreset(preset)
            assertTrue("Preset $preset should produce a valid profile", profile.isValid())
            assertEquals(expectedResolutions[preset], profile.width to profile.height)
            assertEquals(preset, VirtualDisplayPreset.fromWireValueOrNull(preset.wireValue))
            assertEquals(preset, VirtualDisplayPreset.fromWireValueOrNull(preset.name.uppercase()))
        }
        assertNull(VirtualDisplayPreset.fromWireValueOrNull("unknown_preset"))
        assertEquals(VirtualDisplayPreset.FHD_1080P, VirtualDisplayPreset.fromWireValue("unknown_preset"))

        for (placement in DisplayPlacement.entries) {
            assertEquals(placement, DisplayPlacement.fromWireValueOrNull(placement.wireValue))
            assertEquals(placement, DisplayPlacement.fromWireValueOrNull(placement.name.uppercase()))
        }
        assertNull(DisplayPlacement.fromWireValueOrNull("diagonal"))
        assertEquals(DisplayPlacement.RIGHT_OF, DisplayPlacement.fromWireValue("diagonal"))
    }

    @Test
    fun storeJsonRoundtripAndCrudOperations() {
        val profileA = VirtualDisplayProfile(
            id = "vd-1",
            name = "Desk 4K",
            width = 3840,
            height = 2160,
            refreshHz = 60,
            dpiScale = 2.0f,
            placement = DisplayPlacement.RIGHT_OF,
            preset = VirtualDisplayPreset.UHD_4K,
            persistAfterDisconnect = true,
        )
        val profileB = VirtualDisplayProfile.fromDeviceMetrics(
            widthPx = 1200,
            heightPx = 2000,
            densityDpi = 320,
            refreshHz = 120,
            landscape = true,
            name = "Tablet Sidecar",
            placement = DisplayPlacement.LEFT_OF,
            persistAfterDisconnect = false,
            id = "vd-2",
        )

        val encoded = VirtualDisplayStore.encodeProfiles(listOf(profileA, profileB))
        val decoded = VirtualDisplayStore.decodeProfiles(encoded)
        assertEquals(listOf(profileA, profileB), decoded)
        assertEquals(emptyList<VirtualDisplayProfile>(), VirtualDisplayStore.decodeProfiles("not-json"))
        assertNull(
            VirtualDisplayProfile.fromJson(
                """{"id":"bad","name":"Odd","width":1921,"height":1080,"refresh_hz":60,"dpi_scale":1.0,"placement":"right_of","preset":"fhd_1080p"}""",
            ),
        )

        val prefs = InMemorySharedPreferences()
        assertEquals(emptyList<VirtualDisplayProfile>(), VirtualDisplayStore.list(prefs))

        assertTrue(VirtualDisplayStore.add(prefs, profileA).isSuccess)
        assertTrue(VirtualDisplayStore.add(prefs, profileB).isSuccess)
        assertEquals(listOf(profileA, profileB), VirtualDisplayStore.list(prefs))

        // Duplicate ID is rejected
        val duplicate = profileA.copy(name = "Duplicate")
        assertTrue(VirtualDisplayStore.add(prefs, duplicate).isFailure)
        assertEquals(2, VirtualDisplayStore.list(prefs).size)

        // Invalid profile is rejected
        val invalid = profileA.copy(id = "vd-invalid", width = 1921)
        assertTrue(VirtualDisplayStore.add(prefs, invalid).isFailure)
        assertEquals(2, VirtualDisplayStore.list(prefs).size)

        // Remove profileA
        val afterRemove = VirtualDisplayStore.remove(prefs, "vd-1")
        assertEquals(listOf(profileB), afterRemove)
        assertEquals(listOf(profileB), VirtualDisplayStore.list(prefs))

        // saveAll replaces list
        VirtualDisplayStore.saveAll(prefs, listOf(profileA))
        assertEquals(listOf(profileA), VirtualDisplayStore.list(prefs))
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
