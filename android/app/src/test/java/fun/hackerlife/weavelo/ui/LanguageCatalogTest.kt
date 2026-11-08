package `fun`.hackerlife.weavelo.ui

import org.junit.Assert.*
import org.junit.Test

class LanguageCatalogTest {
    @Test fun preferenceAndSystemLanguage() {
        assertEquals(AppLanguage.CHINESE, LanguageCatalog.resolvePreference(null, "zh"))
        assertEquals(AppLanguage.ENGLISH, LanguageCatalog.resolvePreference(null, "fr"))
        assertEquals(AppLanguage.ENGLISH, LanguageCatalog.resolvePreference("en", "zh"))
        assertEquals(AppLanguage.CHINESE, LanguageCatalog.resolvePreference("zh", "en"))
    }

    @Test fun placeholdersPreserveUserData() {
        val name = "report %s 中文.txt"
        assertEquals("获取 $name（42 字节）", LanguageCatalog.format(AppLanguage.CHINESE, "Get %s (%d bytes)", name, 42L))
        assertEquals("Get $name (42 bytes)", LanguageCatalog.format(AppLanguage.ENGLISH, "Get %s (%d bytes)", name, 42L))
    }

    @Test fun catalogHasTranslationsAndMatchingFormatSpecifiers() {
        val placeholders = Regex("%(?:[0-9]+\\$)?[sd]")
        assertTrue(LanguageCatalog.chinese.isNotEmpty())
        LanguageCatalog.chinese.forEach { (english, chinese) ->
            assertTrue("Empty translation for $english", chinese.isNotBlank())
            assertEquals("Mismatched placeholders for $english", placeholders.findAll(english).map { it.value }.toList(), placeholders.findAll(chinese).map { it.value }.toList())
        }
    }
    @Test fun markedKeysExistAndNestedLabelsPreserveNames() {
        val source = java.io.File("src/main/java")
        assertTrue("Android source tree unavailable", source.isDirectory)
        val marker = Regex("""(?:\btr|trFormat|AppText|notificationText|appError)\(\"((?:\\.|[^\"\\])*)\"""")
        val missing = mutableListOf<String>()
        source.walkTopDown().filter { it.isFile && it.extension == "kt" && it.name != "LanguageCatalog.kt" }.forEach { file ->
            marker.findAll(file.readText()).forEach { match ->
                val key = match.groupValues[1].replace("\\n", "\n")
                if (key !in LanguageCatalog.chinese) missing += "${file.name}: $key"
            }
        }
        assertTrue("Missing Chinese keys: $missing", missing.isEmpty())
        val value = `fun`.hackerlife.weavelo.AppText("Sharing %s", listOf(`fun`.hackerlife.weavelo.AppText("Front camera")))
        assertEquals("正在共享 前置摄像头", LanguageCatalog.render(AppLanguage.CHINESE, value))
        assertEquals("Sharing Front camera", LanguageCatalog.render(AppLanguage.ENGLISH, value))
    }

    @Test fun onlyTaggedLocalErrorsAreTranslated() {
        val remoteText = "Camera permission was revoked"
        assertNull(`fun`.hackerlife.weavelo.appTextOf(IllegalStateException(remoteText)))
        val local = `fun`.hackerlife.weavelo.AppText("Camera permission was revoked")
        assertEquals(local, `fun`.hackerlife.weavelo.appTextOf(RuntimeException(`fun`.hackerlife.weavelo.AppTextException(local))))
        assertEquals("详情：$remoteText", LanguageCatalog.format(AppLanguage.CHINESE, "Details: %s", remoteText))
    }
    @Test fun pairingResultsMatchWholeMessagesAndKeepNetworkNameOpaque() {
        val name = "研发. Discovering members... %s"
        val result = "Joined network $name. Discovering members..."
        val text = `fun`.hackerlife.weavelo.pairingResultText(result, name)!!
        assertEquals("已加入网络 $name，正在发现成员…", LanguageCatalog.render(AppLanguage.CHINESE, text))
        assertNull(`fun`.hackerlife.weavelo.pairingResultText("$result extra", name))
    }
}
