package `fun`.hackerlife.weavelo

import android.content.Context
import android.content.SharedPreferences
import org.json.JSONArray
import org.json.JSONObject
import java.util.UUID

enum class TunnelProtocol(val wireValue: String, val displayLabel: String) {
    TCP("tcp", "TCP"),
    UDP("udp", "UDP");

    companion object {
        fun fromWireValueOrNull(value: String?): TunnelProtocol? {
            val normalized = value?.trim()?.lowercase().orEmpty()
            if (normalized.isEmpty()) return null
            return entries.firstOrNull {
                it.wireValue == normalized || it.name.equals(normalized, ignoreCase = true)
            }
        }

        fun fromWireValue(value: String?): TunnelProtocol =
            fromWireValueOrNull(value) ?: TCP
    }
}

enum class TunnelDirection(
    val wireValue: String,
    val displayLabel: String,
    val shortBadge: String,
) {
    LOCAL_TO_REMOTE("local_to_remote", "Local -> Remote", "L->R"),
    REMOTE_TO_LOCAL("remote_to_local", "Remote -> Local", "R->L");

    companion object {
        fun fromWireValueOrNull(value: String?): TunnelDirection? {
            val normalized = value?.trim()?.lowercase().orEmpty()
            if (normalized.isEmpty()) return null
            return entries.firstOrNull {
                it.wireValue == normalized || it.name.equals(normalized, ignoreCase = true)
            }
        }

        fun fromWireValue(value: String?): TunnelDirection =
            fromWireValueOrNull(value) ?: LOCAL_TO_REMOTE
    }
}

data class PortForwardRule(
    val id: String = UUID.randomUUID().toString(),
    val name: String,
    val protocol: TunnelProtocol = TunnelProtocol.TCP,
    val direction: TunnelDirection = TunnelDirection.LOCAL_TO_REMOTE,
    val bindPort: Int,
    val targetHost: String = "127.0.0.1",
    val targetPort: Int,
    val enabled: Boolean = true,
) {
    fun normalized(): PortForwardRule = copy(
        id = id.trim(),
        name = name.trim(),
        targetHost = targetHost.trim(),
    )

    fun validate(): String? {
        if (id.isBlank()) return "Rule ID must not be blank"
        if (bindPort !in MIN_PORT..MAX_PORT) {
            return "Bind port must be between $MIN_PORT and $MAX_PORT"
        }
        if (targetHost.isBlank()) return "Target host must not be blank"
        if (targetPort !in MIN_PORT..MAX_PORT) {
            return "Target port must be between $MIN_PORT and $MAX_PORT"
        }
        return null
    }

    fun isValid(): Boolean = validate() == null

    fun toJsonObject(): JSONObject = JSONObject()
        .put(KEY_ID, id)
        .put(KEY_NAME, name)
        .put(KEY_PROTOCOL, protocol.wireValue)
        .put(KEY_DIRECTION, direction.wireValue)
        .put(KEY_BIND_PORT, bindPort)
        .put(KEY_TARGET_HOST, targetHost)
        .put(KEY_TARGET_PORT, targetPort)
        .put(KEY_ENABLED, enabled)

    fun toJson(): String = toJsonObject().toString()

    companion object {
        const val MIN_PORT = 1
        const val MAX_PORT = 65535

        private const val KEY_ID = "id"
        private const val KEY_NAME = "name"
        private const val KEY_PROTOCOL = "protocol"
        private const val KEY_DIRECTION = "direction"
        private const val KEY_BIND_PORT = "bind_port"
        private const val KEY_TARGET_HOST = "target_host"
        private const val KEY_TARGET_PORT = "target_port"
        private const val KEY_ENABLED = "enabled"

        fun fromJsonObject(json: JSONObject?): PortForwardRule? {
            if (json == null) return null
            val id = json.optString(KEY_ID, "").trim()
            val protocol = TunnelProtocol.fromWireValueOrNull(json.optString(KEY_PROTOCOL, "")) ?: return null
            val direction = TunnelDirection.fromWireValueOrNull(json.optString(KEY_DIRECTION, "")) ?: return null
            if (!json.has(KEY_BIND_PORT) || !json.has(KEY_TARGET_PORT)) return null
            val bindPort = json.optInt(KEY_BIND_PORT, -1)
            val targetPort = json.optInt(KEY_TARGET_PORT, -1)
            val targetHost = json.optString(KEY_TARGET_HOST, "").trim()
            val rule = PortForwardRule(
                id = id,
                name = json.optString(KEY_NAME, "").trim(),
                protocol = protocol,
                direction = direction,
                bindPort = bindPort,
                targetHost = targetHost,
                targetPort = targetPort,
                enabled = json.optBoolean(KEY_ENABLED, true),
            )
            return rule.takeIf { it.isValid() }
        }

        fun fromJson(raw: String?): PortForwardRule? {
            if (raw.isNullOrBlank()) return null
            return runCatching { fromJsonObject(JSONObject(raw)) }.getOrNull()
        }
    }
}

