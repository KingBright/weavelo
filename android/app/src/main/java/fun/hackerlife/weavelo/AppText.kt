package `fun`.hackerlife.weavelo

/** An app-owned message key and its opaque values; never construct from remote or OS text. */
data class AppText(val key: String, val arguments: List<Any> = emptyList())

internal interface AppTextCarrier { val text: AppText }
private fun english(text: AppText): String = if (text.arguments.isEmpty()) text.key
    else String.format(java.util.Locale.ROOT, text.key, *text.arguments.toTypedArray())

internal class AppTextException(override val text: AppText) : IllegalStateException(english(text)), AppTextCarrier
internal class AppTextArgumentException(override val text: AppText) : IllegalArgumentException(english(text)), AppTextCarrier

internal fun appCheck(condition: Boolean, key: String) {
    if (!condition) throw AppTextException(AppText(key))
}
internal fun appRequire(condition: Boolean, key: String) {
    if (!condition) throw AppTextArgumentException(AppText(key))
}
internal fun appError(key: String): Nothing = throw AppTextException(AppText(key))

internal fun appTextOf(error: Throwable?): AppText? {
    var current = error
    repeat(8) {
        if (current is AppTextCarrier) return (current as AppTextCarrier).text
        current = current?.cause
    }
    return null
}

/** Match only complete native results, with the already observed network name as an opaque value. */
internal fun pairingResultText(result: String?, networkName: String): AppText? = when (result) {
    "Already in network $networkName" -> AppText("Already in network %s", listOf(networkName))
    "Joined network $networkName. Discovering members..." -> AppText("Joined network %s. Discovering members...", listOf(networkName))
    "Joined network $networkName. Discovering members... Storage durability confirmation reported an error." ->
        AppText("Joined network %s. Discovering members... Storage durability confirmation reported an error.", listOf(networkName))
    "error:Stop file reception and screen sharing before changing networks" -> AppText("Stop file reception and screen sharing before changing networks")
    "error:native library not loaded" -> AppText("native library not loaded")
    "error:No join result returned" -> AppText("No join result returned")
    else -> null
}
