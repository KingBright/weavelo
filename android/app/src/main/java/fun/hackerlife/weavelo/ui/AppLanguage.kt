package `fun`.hackerlife.weavelo.ui

import android.content.Context
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.runtime.staticCompositionLocalOf
import java.util.Locale

internal class LanguageController(context: Context) {
    private val preferences = context.applicationContext.getSharedPreferences("language", Context.MODE_PRIVATE)
    var language by mutableStateOf(LanguageCatalog.resolvePreference(preferences.getString("selection", null), Locale.getDefault().language))
        private set

    fun select(value: AppLanguage) {
        preferences.edit().putString("selection", if (value == AppLanguage.CHINESE) "zh" else "en").apply()
        language = value
    }
}

internal val LocalLanguage = staticCompositionLocalOf<LanguageController?> { null }

@Composable
internal fun AppLanguageProvider(context: Context, content: @Composable () -> Unit) {
    val controller = remember { LanguageController(context) }
    CompositionLocalProvider(LocalLanguage provides controller, content = content)
}

@Composable
internal fun tr(key: String): String = LanguageCatalog.text(LocalLanguage.current?.language ?: LanguageCatalog.resolvePreference(null, Locale.getDefault().language), key)

/** Self-named options remain understandable in either selected language. */
@Composable
internal fun LanguageSelector() {
    val controller = LocalLanguage.current ?: return
    androidx.compose.foundation.layout.Row {
        androidx.compose.material3.TextButton(onClick = { controller.select(AppLanguage.CHINESE) }) {
            androidx.compose.material3.Text(if (controller.language == AppLanguage.CHINESE) "中文 ✓" else "中文")
        }
        androidx.compose.material3.TextButton(onClick = { controller.select(AppLanguage.ENGLISH) }) {
            androidx.compose.material3.Text(if (controller.language == AppLanguage.ENGLISH) "English ✓" else "English")
        }
    }
}

@Composable
internal fun trFormat(key: String, vararg arguments: Any): String =
    LanguageCatalog.format(LocalLanguage.current?.language ?: LanguageCatalog.resolvePreference(null, Locale.getDefault().language), key, *arguments)

@Composable
internal fun appText(value: `fun`.hackerlife.weavelo.AppText): String {
    val language = LocalLanguage.current?.language ?: LanguageCatalog.resolvePreference(null, Locale.getDefault().language)
    return LanguageCatalog.render(language, value)
}

/** Notification text uses the same saved choice without creating an Activity. */
internal fun android.content.Context.notificationText(key: String): String {
    val saved = applicationContext.getSharedPreferences("language", android.content.Context.MODE_PRIVATE).getString("selection", null)
    return LanguageCatalog.text(LanguageCatalog.resolvePreference(saved, Locale.getDefault().language), key)
}

/** Refresh an already-running foreground notification without restarting its owner. */
internal class LanguageNotificationObserver(context: Context, private val refresh: () -> Unit) : java.io.Closeable {
    private val preferences = context.applicationContext.getSharedPreferences("language", Context.MODE_PRIVATE)
    private val listener = android.content.SharedPreferences.OnSharedPreferenceChangeListener { _, key ->
        if (key == "selection") refresh()
    }
    init { preferences.registerOnSharedPreferenceChangeListener(listener) }
    override fun close() = preferences.unregisterOnSharedPreferenceChangeListener(listener)
}

internal fun cameraNameText(facing: String): `fun`.hackerlife.weavelo.AppText = `fun`.hackerlife.weavelo.AppText(when (facing) {
    "Front" -> "Front camera"
    "Back" -> "Back camera"
    "External" -> "External camera"
    else -> "Device camera"
})
