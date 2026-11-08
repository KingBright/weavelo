package `fun`.hackerlife.weavelo.ui

import androidx.compose.material3.Text
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.test.ext.junit.runners.AndroidJUnit4
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
class LanguageSelectionTest {
    @get:Rule val compose = createComposeRule()

    @Test fun selectionRecomposesAndPersists() {
        val context = androidx.test.platform.app.InstrumentationRegistry.getInstrumentation().targetContext
        val preferences = context.getSharedPreferences("language", android.content.Context.MODE_PRIVATE)
        val old = preferences.getString("selection", null)
        try {
            var starts = 0
            preferences.edit().putString("selection", "en").commit()
            compose.setContent {
                AppLanguageProvider(LocalContext.current) {
                    LaunchedEffect(Unit) { starts++ }
                    LanguageSelector()
                    Text(tr("Settings"))
                }
            }
            compose.onNodeWithText("中文").performClick()
            compose.onNodeWithText("设置").assertExists()
            compose.runOnIdle { org.junit.Assert.assertEquals("zh", preferences.getString("selection", null)) }
            compose.onNodeWithText("English").performClick()
            compose.onNodeWithText("Settings").assertExists()
            compose.runOnIdle { org.junit.Assert.assertEquals("en", preferences.getString("selection", null)); org.junit.Assert.assertEquals(1, starts) }
        } finally {
            preferences.edit().apply { if (old == null) remove("selection") else putString("selection", old) }.commit()
        }
    }
    @Test fun notificationTextFollowsSelectionWithoutRestartingService() {
        val context = androidx.test.platform.app.InstrumentationRegistry.getInstrumentation().targetContext
        val preferences = context.getSharedPreferences("language", android.content.Context.MODE_PRIVATE)
        val old = preferences.getString("selection", null)
        preferences.edit().putString("selection", "en").commit()
        var refreshes = 0
        val observer = LanguageNotificationObserver(context) { refreshes++ }
        try {
            compose.runOnIdle {
                val controller = LanguageController(context)
                controller.select(AppLanguage.CHINESE)
                org.junit.Assert.assertEquals("屏幕共享", context.notificationText("Screen sharing"))
                controller.select(AppLanguage.ENGLISH)
                org.junit.Assert.assertEquals("Screen sharing", context.notificationText("Screen sharing"))
                org.junit.Assert.assertEquals(2, refreshes)
            }
        } finally {
            observer.close()
            preferences.edit().apply { if (old == null) remove("selection") else putString("selection", old) }.commit()
        }
    }
}
