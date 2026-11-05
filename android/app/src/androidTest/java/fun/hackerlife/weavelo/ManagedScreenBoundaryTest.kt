package `fun`.hackerlife.weavelo

import android.app.UiAutomation
import android.net.Uri
import androidx.test.platform.app.InstrumentationRegistry
import org.junit.Assert.fail
import org.junit.Test

class ManagedScreenBoundaryTest {
    @Test fun normalAppUidCannotAttachOrInspectTheShellEndpoint() {
        val instrumentation = InstrumentationRegistry.getInstrumentation()
        instrumentation.getUiAutomation(UiAutomation.FLAG_DONT_SUPPRESS_ACCESSIBILITY_SERVICES)
        val context = instrumentation.targetContext
        for (method in listOf("attach", "status")) {
            try {
                context.contentResolver.call(Uri.parse("content://${context.packageName}.managed"), method, null, null)
                fail("Only the existing local shell may use the activation endpoint")
            } catch (_: SecurityException) { }
        }
    }
}
