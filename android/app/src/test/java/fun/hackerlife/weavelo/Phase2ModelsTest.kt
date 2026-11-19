package `fun`.hackerlife.weavelo

import android.content.SharedPreferences
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class Phase2ModelsTest {

    @Test
    fun colorProfileDefaultsAndJsonRoundtrip() {
        val defaults = ColorProfilePreference()
        assertEquals(ChromaMode.YUV420, defaults.chroma)
        assertEquals(HdrMode.SDR, defaults.hdr)

        for (chroma in ChromaMode.entries) {
            for (hdr in HdrMode.entries) {
                val original = ColorProfilePreference(chroma = chroma, hdr = hdr)
                val restored = ColorProfilePreference.fromJson(original.toJson())
                assertEquals(original, restored)
            }
        }
    }

    @Test
    fun colorProfileFallsBackToDefaultsOnMalformedOrUnknownJson() {
        assertEquals(ColorProfilePreference(), ColorProfilePreference.fromJson(null))
        assertEquals(ColorProfilePreference(), ColorProfilePreference.fromJson(""))
        assertEquals(ColorProfilePreference(), ColorProfilePreference.fromJson("{not-json"))
        assertEquals(
            ColorProfilePreference(),
            ColorProfilePreference.fromJson("""{"chroma":"yuv422","hdr":"dolby"}"""),
        )
        assertEquals(
            ColorProfilePreference(ChromaMode.YUV444, HdrMode.HDR10),
            ColorProfilePreference.fromJson("""{"chroma":"YUV444","hdr":"HDR10"}"""),
        )
    }

    @Test
    fun colorProfileSharedPreferencesPersistence() {
        val prefs = InMemorySharedPreferences()
        assertEquals(ColorProfilePreference(), ColorModePreferences.load(prefs))

        val saved = ColorProfilePreference(chroma = ChromaMode.YUV444, hdr = HdrMode.HDR10)
        ColorModePreferences.save(prefs, saved)
        assertEquals(saved, ColorModePreferences.load(prefs))
    }

    @Test
    fun portForwardRuleValidationAcceptsBoundsAndRejectsInvalidInputs() {
        assertTrue(
            PortForwardRule(
                id = "min",
                name = "Min",
                bindPort = 1,
                targetHost = "127.0.0.1",
                targetPort = 1,
            ).isValid(),
        )
        assertTrue(
            PortForwardRule(
                id = "max",
                name = "Max",
                bindPort = 65535,
                targetHost = "localhost",
                targetPort = 65535,
            ).isValid(),
        )

        val invalidRules = listOf(
            PortForwardRule(id = " ", name = "Blank ID", bindPort = 8080, targetHost = "127.0.0.1", targetPort = 80),
            PortForwardRule(id = "p0", name = "Zero bind", bindPort = 0, targetHost = "127.0.0.1", targetPort = 80),
            PortForwardRule(id = "pneg", name = "Neg bind", bindPort = -1, targetHost = "127.0.0.1", targetPort = 80),
            PortForwardRule(id = "phi", name = "High bind", bindPort = 65536, targetHost = "127.0.0.1", targetPort = 80),
            PortForwardRule(id = "t0", name = "Zero target", bindPort = 8080, targetHost = "127.0.0.1", targetPort = 0),
            PortForwardRule(id = "thi", name = "High target", bindPort = 8080, targetHost = "127.0.0.1", targetPort = 65536),
            PortForwardRule(id = "host", name = "Blank host", bindPort = 8080, targetHost = "   ", targetPort = 80),
        )
        for (rule in invalidRules) {
            assertFalse("Expected invalid: $rule", rule.isValid())
            assertNotNull("Expected validation message: $rule", rule.validate())
        }
    }

    @Test
    fun portForwardStoreRejectsDuplicateBindPortPerProtocolButAllowsDifferentProtocol() {
        val prefs = InMemorySharedPreferences()
        val tcpRule = PortForwardRule(
            id = "rule-tcp-8080",
            name = "Web TCP",
            protocol = TunnelProtocol.TCP,
            direction = TunnelDirection.LOCAL_TO_REMOTE,
            bindPort = 8080,
            targetHost = "127.0.0.1",
            targetPort = 3000,
        )
        val first = PortForwardStore.add(prefs, tcpRule)
        assertTrue(first.isSuccess)
        assertEquals(listOf(tcpRule), first.getOrThrow())

        val duplicateTcp = PortForwardRule(
            id = "rule-tcp-8080-dup",
            name = "Web TCP Duplicate",
            protocol = TunnelProtocol.TCP,
            direction = TunnelDirection.REMOTE_TO_LOCAL,
            bindPort = 8080,
            targetHost = "10.0.0.2",
            targetPort = 4000,
        )
        val duplicateResult = PortForwardStore.add(prefs, duplicateTcp)
        assertTrue(duplicateResult.isFailure)
        assertEquals(1, PortForwardStore.list(prefs).size)

        val udpSamePort = PortForwardRule(
            id = "rule-udp-8080",
            name = "Game UDP",
            protocol = TunnelProtocol.UDP,
            direction = TunnelDirection.LOCAL_TO_REMOTE,
            bindPort = 8080,
            targetHost = "127.0.0.1",
            targetPort = 9000,
        )
        val udpResult = PortForwardStore.add(prefs, udpSamePort)
        assertTrue(udpResult.isSuccess)
        assertEquals(listOf(tcpRule, udpSamePort), udpResult.getOrThrow())
    }

    @Test
    fun portForwardStoreJsonRoundtripAndCrudOperations() {
        val ruleA = PortForwardRule(
            id = "r1",
            name = "SSH",
            protocol = TunnelProtocol.TCP,
            direction = TunnelDirection.LOCAL_TO_REMOTE,
            bindPort = 2222,
            targetHost = "127.0.0.1",
            targetPort = 22,
            enabled = true,
        )
        val ruleB = PortForwardRule(
            id = "r2",
            name = "DNS",
            protocol = TunnelProtocol.UDP,
            direction = TunnelDirection.REMOTE_TO_LOCAL,
            bindPort = 5353,
            targetHost = "192.168.1.10",
            targetPort = 53,
            enabled = false,
        )

        val encoded = PortForwardStore.encodeRules(listOf(ruleA, ruleB))
        val decoded = PortForwardStore.decodeRules(encoded)
        assertEquals(listOf(ruleA, ruleB), decoded)
        assertEquals(emptyList<PortForwardRule>(), PortForwardStore.decodeRules("invalid-json"))
        assertNull(PortForwardRule.fromJson("""{"id":"bad","protocol":"tcp","direction":"local_to_remote","bind_port":0,"target_host":"127.0.0.1","target_port":80}"""))

        val prefs = InMemorySharedPreferences()
        assertTrue(PortForwardStore.add(prefs, ruleA).isSuccess)
        assertTrue(PortForwardStore.add(prefs, ruleB).isSuccess)
        assertEquals(listOf(ruleA, ruleB), PortForwardStore.list(prefs))

        val afterToggle = PortForwardStore.toggle(prefs, "r1")
        assertFalse(afterToggle.first { it.id == "r1" }.enabled)
        assertFalse(PortForwardStore.list(prefs).first { it.id == "r1" }.enabled)

        val afterRemove = PortForwardStore.remove(prefs, "r1")
        assertEquals(listOf(ruleB), afterRemove)
        assertEquals(listOf(ruleB), PortForwardStore.list(prefs))
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
