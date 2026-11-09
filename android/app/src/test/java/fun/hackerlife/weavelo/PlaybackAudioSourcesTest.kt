package `fun`.hackerlife.weavelo

import org.junit.Assert.*
import org.junit.Test

class PlaybackAudioSourcesTest {
    @Test fun rejectsHiddenSharedUidAndOtherUnsafeBindings() {
        fun safe(enabled: Boolean = true, sameProfile: Boolean = true, sharedUid: String? = null,
            packages: Array<String>? = arrayOf("video.app"), name: String = "video.app") =
            safePlaybackAudioIdentity(name, "weavelo", enabled, sameProfile, sharedUid, packages)
        assertTrue(safe())
        // Package visibility can hide the second package; the declaration still fails closed.
        assertFalse(safe(sharedUid = "shared.uid"))
        assertFalse(safe(packages = arrayOf("video.app", "other.app")))
        assertFalse(safe(packages = null))
        assertFalse(safe(enabled = false))
        assertFalse(safe(sameProfile = false))
        assertFalse(safe(name = "weavelo", packages = arrayOf("weavelo")))
    }

    @Test fun targetIdentityIncludesUidAndPackageUpdate() {
        val original = PlaybackAudioTarget("video.app", 12345, 100L)
        assertNotEquals(original, original.copy(uid = 12346))
        assertNotEquals(original, original.copy(updated = 101L))
        assertNotEquals(original, original.copy(packageName = "other.app"))
    }
}
