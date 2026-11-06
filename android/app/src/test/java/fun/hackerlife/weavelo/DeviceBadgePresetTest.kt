package `fun`.hackerlife.weavelo

import org.junit.Assert.assertEquals
import org.junit.Test

class DeviceBadgePresetTest {
    @Test fun knownDiscoveryNamesMapToTheExistingArtworkPresets() {
        assertEquals(DeviceBadgePreset.COMPUTER, DeviceBadgePreset.fromWire("computer"))
        assertEquals(DeviceBadgePreset.MAC_BOOK, DeviceBadgePreset.fromWire("mac_book"))
        assertEquals(DeviceBadgePreset.MAC_STUDIO, DeviceBadgePreset.fromWire("mac_studio"))
        assertEquals(DeviceBadgePreset.WINDOWS, DeviceBadgePreset.fromWire("windows"))
        assertEquals(DeviceBadgePreset.LINUX, DeviceBadgePreset.fromWire("linux"))
        assertEquals(DeviceBadgePreset.PHONE, DeviceBadgePreset.fromWire("phone"))
    }

    @Test fun unknownOrMissingPresetUsesComputerWithoutGuessingFromDeviceName() {
        val device = HostDevice("nubia", "nubia NX741J", "127.0.0.1:40000", "LAN", true, true)
        assertEquals(DeviceBadgePreset.COMPUTER, device.deviceBadge)
        assertEquals(DeviceBadgePreset.COMPUTER, DeviceBadgePreset.fromWire("unknown"))
        assertEquals(DeviceBadgePreset.COMPUTER, DeviceBadgePreset.fromWire(null))
    }
}
