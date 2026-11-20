package `fun`.hackerlife.weavelo.ui

import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.Switch
import androidx.compose.material3.SwitchDefaults
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import `fun`.hackerlife.weavelo.PortForwardRule
import `fun`.hackerlife.weavelo.PortForwardStore
import `fun`.hackerlife.weavelo.TunnelDirection
import `fun`.hackerlife.weavelo.TunnelProtocol

@Composable
fun PortForwardDialog(
    onDismiss: () -> Unit,
    onRulesChanged: ((List<PortForwardRule>) -> Unit)? = null,
) {
    val p = WeaveloTokens.colors
    val context = LocalContext.current.applicationContext
    var rules by remember { mutableStateOf(PortForwardStore.list(context)) }
    var showAddForm by remember { mutableStateOf(false) }

    var nameInput by remember { mutableStateOf("") }
    var protocolInput by remember { mutableStateOf(TunnelProtocol.TCP) }
    var directionInput by remember { mutableStateOf(TunnelDirection.LOCAL_TO_REMOTE) }
    var bindPortInput by remember { mutableStateOf("") }
    var targetHostInput by remember { mutableStateOf("127.0.0.1") }
    var targetPortInput by remember { mutableStateOf("") }
    var formError by remember { mutableStateOf<String?>(null) }

    fun resetForm() {
        nameInput = ""
        protocolInput = TunnelProtocol.TCP
        directionInput = TunnelDirection.LOCAL_TO_REMOTE
        bindPortInput = ""
        targetHostInput = "127.0.0.1"
        targetPortInput = ""
        formError = null
    }

    val activeCount = rules.count { it.enabled }
    val invalidPortsMsg = tr("Enter valid bind and target ports (1-65535)")
    val invalidRuleMsg = tr("Invalid port forwarding rule")

    PrecisionDialog(
        title = tr("Port forwarding"),
        subtitle = tr("Manage TCP/UDP tunnel rules between local and remote hosts."),
        icon = MobileSymbol.Network,
        maxWidth = 520.dp,
        onDismissRequest = onDismiss,
        confirmButton = {
            PrecisionButton(
                text = tr("Close"),
                style = PrecisionButtonStyle.TONAL,
                compact = true,
                onClick = onDismiss,
            )
        },
    ) {
        Column(verticalArrangement = Arrangement.spacedBy(10.dp)) {
            Row(
                modifier = Modifier.fillMaxWidth(),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                PrecisionPillBadge(
                    text = trFormat("%d / %d active", activeCount, rules.size),
                    tone = if (activeCount > 0) PillTone.EMERALD else PillTone.NEUTRAL,
                    showDot = activeCount > 0,
                    monospace = true,
                )
                Spacer(Modifier.weight(1f))
                PrecisionButton(
                    text = if (showAddForm) tr("Cancel") else tr("Add Rule"),
                    style = if (showAddForm) PrecisionButtonStyle.SUBTLE else PrecisionButtonStyle.PRIMARY,
                    compact = true,
                    onClick = {
                        if (showAddForm) {
                            resetForm()
                            showAddForm = false
                        } else {
                            formError = null
                            showAddForm = true
                        }
                    },
                )
            }

            if (showAddForm) {
                PrecisionCard(
                    sunken = true,
                    cornerRadius = 14.dp,
                    contentPadding = PaddingValues(12.dp),
                    verticalSpacing = 8.dp,
                ) {
                    PrecisionTextField(
                        value = nameInput,
                        onValueChange = { nameInput = it; formError = null },
                        label = tr("Rule name (optional)"),
                    )
                    PrecisionSegmentedRow(
                        options = listOf(
                            TunnelProtocol.TCP to TunnelProtocol.TCP.displayLabel,
                            TunnelProtocol.UDP to TunnelProtocol.UDP.displayLabel,
                        ),
                        selected = protocolInput,
                        modifier = Modifier.fillMaxWidth(),
                        onSelect = { protocolInput = it; formError = null },
                    )
                    PrecisionSegmentedRow(
                        options = listOf(
                            TunnelDirection.LOCAL_TO_REMOTE to tr(TunnelDirection.LOCAL_TO_REMOTE.displayLabel),
                            TunnelDirection.REMOTE_TO_LOCAL to tr(TunnelDirection.REMOTE_TO_LOCAL.displayLabel),
                        ),
                        selected = directionInput,
                        modifier = Modifier.fillMaxWidth(),
                        onSelect = { directionInput = it; formError = null },
                    )
                    Row(
                        modifier = Modifier.fillMaxWidth(),
                        horizontalArrangement = Arrangement.spacedBy(8.dp),
                    ) {
                        val bindPortValue = bindPortInput.toIntOrNull()
                        val targetPortValue = targetPortInput.toIntOrNull()
                        PrecisionTextField(
                            value = bindPortInput,
                            onValueChange = { bindPortInput = it.filter(Char::isDigit); formError = null },
                            label = tr("Bind port"),
                            modifier = Modifier.weight(1f),
                            isError = bindPortInput.isNotEmpty() &&
                                (bindPortValue == null || bindPortValue !in PortForwardRule.MIN_PORT..PortForwardRule.MAX_PORT),
                            monospace = true,
                            keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Number),
                        )
                        PrecisionTextField(
                            value = targetPortInput,
                            onValueChange = { targetPortInput = it.filter(Char::isDigit); formError = null },
                            label = tr("Target port"),
                            modifier = Modifier.weight(1f),
                            isError = targetPortInput.isNotEmpty() &&
                                (targetPortValue == null || targetPortValue !in PortForwardRule.MIN_PORT..PortForwardRule.MAX_PORT),
                            monospace = true,
                            keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Number),
                        )
                    }
                    PrecisionTextField(
                        value = targetHostInput,
                        onValueChange = { targetHostInput = it; formError = null },
                        label = tr("Target host"),
                        isError = targetHostInput.isBlank(),
                        monospace = true,
                    )
                    formError?.let { message ->
                        Text(
                            text = message,
                            color = p.dangerText,
                            fontSize = 12.sp,
                        )
                    }
                    Row(
                        modifier = Modifier.fillMaxWidth(),
                        horizontalArrangement = Arrangement.End,
                        verticalAlignment = Alignment.CenterVertically,
                    ) {
                        PrecisionButton(
                            text = tr("Save"),
                            leadingSymbol = MobileSymbol.Check,
                            style = PrecisionButtonStyle.PRIMARY,
                            compact = true,
                            onClick = {
                                val bindPort = bindPortInput.toIntOrNull()
                                val targetPort = targetPortInput.toIntOrNull()
                                if (bindPort == null || targetPort == null) {
                                    formError = invalidPortsMsg
                                    return@PrecisionButton
                                }
                                val displayName = nameInput.trim().ifEmpty {
                                    "${protocolInput.displayLabel} :$bindPort"
                                }
                                val rule = PortForwardRule(
                                    name = displayName,
                                    protocol = protocolInput,
                                    direction = directionInput,
                                    bindPort = bindPort,
                                    targetHost = targetHostInput,
                                    targetPort = targetPort,
                                    enabled = true,
                                )
                                PortForwardStore.add(context, rule)
                                    .onSuccess { updated ->
                                        rules = updated
                                        onRulesChanged?.invoke(updated)
                                        resetForm()
                                        showAddForm = false
                                    }
                                    .onFailure { failure ->
                                        formError = failure.message ?: invalidRuleMsg
                                    }
                            },
                        )
                    }
                }
            }

            if (rules.isEmpty()) {
                Box(
                    modifier = Modifier
                        .fillMaxWidth()
                        .clip(RoundedCornerShape(12.dp))
                        .background(p.surfaceSunken)
                        .border(BorderStroke(1.dp, p.borderMuted), RoundedCornerShape(12.dp))
                        .padding(16.dp),
                    contentAlignment = Alignment.Center,
                ) {
                    Text(
                        text = tr("No port forwarding rules configured yet."),
                        color = p.textSecondary,
                        fontSize = 12.5.sp,
                    )
                }
            } else {
                LazyColumn(
                    modifier = Modifier.heightIn(max = 320.dp),
                    verticalArrangement = Arrangement.spacedBy(8.dp),
                ) {
                    items(rules, key = { it.id }) { rule ->
                        PortForwardRuleRow(
                            rule = rule,
                            onToggle = {
                                val updated = PortForwardStore.toggle(context, rule.id)
                                rules = updated
                                onRulesChanged?.invoke(updated)
                            },
                            onDelete = {
                                val updated = PortForwardStore.remove(context, rule.id)
                                rules = updated
                                onRulesChanged?.invoke(updated)
                            },
                        )
                    }
                }
            }
        }
    }
}

