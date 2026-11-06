package `fun`.hackerlife.weavelo.ui

import androidx.compose.material3.MaterialTheme
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.test.ext.junit.runners.AndroidJUnit4
import `fun`.hackerlife.weavelo.DeviceBadgePreset
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
class DeviceBadgeCardTest {
    @get:Rule
    val compose = createComposeRule()

    @Test
    fun peerCardRendersTheAdvertisedPhoneBadge() {
        compose.setContent {
            MaterialTheme {
                HostCard(
                    name = "nubia NX741J",
                    tag = "LAN",
                    subtext = "Available",
                    deviceBadge = DeviceBadgePreset.PHONE,
                    onConnect = {}
                )
            }
        }

        compose.onNodeWithContentDescription("Device type: Android phone").assertExists()
    }
}