object PortForwardStore {
    private const val PREFS_NAME = "weavelo_port_forward_store"
    private const val KEY_RULES_JSON = "port_forward_rules_json"

    fun encodeRules(rules: List<PortForwardRule>): String {
        val array = JSONArray()
        rules.forEach { array.put(it.toJsonObject()) }
        return array.toString()
    }

    fun decodeRules(raw: String?): List<PortForwardRule> {
        if (raw.isNullOrBlank()) return emptyList()
        return runCatching {
            val array = JSONArray(raw)
            val accepted = mutableListOf<PortForwardRule>()
            val seenIds = mutableSetOf<String>()
            val seenBindings = mutableSetOf<Pair<TunnelProtocol, Int>>()
            for (index in 0 until array.length()) {
                val rule = PortForwardRule.fromJsonObject(array.optJSONObject(index)) ?: continue
                val binding = rule.protocol to rule.bindPort
                if (seenIds.add(rule.id) && seenBindings.add(binding)) {
                    accepted += rule
                }
            }
            accepted.toList()
        }.getOrDefault(emptyList())
    }

    fun validateForAdd(
        rule: PortForwardRule,
        existing: List<PortForwardRule>,
    ): String? {
        val normalized = rule.normalized()
        normalized.validate()?.let { return it }
        if (existing.any { it.id == normalized.id }) {
            return "A rule with this ID already exists"
        }
        if (existing.any { it.protocol == normalized.protocol && it.bindPort == normalized.bindPort }) {
            return "Bind port ${normalized.bindPort}/${normalized.protocol.displayLabel} is already in use"
        }
        return null
    }

    fun list(context: Context): List<PortForwardRule> =
        list(context.getSharedPreferences(PREFS_NAME, Context.MODE_PRIVATE))

    fun list(prefs: SharedPreferences): List<PortForwardRule> =
        decodeRules(prefs.getString(KEY_RULES_JSON, null))

    fun saveAll(context: Context, rules: List<PortForwardRule>) {
        saveAll(context.getSharedPreferences(PREFS_NAME, Context.MODE_PRIVATE), rules)
    }

    fun saveAll(prefs: SharedPreferences, rules: List<PortForwardRule>) {
        prefs.edit().putString(KEY_RULES_JSON, encodeRules(rules)).apply()
    }

    fun add(context: Context, rule: PortForwardRule): Result<List<PortForwardRule>> =
        add(context.getSharedPreferences(PREFS_NAME, Context.MODE_PRIVATE), rule)

    fun add(prefs: SharedPreferences, rule: PortForwardRule): Result<List<PortForwardRule>> {
        val normalized = rule.normalized()
        val existing = list(prefs)
        validateForAdd(normalized, existing)?.let { error ->
            return Result.failure(IllegalArgumentException(error))
        }
        val updated = existing + normalized
        saveAll(prefs, updated)
        return Result.success(updated)
    }

    fun toggle(context: Context, id: String): List<PortForwardRule> =
        toggle(context.getSharedPreferences(PREFS_NAME, Context.MODE_PRIVATE), id)

    fun toggle(prefs: SharedPreferences, id: String): List<PortForwardRule> {
        val updated = list(prefs).map { rule ->
            if (rule.id == id) rule.copy(enabled = !rule.enabled) else rule
        }
        saveAll(prefs, updated)
        return updated
    }

    fun remove(context: Context, id: String): List<PortForwardRule> =
        remove(context.getSharedPreferences(PREFS_NAME, Context.MODE_PRIVATE), id)

    fun remove(prefs: SharedPreferences, id: String): List<PortForwardRule> {
        val updated = list(prefs).filterNot { it.id == id }
        saveAll(prefs, updated)
        return updated
    }
}
