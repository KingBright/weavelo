package `fun`.hackerlife.weavelo

import android.graphics.BitmapFactory
import androidx.test.platform.app.InstrumentationRegistry
import `fun`.hackerlife.weavelo.managed.ManagedScreenBridge
import org.junit.Assert.*
import org.junit.Assume.assumeTrue
import org.junit.Test
import org.json.JSONArray
import org.json.JSONObject

/** Opt-in, read-only check of the already attached helper and one explicit app. */
class ManagedApplicationArtworkTest {
    @Test fun selectedApplicationHasNativeArtwork() {
        val packageName=InstrumentationRegistry.getArguments().getString("weavelo.artwork.package")
        assumeTrue("An explicit installed test application is required",!packageName.isNullOrBlank())
        InstrumentationRegistry.getInstrumentation().getUiAutomation(android.app.UiAutomation.FLAG_DONT_SUPPRESS_ACCESSIBILITY_SERVICES)
        val helper=checkNotNull(ManagedScreenBridge.attached()) {"Open the already authorized Weavelo helper first"}
        val png=helper.applicationIcon(packageName!!,256)
        val dimensions=BitmapFactory.Options().apply {inJustDecodeBounds=true}
        BitmapFactory.decodeByteArray(png,0,png.size,dimensions)
        assertEquals(256,dimensions.outWidth)
        assertEquals(256,dimensions.outHeight)
        assertTrue(png.size in 1..256*1024)
        val digest=java.security.MessageDigest.getInstance("SHA-256").digest(png)
        val icon=JSONObject().put("id",JSONArray(digest.map {it.toInt() and 255}))
            .put("png",JSONArray(png.map {it.toInt() and 255}))
        // Grant zero cannot own a source. Reaching this error proves JNI's
        // native PNG validation passed, without changing any live artwork.
        val payload=JSONObject().put("source",JSONObject().put("Window",1))
            .put("grant",0).put("epoch",0).put("icon",icon)
        try {
            PublisherClient.action(45,payload.toString())
            fail("An unowned source must not accept artwork")
        } catch(error:IllegalStateException) {
            assertEquals("Application artwork source is no longer selected",error.message)
        }
    }
}
