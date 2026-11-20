//! One-click redacted diagnostics bundle for safe export and bug reporting.
//!
//! Strips credentials, bearer/OAuth tokens, PEM blocks, long hex/base64 secrets,
//! and non-loopback IPv4/IPv6 addresses while enforcing a strict maximum output
//! size by truncating the oldest events first.

use serde::{Deserialize, Serialize};
use std::net::{Ipv4Addr, Ipv6Addr};

/// Current schema version for [`DiagnosticBundle`].
pub const DIAGNOSTIC_BUNDLE_SCHEMA_VERSION: u32 = 1;

/// Maximum serialized UTF-8 byte size for a diagnostics bundle JSON payload (256 KiB).
pub const MAX_DIAGNOSTIC_BUNDLE_BYTES: usize = 256 * 1024;

const REDACTED_PEM: &str = "[REDACTED_PEM]";
const REDACTED_TOKEN: &str = "[REDACTED_TOKEN]";
const REDACTED_SECRET: &str = "[REDACTED_SECRET]";
const REDACTED_IP: &str = "[REDACTED_IP]";

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct DiagnosticTransportSummary {
    pub route_mode: String,
    pub active_scope: String,
    pub relay_connected: bool,
    pub p2p_state: String,
    pub rtt_ms: f32,
    pub packet_loss_percent: f32,
    pub fps: f32,
    pub video_bitrate_kbps: u32,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct DiagnosticSessionSummary {
    pub encoder_profile: String,
    pub chroma: String,
    pub hdr_mode: String,
    pub virtual_displays_active: u32,
    pub port_forwards_active: u32,
    pub privacy_mode_active: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DiagnosticBundle {
    pub schema_version: u32,
    pub generated_at_unix_ms: u64,
    pub app_version: String,
    pub platform: String,
    pub transport: DiagnosticTransportSummary,
    pub session: DiagnosticSessionSummary,
    pub recent_events: Vec<String>,
}

impl Default for DiagnosticBundle {
    fn default() -> Self {
        Self {
            schema_version: DIAGNOSTIC_BUNDLE_SCHEMA_VERSION,
            generated_at_unix_ms: 0,
            app_version: String::new(),
            platform: String::new(),
            transport: DiagnosticTransportSummary::default(),
            session: DiagnosticSessionSummary::default(),
            recent_events: Vec::new(),
        }
    }
}

impl DiagnosticBundle {
    /// Returns a cloned bundle with all text fields and events redacted and
    /// non-finite floating-point metrics normalized to `0.0`.
    pub fn redacted(&self) -> Self {
        Self {
            schema_version: self.schema_version,
            generated_at_unix_ms: self.generated_at_unix_ms,
            app_version: redact_sensitive_text(&self.app_version),
            platform: redact_sensitive_text(&self.platform),
            transport: DiagnosticTransportSummary {
                route_mode: redact_sensitive_text(&self.transport.route_mode),
                active_scope: redact_sensitive_text(&self.transport.active_scope),
                relay_connected: self.transport.relay_connected,
                p2p_state: redact_sensitive_text(&self.transport.p2p_state),
                rtt_ms: finite_or_zero(self.transport.rtt_ms),
                packet_loss_percent: finite_or_zero(self.transport.packet_loss_percent),
                fps: finite_or_zero(self.transport.fps),
                video_bitrate_kbps: self.transport.video_bitrate_kbps,
            },
            session: DiagnosticSessionSummary {
                encoder_profile: redact_sensitive_text(&self.session.encoder_profile),
                chroma: redact_sensitive_text(&self.session.chroma),
                hdr_mode: redact_sensitive_text(&self.session.hdr_mode),
                virtual_displays_active: self.session.virtual_displays_active,
                port_forwards_active: self.session.port_forwards_active,
                privacy_mode_active: self.session.privacy_mode_active,
            },
            recent_events: self
                .recent_events
                .iter()
                .map(|event| redact_sensitive_text(event))
                .collect(),
        }
    }

    /// Serializes this bundle into redacted JSON, strictly bounded to
    /// [`MAX_DIAGNOSTIC_BUNDLE_BYTES`] by dropping the oldest entries from
    /// `recent_events` if necessary.
    pub fn to_redacted_json(&self) -> Result<String, serde_json::Error> {
        let mut bundle = self.redacted();
        let json = serde_json::to_string(&bundle)?;
        if json.len() <= MAX_DIAGNOSTIC_BUNDLE_BYTES {
            return Ok(json);
        }

        // Binary search for the largest suffix of recent_events (newest events)
        // that keeps the serialized JSON within MAX_DIAGNOSTIC_BUNDLE_BYTES.
        let all_events = std::mem::take(&mut bundle.recent_events);
        let mut lo = 0usize;
        let mut hi = all_events.len();
        let mut best_json: Option<String> = None;

        while lo <= hi {
            let keep = lo + (hi - lo) / 2;
            let start = all_events.len() - keep;
            bundle.recent_events = all_events[start..].to_vec();
            let candidate = serde_json::to_string(&bundle)?;
            if candidate.len() <= MAX_DIAGNOSTIC_BUNDLE_BYTES {
                best_json = Some(candidate);
                lo = keep + 1;
            } else if keep == 0 {
                break;
            } else {
                hi = keep - 1;
            }
        }

        if let Some(bounded) = best_json {
            return Ok(bounded);
        }

        // Extreme fallback: even with zero recent_events, metadata fields alone
        // exceeded MAX_DIAGNOSTIC_BUNDLE_BYTES. Clamp metadata strings safely.
        bundle.recent_events.clear();
        const MAX_META_FIELD_BYTES: usize = 4096;
        truncate_utf8_in_place(&mut bundle.app_version, MAX_META_FIELD_BYTES);
        truncate_utf8_in_place(&mut bundle.platform, MAX_META_FIELD_BYTES);
        truncate_utf8_in_place(&mut bundle.transport.route_mode, MAX_META_FIELD_BYTES);
        truncate_utf8_in_place(&mut bundle.transport.active_scope, MAX_META_FIELD_BYTES);
        truncate_utf8_in_place(&mut bundle.transport.p2p_state, MAX_META_FIELD_BYTES);
        truncate_utf8_in_place(&mut bundle.session.encoder_profile, MAX_META_FIELD_BYTES);
        truncate_utf8_in_place(&mut bundle.session.chroma, MAX_META_FIELD_BYTES);
        truncate_utf8_in_place(&mut bundle.session.hdr_mode, MAX_META_FIELD_BYTES);

        serde_json::to_string(&bundle)
    }
}

fn finite_or_zero(value: f32) -> f32 {
    if value.is_finite() { value } else { 0.0 }
}

fn truncate_utf8_in_place(s: &mut String, max_bytes: usize) {
    if s.len() <= max_bytes {
        return;
    }
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    s.truncate(end);
}

/// Strictly redacts sensitive material from free-form diagnostic text:
/// - PEM blocks (`-----BEGIN ...-----` through `-----END ...-----`)
/// - URL embedded credentials (`scheme://user:password@host`)
/// - Bearer / OAuth / Basic tokens, JWTs, and common provider token prefixes
/// - Key-value passwords, secrets, tokens, and keys (`password=...`, `secret=...`, `token=...`, etc.)
/// - Non-loopback IPv4 and IPv6 addresses (preserving loopback `127.0.0.0/8`, `::1`, `::ffff:127.0.0.1`)
/// - Long hex (>= 32 chars) and high-entropy base64/base64url secrets
pub fn redact_sensitive_text(input: &str) -> String {
    if input.is_empty() {
        return String::new();
    }
    let step1 = redact_pem_blocks(input);
    let step2 = redact_url_userinfo(&step1);
    let step3 = redact_auth_scheme_and_known_tokens(&step2);
    let step4 = redact_key_value_secrets(&step3);
    let step5 = redact_non_loopback_ips(&step4);
    redact_long_hex_and_base64_secrets(&step5)
}

fn redact_pem_blocks(input: &str) -> String {
    let lower = input.to_ascii_lowercase();
    let begin_marker = "-----begin ";
    let end_marker = "-----end ";

    let mut out = String::with_capacity(input.len());
    let mut cursor = 0usize;

    while let Some(rel_start) = lower[cursor..].find(begin_marker) {
        let start = cursor + rel_start;
        let after_begin = start + begin_marker.len();

        // Find the closing "-----" on the BEGIN header line.
        let header_end = lower[after_begin..]
            .find("-----")
            .and_then(|rel| {
                let slice = &lower[after_begin..after_begin + rel];
                if slice.len() <= 80 && !slice.contains('\n') && !slice.contains('\r') {
                    Some(after_begin + rel + 5)
                } else {
                    None
                }
            });

        let Some(body_start) = header_end else {
            out.push_str(&input[cursor..after_begin]);
            cursor = after_begin;
            continue;
        };

        out.push_str(&input[cursor..start]);
        out.push_str(REDACTED_PEM);

        if let Some(rel_end_marker) = lower[body_start..].find(end_marker) {
            let end_marker_start = body_start + rel_end_marker + end_marker.len();
            if let Some(rel_end_dashes) = lower[end_marker_start..].find("-----") {
                cursor = end_marker_start + rel_end_dashes + 5;
            } else {
                cursor = input.len();
            }
        } else {
            // Truncated PEM block: redact through the end of the input.
            cursor = input.len();
        }
    }

    out.push_str(&input[cursor..]);
    out
}

fn redact_url_userinfo(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut cursor = 0usize;

    while let Some(rel_sep) = input[cursor..].find("://") {
        let sep_pos = cursor + rel_sep;
        let authority_start = sep_pos + 3;
        let bytes = input.as_bytes();

        let mut authority_end = authority_start;
        while authority_end < bytes.len() {
            let b = bytes[authority_end];
            if b.is_ascii_whitespace()
                || matches!(b, b'/' | b'?' | b'#' | b'"' | b'\'' | b')' | b'>' | b',' | b';')
            {
                break;
            }
            authority_end += 1;
        }

        let authority = &input[authority_start..authority_end];
        if let Some(at_pos) = authority.rfind('@') {
            let userinfo = &authority[..at_pos];
            if let Some(colon_pos) = userinfo.find(':') {
                let user = &userinfo[..colon_pos];
                let pass = &userinfo[colon_pos + 1..];
                if !pass.is_empty() && !pass.starts_with("[REDACTED_") {
                    out.push_str(&input[cursor..authority_start]);
                    out.push_str(user);
                    out.push(':');
                    out.push_str(REDACTED_SECRET);
                    out.push_str(&authority[at_pos..]);
                    cursor = authority_end;
                    continue;
                }
            }
        }

        out.push_str(&input[cursor..authority_start]);
        cursor = authority_start;
    }

    out.push_str(&input[cursor..]);
    out
}

fn redact_auth_scheme_and_known_tokens(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = String::with_capacity(input.len());
    let mut i = 0usize;

    while i < bytes.len() {
        let at_word_boundary = i == 0
            || !(bytes[i - 1].is_ascii_alphanumeric()
                || bytes[i - 1] == b'_'
                || bytes[i - 1] == b'-');

        if at_word_boundary {
            // 1. Auth schemes: Bearer <tok>, OAuth <tok>, Basic <b64>
            if let Some((scheme_len, is_basic)) = match_auth_scheme(&input[i..]) {
                let mut ws_end = i + scheme_len;
                while ws_end < bytes.len() && matches!(bytes[ws_end], b' ' | b'\t') {
                    ws_end += 1;
                }
                if ws_end > i + scheme_len && ws_end < bytes.len() {
                    let tok_end = scan_token_end(bytes, ws_end);
                    let token = &input[ws_end..tok_end];
                    if !token.is_empty()
                        && !token.starts_with("[REDACTED_")
                        && (!is_basic || looks_like_basic_credential(token, &input[..i]))
                    {
                        out.push_str(&input[i..ws_end]);
                        out.push_str(REDACTED_TOKEN);
                        i = tok_end;
                        continue;
                    }
                }
            }

            // 2. Standalone JWT: eyJ<b64url>.eyJ<b64url>.<b64url>
            if input[i..].starts_with("eyJ") {
                let tok_end = scan_jwt_end(bytes, i);
                if is_valid_jwt(&input[i..tok_end]) {
                    out.push_str(REDACTED_TOKEN);
                    i = tok_end;
                    continue;
                }
            }

            // 3. Well-known provider token prefixes
            if let Some(prefix_len) = match_known_token_prefix(&input[i..]) {
                let tok_end = scan_token_end(bytes, i);
                if tok_end >= i + prefix_len + 12 {
                    out.push_str(REDACTED_TOKEN);
                    i = tok_end;
                    continue;
                }
            }
        }

        let ch = input[i..].chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }

    out
}

fn match_auth_scheme(rest: &str) -> Option<(usize, bool)> {
    let bytes = rest.as_bytes();
    for (scheme, is_basic) in [("bearer", false), ("oauth", false), ("basic", true)] {
        let len = scheme.len();
        if bytes.len() > len
            && bytes[..len].eq_ignore_ascii_case(scheme.as_bytes())
            && matches!(bytes[len], b' ' | b'\t')
        {
            return Some((len, is_basic));
        }
    }
    None
}

fn looks_like_basic_credential(token: &str, preceding: &str) -> bool {
    let trimmed_prev = preceding.trim_end();
    if trimmed_prev
        .to_ascii_lowercase()
        .ends_with("authorization:")
        || trimmed_prev
            .to_ascii_lowercase()
            .ends_with("authorization\":")
    {
        return true;
    }
    token.len() >= 12
        && token.bytes().all(|b| {
            b.is_ascii_alphanumeric() || matches!(b, b'+' | b'/' | b'=' | b'_' | b'-')
        })
        && (token.ends_with('=')
            || (token.bytes().any(|b| b.is_ascii_uppercase())
                && token.bytes().any(|b| b.is_ascii_lowercase())
                && token.bytes().any(|b| b.is_ascii_digit())))
}

fn scan_token_end(bytes: &[u8], start: usize) -> usize {
    let mut end = start;
    while end < bytes.len() {
        let b = bytes[end];
        if b.is_ascii_alphanumeric()
            || matches!(b, b'.' | b'_' | b'~' | b'+' | b'/' | b'=' | b'-')
        {
            end += 1;
        } else {
            break;
        }
    }
    end
}

fn scan_jwt_end(bytes: &[u8], start: usize) -> usize {
    let mut end = start;
    while end < bytes.len() {
        let b = bytes[end];
        if b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-') {
            end += 1;
        } else {
            break;
        }
    }
    end
}

fn is_valid_jwt(candidate: &str) -> bool {
    let mut parts = candidate.split('.');
    let (Some(header), Some(payload), Some(sig), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return false;
    };
    header.starts_with("eyJ")
        && header.len() >= 8
        && payload.len() >= 8
        && sig.len() >= 6
}

fn match_known_token_prefix(rest: &str) -> Option<usize> {
    const PREFIXES: &[&str] = &[
        "github_pat_",
        "sk-proj-",
        "sk-live-",
        "sk-test-",
        "sk_live_",
        "sk_test_",
        "glpat-",
        "xoxb-",
        "xoxp-",
        "xoxa-",
        "xoxr-",
        "ya29.",
        "AIza",
        "AKIA",
        "ghp_",
        "gho_",
        "ghu_",
        "ghs_",
        "ghr_",
    ];
    for prefix in PREFIXES {
        if rest.starts_with(prefix) {
            return Some(prefix.len());
        }
    }
    None
}

fn redact_key_value_secrets(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = String::with_capacity(input.len());
    let mut i = 0usize;

    while i < bytes.len() {
        let prev_is_key_char = i > 0
            && (bytes[i - 1].is_ascii_alphanumeric()
                || bytes[i - 1] == b'_'
                || bytes[i - 1] == b'-');

        if !prev_is_key_char {
            // Optional opening quote around key name (e.g. JSON "password": "...")
            let (key_quote, key_start) = if matches!(bytes[i], b'"' | b'\'') {
                (Some(bytes[i]), i + 1)
            } else {
                (None, i)
            };

            let mut key_end = key_start;
            while key_end < bytes.len()
                && (bytes[key_end].is_ascii_alphanumeric()
                    || matches!(bytes[key_end], b'_' | b'-'))
            {
                key_end += 1;
            }

            if key_end > key_start {
                let key = &input[key_start..key_end];
                let after_key = if let Some(q) = key_quote {
                    if key_end < bytes.len() && bytes[key_end] == q {
                        Some(key_end + 1)
                    } else {
                        None
                    }
                } else {
                    Some(key_end)
                };

                if let Some(mut sep_pos) = after_key
                    && is_sensitive_key(key)
                {
                    while sep_pos < bytes.len() && matches!(bytes[sep_pos], b' ' | b'\t') {
                        sep_pos += 1;
                    }
                    if sep_pos < bytes.len() && matches!(bytes[sep_pos], b'=' | b':') {
                        let sep_char = bytes[sep_pos];
                        // Avoid matching Rust `::` or `==` or `=>`
                        let next_byte = bytes.get(sep_pos + 1).copied();
                        let invalid_sep = (sep_char == b':' && next_byte == Some(b':'))
                            || (sep_char == b'=' && matches!(next_byte, Some(b'=') | Some(b'>')));

                        if !invalid_sep {
                            let mut val_start = sep_pos + 1;
                            while val_start < bytes.len()
                                && matches!(bytes[val_start], b' ' | b'\t')
                            {
                                val_start += 1;
                            }

                            if val_start < bytes.len() {
                                if matches!(bytes[val_start], b'"' | b'\'') {
                                    let quote = bytes[val_start];
                                    let inner_start = val_start + 1;
                                    let mut inner_end = inner_start;
                                    while inner_end < bytes.len() {
                                        if bytes[inner_end] == b'\\' && inner_end + 1 < bytes.len()
                                        {
                                            let escaped_len = input[inner_end + 1..]
                                                .chars()
                                                .next()
                                                .map_or(0, char::len_utf8);
                                            inner_end += 1 + escaped_len;
                                            continue;
                                        }
                                        if bytes[inner_end] == quote
                                            || bytes[inner_end] == b'\n'
                                            || bytes[inner_end] == b'\r'
                                        {
                                            break;
                                        }
                                        inner_end += 1;
                                    }
                                    let inner = &input[inner_start..inner_end];
                                    if !inner.is_empty() && !inner.contains("[REDACTED_") {
                                        out.push_str(&input[i..inner_start]);
                                        out.push_str(REDACTED_SECRET);
                                        if inner_end < bytes.len() && bytes[inner_end] == quote {
                                            out.push(quote as char);
                                            i = inner_end + 1;
                                        } else {
                                            i = inner_end;
                                        }
                                        continue;
                                    }
                                } else {
                                    let mut val_end = val_start;
                                    while val_end < bytes.len() {
                                        let b = bytes[val_end];
                                        if b.is_ascii_whitespace()
                                            || matches!(
                                                b,
                                                b'&' | b';'
                                                    | b','
                                                    | b')'
                                                    | b']'
                                                    | b'}'
                                                    | b'"'
                                                    | b'\''
                                                    | b'<'
                                                    | b'>'
                                            )
                                        {
                                            break;
                                        }
                                        val_end += 1;
                                    }
                                    let val = &input[val_start..val_end];
                                    // Preserve already-redacted values like `Authorization: Bearer [REDACTED_TOKEN]`.
                                    let next_segment_redacted = input[val_start..]
                                        .starts_with("Bearer [REDACTED_")
                                        || input[val_start..].starts_with("OAuth [REDACTED_")
                                        || input[val_start..].starts_with("Basic [REDACTED_");
                                    if !val.is_empty()
                                        && !val.contains("[REDACTED_")
                                        && !next_segment_redacted
                                    {
                                        out.push_str(&input[i..val_start]);
                                        out.push_str(REDACTED_SECRET);
                                        i = val_end;
                                        continue;
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        let ch = input[i..].chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }

    out
}

fn is_sensitive_key(key: &str) -> bool {
    let lower = key.to_ascii_lowercase();
    const EXACT_KEYS: &[&str] = &[
        "password",
        "passwd",
        "pwd",
        "passphrase",
        "secret",
        "token",
        "key",
        "apikey",
        "api_key",
        "api-key",
        "private_key",
        "private-key",
        "secret_key",
        "secret-key",
        "access_key",
        "access-key",
        "auth_key",
        "auth-key",
        "encryption_key",
        "encryption-key",
        "signing_key",
        "signing-key",
        "session_key",
        "session-key",
        "master_key",
        "master-key",
        "psk",
        "pre_shared_key",
        "pre-shared-key",
        "authorization",
        "cookie",
        "set-cookie",
        "set_cookie",
        "credential",
        "credentials",
        "jwt",
        "bearer",
        "oauth",
    ];
    if EXACT_KEYS.contains(&lower.as_str()) {
        return true;
    }

    const SUFFIXES: &[&str] = &[
        "_password",
        "-password",
        "_passwd",
        "-passwd",
        "_passphrase",
        "-passphrase",
        "_secret",
        "-secret",
        "_token",
        "-token",
        "_apikey",
        "-apikey",
        "_key",
        "-key",
        "_psk",
        "-psk",
        "_credential",
        "-credential",
        "_credentials",
        "-credentials",
    ];
    SUFFIXES.iter().any(|suffix| lower.ends_with(suffix))
}

fn redact_non_loopback_ips(input: &str) -> String {
    let ipv6_redacted = redact_non_loopback_ipv6(input);
    redact_non_loopback_ipv4(&ipv6_redacted)
}

fn is_loopback_ipv6(addr: &Ipv6Addr) -> bool {
    addr.is_loopback() || addr.to_ipv4_mapped().is_some_and(|v4| v4.is_loopback())
}

fn parse_ipv6_with_optional_zone(raw: &str) -> Option<Ipv6Addr> {
    let base = match raw.find('%') {
        Some(idx) => {
            let zone = &raw[idx + 1..];
            if zone.is_empty() {
                return None;
            }
            &raw[..idx]
        }
        None => raw,
    };
    base.parse::<Ipv6Addr>().ok()
}

fn redact_non_loopback_ipv6(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = String::with_capacity(input.len());
    let mut i = 0usize;

    while i < bytes.len() {
        // Case 1: Bracketed IPv6 literal, e.g. [2001:db8::1]:443 or [::1]:8080
        if bytes[i] == b'['
            && let Some(rel_close) = input[i + 1..].find(']')
        {
            let close_idx = i + 1 + rel_close;
            let inner = &input[i + 1..close_idx];
            if inner.contains(':')
                && let Some(addr) = parse_ipv6_with_optional_zone(inner)
            {
                if is_loopback_ipv6(&addr) {
                    out.push_str(&input[i..=close_idx]);
                } else {
                    out.push_str(REDACTED_IP);
                }
                i = close_idx + 1;
                continue;
            }
        }

        // Case 2: Unbracketed IPv6 literal
        let valid_start_boundary = i == 0
            || !(bytes[i - 1].is_ascii_alphanumeric()
                || bytes[i - 1] == b':'
                || bytes[i - 1] == b'.'
                || bytes[i - 1] == b'_');

        if valid_start_boundary && (bytes[i].is_ascii_hexdigit() || bytes[i] == b':') {
            let mut end = i;
            let mut seen_percent = false;
            while end < bytes.len() {
                let b = bytes[end];
                if !seen_percent {
                    if b.is_ascii_hexdigit() || b == b':' || b == b'.' {
                        end += 1;
                    } else if b == b'%' {
                        seen_percent = true;
                        end += 1;
                    } else {
                        break;
                    }
                } else if b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.') {
                    end += 1;
                } else {
                    break;
                }
            }

            // Trim trailing punctuation '.' or single trailing ':' (unless '::')
            let mut trimmed_end = end;
            while trimmed_end > i {
                let last = bytes[trimmed_end - 1];
                if last == b'.' {
                    trimmed_end -= 1;
                } else if last == b':'
                    && !(trimmed_end >= i + 2 && bytes[trimmed_end - 2] == b':')
                {
                    trimmed_end -= 1;
                } else {
                    break;
                }
            }

            let valid_end_boundary = trimmed_end == bytes.len()
                || !(bytes[trimmed_end].is_ascii_alphanumeric() || bytes[trimmed_end] == b'_');

            let candidate = &input[i..trimmed_end];
            if valid_end_boundary
                && candidate.bytes().filter(|&b| b == b':').count() >= 2
                && let Some(addr) = parse_ipv6_with_optional_zone(candidate)
            {
                if is_loopback_ipv6(&addr) {
                    out.push_str(candidate);
                } else {
                    out.push_str(REDACTED_IP);
                }
                i = trimmed_end;
                continue;
            }
        }

        let ch = input[i..].chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }

    out
}

fn redact_non_loopback_ipv4(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = String::with_capacity(input.len());
    let mut i = 0usize;

    while i < bytes.len() {
        let valid_start_boundary = i == 0
            || !(bytes[i - 1].is_ascii_alphanumeric()
                || bytes[i - 1] == b'.'
                || bytes[i - 1] == b'_');

        if valid_start_boundary && bytes[i].is_ascii_digit() {
            let mut end = i;
            while end < bytes.len() && (bytes[end].is_ascii_digit() || bytes[end] == b'.') {
                end += 1;
            }

            let mut trimmed_end = end;
            while trimmed_end > i && bytes[trimmed_end - 1] == b'.' {
                trimmed_end -= 1;
            }

            let valid_end_boundary = trimmed_end == bytes.len()
                || !(bytes[trimmed_end].is_ascii_alphanumeric() || bytes[trimmed_end] == b'_');

            let candidate = &input[i..trimmed_end];
            if valid_end_boundary
                && candidate.bytes().filter(|&b| b == b'.').count() == 3
                && let Ok(addr) = candidate.parse::<Ipv4Addr>()
            {
                if addr.is_loopback() {
                    out.push_str(candidate);
                } else {
                    out.push_str(REDACTED_IP);
                }
                i = trimmed_end;
                continue;
            }
        }

        let ch = input[i..].chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }

    out
}

fn redact_long_hex_and_base64_secrets(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = String::with_capacity(input.len());
    let mut i = 0usize;

    while i < bytes.len() {
        if is_secret_token_char(bytes[i]) {
            let start = i;
            while i < bytes.len() && is_secret_token_char(bytes[i]) {
                i += 1;
            }
            let token = &input[start..i];
            if is_long_hex_secret(token) || is_long_base64_secret(token) {
                out.push_str(REDACTED_SECRET);
            } else {
                out.push_str(token);
            }
        } else {
            let ch = input[i..].chars().next().unwrap();
            out.push(ch);
            i += ch.len_utf8();
        }
    }

    out
}

fn is_secret_token_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'+' | b'/' | b'=' | b'_' | b'-')
}

fn is_long_hex_secret(token: &str) -> bool {
    let hex = token
        .strip_prefix("0x")
        .or_else(|| token.strip_prefix("0X"))
        .unwrap_or(token);
    hex.len() >= 32 && hex.bytes().all(|b| b.is_ascii_hexdigit())
}

fn is_long_base64_secret(token: &str) -> bool {
    if token.len() < 32 || token.starts_with('/') {
        return false;
    }

    // Avoid redacting filesystem-like relative paths such as `remote_core/src/diagnostics_bundle`.
    if token.contains('/') {
        let segments: Vec<&str> = token.split('/').collect();
        if segments.len() >= 2
            && segments.iter().all(|seg| {
                !seg.is_empty()
                    && seg
                        .bytes()
                        .all(|b| b.is_ascii_lowercase() || b == b'_' || b == b'-')
            })
        {
            return false;
        }
    }

    let has_upper = token.bytes().any(|b| b.is_ascii_uppercase());
    let has_lower = token.bytes().any(|b| b.is_ascii_lowercase());
    let has_digit = token.bytes().any(|b| b.is_ascii_digit());
    let has_b64_sym = token.bytes().any(|b| matches!(b, b'+' | b'/' | b'='));
    let ends_with_pad = token.ends_with('=');

    if ends_with_pad && (has_upper || has_lower) && (has_digit || has_b64_sym || (has_upper && has_lower)) {
        return true;
    }

    let class_count = u8::from(has_upper)
        + u8::from(has_lower)
        + u8::from(has_digit)
        + u8::from(has_b64_sym);

    token.len() >= 40 && class_count >= 3
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_bundle() -> DiagnosticBundle {
        DiagnosticBundle {
            schema_version: DIAGNOSTIC_BUNDLE_SCHEMA_VERSION,
            generated_at_unix_ms: 1_763_629_542_000,
            app_version: "2.0.0-alpha.6".to_string(),
            platform: "macos-aarch64".to_string(),
            transport: DiagnosticTransportSummary {
                route_mode: "p2p_direct".to_string(),
                active_scope: "lan".to_string(),
                relay_connected: true,
                p2p_state: "connected".to_string(),
                rtt_ms: 4.25,
                packet_loss_percent: 0.1,
                fps: 60.0,
                video_bitrate_kbps: 18_500,
            },
            session: DiagnosticSessionSummary {
                encoder_profile: "hevc_main10".to_string(),
                chroma: "4:2:0".to_string(),
                hdr_mode: "pq10".to_string(),
                virtual_displays_active: 2,
                port_forwards_active: 1,
                privacy_mode_active: true,
            },
            recent_events: vec![
                "p2p connected via 127.0.0.1:9000".to_string(),
                "encoder initialized: hevc_main10 60fps".to_string(),
            ],
        }
    }

    #[test]
    fn redacts_bearer_oauth_basic_jwt_and_provider_tokens() {
        let jwt = "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiIxMjM0NTY3ODkwIiwibmFtZSI6IkFsaWNlIn0.SflKxwRJSMeKKF2QT4fwpMeJf36POk6yJV_adQssw5c";
        let input = format!(
            "Authorization: Bearer abc123.def456-ghi789\n\
             header OAuth ya29.a0AfH6SMBxSecretOAuthToken999\n\
             Authorization: Basic dXNlcjpzZWNyZXRwYXNz\n\
             jwt={jwt}\n\
             standalone {jwt}\n\
             gh=ghp_1234567890abcdef1234567890abcdef1234"
        );
        let redacted = redact_sensitive_text(&input);

        assert!(!redacted.contains("abc123.def456-ghi789"));
        assert!(!redacted.contains("ya29.a0AfH6SMBxSecretOAuthToken999"));
        assert!(!redacted.contains("dXNlcjpzZWNyZXRwYXNz"));
        assert!(!redacted.contains("eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9"));
        assert!(!redacted.contains("ghp_1234567890abcdef"));
        assert!(redacted.contains("Authorization: Bearer [REDACTED_TOKEN]"));
        assert!(redacted.contains("header OAuth [REDACTED_TOKEN]"));
        assert!(redacted.contains("Authorization: Basic [REDACTED_TOKEN]"));
    }

    #[test]
    fn redacts_key_value_passwords_secrets_tokens_and_keys() {
        let input = r#"connect password=mySecretPass123 secret="top_secret_val" token='tok_98765' api_key: kf_live_12345 {"client_secret": "s3cr3t!", "relay_token": "r_tok_42", "psk": "deadbeef"} https://alice:hunter2@relay.example.com/ws"#;
        let redacted = redact_sensitive_text(input);

        for leaked in [
            "mySecretPass123",
            "top_secret_val",
            "tok_98765",
            "kf_live_12345",
            "s3cr3t!",
            "r_tok_42",
            "deadbeef",
            "hunter2",
        ] {
            assert!(
                !redacted.contains(leaked),
                "expected `{leaked}` to be redacted in: {redacted}"
            );
        }

        assert!(redacted.contains("password=[REDACTED_SECRET]"));
        assert!(redacted.contains("secret=\"[REDACTED_SECRET]\""));
        assert!(redacted.contains("token='[REDACTED_SECRET]'"));
        assert!(redacted.contains("api_key: [REDACTED_SECRET]"));
        assert!(redacted.contains("\"client_secret\": \"[REDACTED_SECRET]\""));
        assert!(redacted.contains("https://alice:[REDACTED_SECRET]@relay.example.com/ws"));
    }

    #[test]
    fn redacts_pem_blocks_complete_and_truncated() {
        let input = "before\n-----BEGIN PRIVATE KEY-----\nMIIEvQIBADANBgkqhkiG9w0BAQEFAASCBKcwggSjAgEAAoIBAQC7\n-----END PRIVATE KEY-----\nafter";
        let redacted = redact_sensitive_text(input);
        assert_eq!(redacted, "before\n[REDACTED_PEM]\nafter");

        let truncated = "cert: -----BEGIN RSA PRIVATE KEY-----\nMIIEowIBAAKCAQEA1234567890";
        let redacted_truncated = redact_sensitive_text(truncated);
        assert_eq!(redacted_truncated, "cert: [REDACTED_PEM]");
    }

    #[test]
    fn redacts_long_hex_and_base64_secrets_without_corrupting_normal_text() {
        let hex_key = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        let b64_key = "SGVsbG8gV29ybGQgVGhpcyBpcyBhIHNlY3JldCBrZXk=";
        let b64url_key = "AbCdEfGhIjKlMnOpQrStUvWxYz0123456789_-AbCdE";
        let input = format!(
            "hex={hex_key} raw_hex {hex_key} b64 {b64_key} b64url {b64url_key} \
             path /Users/dev/remote_core/src/diagnostics_bundle.rs \
             ident virtual_displays_active_count_threshold_exceeded \
             version 2.0.0-alpha.6"
        );
        let redacted = redact_sensitive_text(&input);

        assert!(!redacted.contains(hex_key));
        assert!(!redacted.contains(b64_key));
        assert!(!redacted.contains(b64url_key));
        assert!(redacted.contains("/Users/dev/remote_core/src/diagnostics_bundle.rs"));
        assert!(redacted.contains("virtual_displays_active_count_threshold_exceeded"));
        assert!(redacted.contains("2.0.0-alpha.6"));
    }

    #[test]
    fn redacts_non_loopback_ipv4_and_ipv6_while_preserving_loopback() {
        let input = "local=127.0.0.1:8080 local2=127.0.1.1 local6=[::1]:9000 plain6=::1 mapped=::ffff:127.0.0.1 \
                     lan=192.168.1.42:5000 wan=203.0.113.99 cgnat=100.64.0.5 \
                     v6=2001:db8:85a3::8a2e:370:7334 bracket=[fe80::1%en0]:443 mapped_wan=::ffff:198.51.100.7 \
                     module=remote_core::diagnostics_bundle time=12:34:56";
        let redacted = redact_sensitive_text(input);

        // Loopback preserved
        assert!(redacted.contains("127.0.0.1:8080"));
        assert!(redacted.contains("127.0.1.1"));
        assert!(redacted.contains("[::1]:9000"));
        assert!(redacted.contains("plain6=::1"));
        assert!(redacted.contains("mapped=::ffff:127.0.0.1"));

        // Non-IP colon constructs preserved
        assert!(redacted.contains("remote_core::diagnostics_bundle"));
        assert!(redacted.contains("time=12:34:56"));

        // Non-loopback IPs stripped
        for ip in [
            "192.168.1.42",
            "203.0.113.99",
            "100.64.0.5",
            "2001:db8:85a3::8a2e:370:7334",
            "fe80::1",
            "198.51.100.7",
        ] {
            assert!(
                !redacted.contains(ip),
                "expected non-loopback IP `{ip}` to be redacted in: {redacted}"
            );
        }
        assert!(redacted.contains("lan=[REDACTED_IP]:5000"));
        assert!(redacted.contains("bracket=[REDACTED_IP]:443"));
    }

    #[test]
    fn json_roundtrip_and_bundle_redaction() {
        let mut bundle = sample_bundle();
        bundle.transport.p2p_state = "connected peer=203.0.113.77".to_string();
        bundle.recent_events.push(
            "relay auth Authorization: Bearer super_secret_token_xyz from 10.20.30.40".to_string(),
        );

        let json = bundle.to_redacted_json().expect("serialize bundle");
        assert!(json.len() <= MAX_DIAGNOSTIC_BUNDLE_BYTES);
        assert!(!json.contains("203.0.113.77"));
        assert!(!json.contains("super_secret_token_xyz"));
        assert!(!json.contains("10.20.30.40"));
        assert!(json.contains("127.0.0.1:9000"));

        let decoded: DiagnosticBundle =
            serde_json::from_str(&json).expect("deserialize redacted bundle");
        assert_eq!(decoded.schema_version, DIAGNOSTIC_BUNDLE_SCHEMA_VERSION);
        assert_eq!(decoded.generated_at_unix_ms, bundle.generated_at_unix_ms);
        assert_eq!(decoded.app_version, "2.0.0-alpha.6");
        assert_eq!(decoded.platform, "macos-aarch64");
        assert_eq!(decoded.transport.route_mode, "p2p_direct");
        assert_eq!(decoded.transport.p2p_state, "connected peer=[REDACTED_IP]");
        assert_eq!(decoded.transport.video_bitrate_kbps, 18_500);
        assert_eq!(decoded.session.virtual_displays_active, 2);
        assert_eq!(decoded.session.port_forwards_active, 1);
        assert!(decoded.session.privacy_mode_active);
        assert_eq!(decoded.recent_events.len(), 3);
    }

    #[test]
    fn enforces_max_bundle_bytes_by_truncating_oldest_events_first() {
        let mut bundle = sample_bundle();
        bundle.recent_events = (0..1500)
            .map(|idx| format!("event-{idx:04}: {}", "x".repeat(256)))
            .collect();

        let json = bundle.to_redacted_json().expect("serialize bounded bundle");
        assert!(
            json.len() <= MAX_DIAGNOSTIC_BUNDLE_BYTES,
            "bundle JSON size {} exceeded limit {}",
            json.len(),
            MAX_DIAGNOSTIC_BUNDLE_BYTES
        );

        let decoded: DiagnosticBundle =
            serde_json::from_str(&json).expect("deserialize bounded bundle");
        assert!(!decoded.recent_events.is_empty());
        assert!(decoded.recent_events.len() < 1500);
        // Oldest event dropped; newest event preserved.
        assert!(!decoded.recent_events[0].starts_with("event-0000:"));
        assert!(
            decoded
                .recent_events
                .last()
                .unwrap()
                .starts_with("event-1499:")
        );
    }

    #[test]
    fn handles_non_finite_metrics_utf8_truncation_and_idempotency() {
        let mut bundle = sample_bundle();
        bundle.transport.rtt_ms = f32::NAN;
        bundle.transport.packet_loss_percent = f32::INFINITY;
        bundle.transport.fps = f32::NEG_INFINITY;
        bundle.platform = "测试平台🚀".repeat(80_000); // > 256 KiB multi-byte UTF-8 metadata

        let json = bundle
            .to_redacted_json()
            .expect("serialize extreme bundle");
        assert!(json.len() <= MAX_DIAGNOSTIC_BUNDLE_BYTES);
        let decoded: DiagnosticBundle =
            serde_json::from_str(&json).expect("roundtrip extreme bundle");
        assert_eq!(decoded.transport.rtt_ms, 0.0);
        assert_eq!(decoded.transport.packet_loss_percent, 0.0);
        assert_eq!(decoded.transport.fps, 0.0);

        // Idempotency & edge cases (multiple PEMs, CIDR, escaped JSON quotes, version strings)
        let tricky = r#"{"PASSWORD":"pa\"ss\\123","X-API-KEY":"k_999"} 192.168.0.1/24 2001:db8::1/64 [::]:8080 v1.2.3.4 1.2.3.4.5
-----BEGIN CERTIFICATE-----
AAAABBBBCCCC
-----END CERTIFICATE-----
-----BEGIN EC PRIVATE KEY-----
DDDDEEEEFFFF
-----END EC PRIVATE KEY-----"#;
        let once = redact_sensitive_text(tricky);
        let twice = redact_sensitive_text(&once);
        assert_eq!(once, twice, "redaction must be idempotent");
        assert!(!once.contains("pa\\\"ss"));
        assert!(!once.contains("k_999"));
        assert!(!once.contains("192.168.0.1"));
        assert!(!once.contains("2001:db8::1"));
        assert!(!once.contains("[::]:8080"));
        assert!(!once.contains("AAAABBBBCCCC"));
        assert!(!once.contains("DDDDEEEEFFFF"));
        assert!(once.contains("[REDACTED_IP]/24"));
        assert!(once.contains("[REDACTED_IP]/64"));
        assert!(once.contains("[REDACTED_IP]:8080"));
        assert!(once.contains("v1.2.3.4"));
        assert!(once.contains("1.2.3.4.5"));
    }
}