@Composable
private fun PortForwardRuleRow(
    rule: PortForwardRule,
    onToggle: () -> Unit,
    onDelete: () -> Unit,
) {
    val p = WeaveloTokens.colors
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .clip(RoundedCornerShape(12.dp))
            .background(if (rule.enabled) p.surface else p.surfaceSunken)
            .border(
                BorderStroke(1.dp, if (rule.enabled) p.border else p.borderMuted),
                RoundedCornerShape(12.dp),
            )
            .padding(horizontal = 12.dp, vertical = 10.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(10.dp),
    ) {
        Column(
            modifier = Modifier.weight(1f),
            verticalArrangement = Arrangement.spacedBy(4.dp),
        ) {
            Row(
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(6.dp),
            ) {
                PrecisionPillBadge(
                    text = rule.protocol.displayLabel,
                    tone = if (rule.enabled) PillTone.BRAND else PillTone.NEUTRAL,
                    monospace = true,
                )
                PrecisionPillBadge(
                    text = rule.direction.shortBadge,
                    tone = if (rule.enabled) PillTone.CYAN else PillTone.NEUTRAL,
                    monospace = true,
                )
                Text(
                    text = rule.name.ifBlank { "${rule.protocol.displayLabel} :${rule.bindPort}" },
                    color = if (rule.enabled) p.textPrimary else p.textSecondary,
                    fontSize = 13.sp,
                    fontWeight = FontWeight.SemiBold,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
            }
            Text(
                text = ":${rule.bindPort} -> ${rule.targetHost}:${rule.targetPort}",
                color = p.textTertiary,
                fontSize = 11.5.sp,
                fontFamily = FontFamily.Monospace,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
        }
        Switch(
            checked = rule.enabled,
            onCheckedChange = { onToggle() },
            colors = SwitchDefaults.colors(
                checkedThumbColor = p.brandOn,
                checkedTrackColor = p.brand,
                checkedBorderColor = Color.Transparent,
                uncheckedThumbColor = p.textSecondary,
                uncheckedTrackColor = p.surfaceSunken,
                uncheckedBorderColor = p.borderStrong,
            ),
        )
        MobileAction(
            symbol = MobileSymbol.Close,
            label = tr("Delete"),
            destructive = true,
            modifier = Modifier.size(34.dp),
            onClick = onDelete,
        )
    }
}
