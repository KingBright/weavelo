package `fun`.hackerlife.weavelo

/** Process-local intent, not credentials or a promise to survive force-stop. */
data class WorkspaceSessionRequest(val endpoint: String, val deviceId: String, val sessionKey: String,
    val incomingId: String? = null)

/** A stale Service teardown must never close a replacement owner's connection. */
class WorkspaceSessionOwnership {
    private var owner: Any? = null
    private var desired: WorkspaceSessionRequest? = null
    private var endedIncoming: String? = null

    @Synchronized fun request(): WorkspaceSessionRequest? = desired
    @Synchronized fun visibleRequest(): WorkspaceSessionRequest? = desired?.takeUnless { it.sessionKey == endedIncoming }

    @Synchronized fun attach(token: Any, request: WorkspaceSessionRequest, backendActive: Boolean): Boolean {
        val reuse = backendActive && desired?.endpoint == request.endpoint &&
            desired?.deviceId == request.deviceId && desired?.sessionKey == request.sessionKey &&
            desired?.incomingId == request.incomingId
        if (desired != request) endedIncoming = null
        owner = token
        desired = request
        return reuse
    }

    @Synchronized fun release(token: Any, preserveForForeground: Boolean): Boolean {
        if (owner !== token) return false
        owner = null
        if (!preserveForForeground) { desired = null; endedIncoming = null }
        return true
    }

    @Synchronized fun endIncoming(sessionKey: String): Boolean {
        if (desired?.sessionKey != sessionKey || desired?.incomingId == null) return false
        endedIncoming = sessionKey
        return true
    }

    /** A completed view may retire only after this connection's files finish. */
    @Synchronized fun retireEndedIncoming(filesBusy: Boolean): Boolean {
        val key = endedIncoming ?: return false
        return !filesBusy && disconnect(key)
    }

    @Synchronized fun disconnect(sessionKey: String): Boolean {
        if (desired?.sessionKey != sessionKey) return false
        owner = null
        desired = null
        endedIncoming = null
        return true
    }
}
