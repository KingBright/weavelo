package `fun`.hackerlife.weavelo

import org.junit.Assert.*
import org.junit.Test

class WorkspaceSessionOwnershipTest {
    private val request = WorkspaceSessionRequest("192.0.2.1:39271", "device-a", "session-a")

    @Test fun recreatedUiReusesTheServiceConnectionAndSessionBinding() {
        val state = WorkspaceSessionOwnership(); val service = Any()
        assertFalse(state.attach(service, request, backendActive = false)) // open once
        repeat(4) { assertTrue(state.attach(service, request.copy(), backendActive = true)) }
        assertEquals(request, state.request())
    }

    @Test fun activityDisposalIsNotAnOwnershipReleaseAndForegroundCanReattach() {
        val state = WorkspaceSessionOwnership(); val service = Any()
        state.attach(service, request, false)
        // A UI has no Service token. Its teardown cannot stop the backend.
        assertFalse(state.release(Any(), preserveForForeground = false))
        assertEquals(request, state.request())
        assertTrue(state.attach(service, request, true))
    }

    @Test fun staleServiceTeardownCannotCloseOrForgetReplacementConnection() {
        val state = WorkspaceSessionOwnership(); val old = Any(); val replacement = Any()
        state.attach(old, request, false)
        val next = request.copy(deviceId = "device-b", sessionKey = "session-b")
        assertFalse(state.attach(replacement, next, false))
        assertFalse(state.release(old, preserveForForeground = false))
        assertEquals(next, state.request())
        assertTrue(state.release(replacement, preserveForForeground = false))
        assertNull(state.request())
    }

    @Test fun idleExpiryOnlyRetainsIntentForUserForegroundResume() {
        val state = WorkspaceSessionOwnership(); val old = Any()
        state.attach(old, request, false)
        assertTrue(state.release(old, preserveForForeground = true))
        assertEquals(request, state.request())
        assertFalse(state.release(old, preserveForForeground = false))
        assertFalse(state.attach(Any(), state.request()!!, backendActive = false))
    }

    @Test fun explicitDisconnectDoesNotResurrectOnUiRecreation() {
        val state = WorkspaceSessionOwnership(); val service = Any()
        state.attach(service, request, false)
        assertTrue(state.release(service, preserveForForeground = false))
        assertNull(state.request())
        assertFalse(state.release(service, preserveForForeground = true))
        assertNull(state.request())
    }

    @Test fun sameEndpointWithDifferentPeerOrSessionMustNeverReuseOldFileBinding() {
        for (next in listOf(request.copy(deviceId = "other-peer"), request.copy(sessionKey = "fresh-session"))) {
            val state = WorkspaceSessionOwnership(); val service = Any()
            state.attach(service, request, false)
            assertFalse(state.attach(service, next, backendActive = true))
            assertEquals(next, state.request())
        }
    }

    @Test fun failedOrSleepingBackendMustReopenEvenForTheSameUiIntent() {
        val state = WorkspaceSessionOwnership(); val service = Any()
        state.attach(service, request, false)
        assertFalse(state.attach(service, request, backendActive = false))
    }

    @Test fun aClaimedSessionCannotBeReusedAsOrdinaryPullOrAnotherOffer() {
        val state = WorkspaceSessionOwnership(); val service = Any()
        val incoming = request.copy(incomingId = "offer-a")
        state.attach(service, incoming, false)
        assertFalse(state.attach(service, request, backendActive = true))
        assertFalse(state.attach(service, incoming.copy(incomingId = "offer-b"), backendActive = true))
    }

    @Test fun explicitStopAfterIdleCannotLeaveARememberedReconnectIntent() {
        val state = WorkspaceSessionOwnership(); val service = Any()
        state.attach(service, request, false)
        state.release(service, preserveForForeground = true)
        assertTrue(state.disconnect(request.sessionKey))
        assertNull(state.request())
    }

    @Test fun staleUiDisconnectCannotStopANewPeersSession() {
        val state = WorkspaceSessionOwnership(); val service = Any()
        state.attach(service, request, false)
        val replacement = request.copy(deviceId = "device-b", sessionKey = "session-b")
        state.attach(service, replacement, false)
        assertFalse(state.disconnect(request.sessionKey))
        assertEquals(replacement, state.request())
    }

    @Test fun endedIncomingViewRetainsItsFileOwnerThenAcceptsANewOfferWithoutAStaleStop() {
        val state = WorkspaceSessionOwnership(); val old = Any(); val nextOwner = Any()
        val incoming = request.copy(incomingId = "offer-a")
        state.attach(old, incoming, false)
        assertTrue(state.endIncoming(incoming.sessionKey))
        assertNull(state.visibleRequest()) // recreation cannot reopen an ended view
        assertFalse(state.retireEndedIncoming(filesBusy = true))
        assertEquals(incoming, state.request()) // active files keep their actual connection
        assertTrue(state.retireEndedIncoming(filesBusy = false))
        assertNull(state.request())
        val next = request.copy(deviceId = "device-b", sessionKey = "session-b", incomingId = "offer-b")
        assertFalse(state.attach(nextOwner, next, false))
        assertFalse(state.release(old, preserveForForeground = false))
        assertFalse(state.endIncoming(incoming.sessionKey))
        assertFalse(state.retireEndedIncoming(filesBusy = false))
        assertEquals(next, state.visibleRequest())
    }

    @Test fun ordinaryPullAndLiveIncomingConnectionsCannotBeRetiredByTheEndedOfferPath() {
        val state = WorkspaceSessionOwnership(); val service = Any()
        state.attach(service, request, false)
        assertFalse(state.endIncoming(request.sessionKey))
        assertFalse(state.retireEndedIncoming(filesBusy = false))
        assertEquals(request, state.visibleRequest())
        val incoming = request.copy(incomingId = "offer-a")
        state.attach(service, incoming, false)
        assertFalse(state.retireEndedIncoming(filesBusy = false))
        assertEquals(incoming, state.visibleRequest())
    }
    @Test fun manualAddressOpensOnlyTheAuthenticatedMainDisplayOnce() {
        val intent = WorkspaceInitialSource()
        val manual = WorkspaceSessionRequest("192.0.2.10:5001", "", "manual", openMainDisplay = true)
        intent.bind(manual, sameSession = false)
        assertFalse(intent.takeMainDisplay("manual", available = false))
        assertFalse(intent.takeMainDisplay("different", available = true))
        assertTrue(intent.takeMainDisplay("manual", available = true))
        intent.bind(manual, sameSession = true)
        assertFalse(intent.takeMainDisplay("manual", available = true))
    }

    @Test fun cancellingManualStartupCannotSelectAfterDisconnectOrReplacement() {
        val intent = WorkspaceInitialSource()
        val manual = WorkspaceSessionRequest("192.0.2.10:5001", "", "manual", openMainDisplay = true)
        intent.bind(manual, sameSession = false)
        intent.cancel("manual")
        assertFalse(intent.takeMainDisplay("manual", available = true))
        val ordinary = WorkspaceSessionRequest("192.0.2.11:5001", "", "ordinary")
        intent.bind(ordinary, sameSession = false)
        assertFalse(intent.takeMainDisplay("manual", available = true))
        assertFalse(intent.takeMainDisplay("ordinary", available = true))
        intent.bind(manual.copy(incomingId = "offer"), sameSession = false)
        assertFalse(intent.takeMainDisplay("manual", available = true))
    }

    @Test fun primaryScreenSourcePrefersConcreteDisplayOverWaylandPlaceholder() {
        val waylandPlaceholder = WorkspaceSource("MainDisplay", "Desktop", 0, 0)
        val waylandDisplay = WorkspaceSource(org.json.JSONObject().put("Display", 0x8000_0001L), "Shared display", 3840, 2160)
        val appWindow = WorkspaceSource(org.json.JSONObject().put("Window", 42), "Terminal", 1280, 720, "Konsole")
        assertEquals(waylandDisplay, primaryScreenSource(listOf(waylandPlaceholder, waylandDisplay, appWindow)))

        val androidProjection = WorkspaceSource("MainDisplay", "Android selected screen or app", 1080, 2400, "MediaProjection")
        assertEquals(androidProjection, primaryScreenSource(listOf(androidProjection)))

        val macDisplay = WorkspaceSource(org.json.JSONObject().put("Display", 1), "Built-in Display (1728×1117)", 1728, 1117)
        assertEquals(macDisplay, primaryScreenSource(listOf(macDisplay, appWindow)))
    }

}
