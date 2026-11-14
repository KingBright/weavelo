use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use thiserror::Error;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio_rustls::rustls::pki_types::ServerName;
use tokio_rustls::rustls::{ClientConfig, RootCertStore};
use tokio_rustls::TlsConnector;
use url::Url;

pub const PSN_AUTH_HOST: &str = "auth.api.sonyentertainmentnetwork.com";
pub const PSN_CLIENT_ID: &str = "ba495a24-818c-472b-b12d-ff231c1b5745";
pub const PSN_CLIENT_SECRET: &str = "mvaiZkRsAsI1IBkY";
pub const PSN_REDIRECT_URI: &str = "https://remoteplay.dl.playstation.net/remoteplay/redirect";

#[derive(Debug, Error)]
pub enum PsnAuthError {
    #[error("invalid PSN OAuth input: expected redirect URL, authorization code, or 8-byte Base64 AccountID")]
    InvalidInput,
    #[error("network/TLS error: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid TLS server name: {0}")]
    InvalidServerName(String),
    #[error("malformed HTTP response from Sony OAuth server")]
    MalformedHttp,
    #[error("HTTP {status} from Sony OAuth server: {body}")]
    HttpStatus { status: u16, body: String },
    #[error("Sony OAuth API error: {0}")]
    ApiError(String),
    #[error("JSON parse error: {0}")]
    Json(#[from] serde_json::Error),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PsnAccount {
    pub user_id: Option<u64>,
    pub online_id: Option<String>,
    pub account_id: [u8; 8],
    pub account_id_b64: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParsedAuthInput {
    OAuthCode(String),
    DirectAccountId([u8; 8]),
}

/// Official Sony Remote Play OAuth authorization URL suitable for QR code rendering or opening in a browser.
pub fn psn_authorize_url() -> String {
    format!(
        "https://{host}/2.0/oauth/authorize?service_entity=urn:service-entity:psn&response_type=code&client_id={client_id}&redirect_uri={redirect_uri}&scope=psn:clientapp&request_locale=en_US&ui=pr&service_logo=ps&layout_type=popup&smcid=remoteplay&prompt=always&PlatformPrivacyWs1=minimal&",
        host = PSN_AUTH_HOST,
        client_id = PSN_CLIENT_ID,
        redirect_uri = PSN_REDIRECT_URI,
    )
}

/// Parses a user-pasted Sony redirect URL, raw OAuth `code`, or 12-char Base64 8-byte `AccountID`.
pub fn parse_psn_auth_input(input: &str) -> Result<ParsedAuthInput, PsnAuthError> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err(PsnAuthError::InvalidInput);
    }

    if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        let url = Url::parse(trimmed).map_err(|_| PsnAuthError::InvalidInput)?;
        for (k, v) in url.query_pairs() {
            if k == "code" && !v.trim().is_empty() {
                return Ok(ParsedAuthInput::OAuthCode(v.trim().to_string()));
            }
        }
        return Err(PsnAuthError::InvalidInput);
    }

    if let Some(rest) = trimmed.strip_prefix("code=") {
        let code = rest.split('&').next().unwrap_or("").trim();
        if code.is_empty() {
            return Err(PsnAuthError::InvalidInput);
        }
        return Ok(ParsedAuthInput::OAuthCode(code.to_string()));
    }

    // Standard Base64 of an 8-byte AccountID is always 12 chars ending with `=`.
    if trimmed.len() == 12 && trimmed.ends_with('=') {
        if let Ok(decoded) = BASE64_STANDARD.decode(trimmed) {
            if let Ok(arr) = <[u8; 8]>::try_from(decoded.as_slice()) {
                return Ok(ParsedAuthInput::DirectAccountId(arr));
            }
        }
    }

    Ok(ParsedAuthInput::OAuthCode(trimmed.to_string()))
}

/// Decodes a Base64-encoded 8-byte PSN AccountID.
pub fn decode_account_id_b64(b64: &str) -> Result<[u8; 8], PsnAuthError> {
    let decoded = BASE64_STANDARD
        .decode(b64.trim())
        .map_err(|_| PsnAuthError::InvalidInput)?;
    <[u8; 8]>::try_from(decoded.as_slice()).map_err(|_| PsnAuthError::InvalidInput)
}

/// Encodes a numeric PSN `user_id` (`u64`) to its little-endian 8-byte `AccountID` and Base64 representation.
pub fn account_from_user_id(user_id: u64, online_id: Option<String>) -> PsnAccount {
    let account_id = user_id.to_le_bytes();
    let account_id_b64 = BASE64_STANDARD.encode(account_id);
    PsnAccount {
        user_id: Some(user_id),
        online_id,
        account_id,
        account_id_b64,
    }
}

/// Exchanges a pasted redirect URL, OAuth code, or direct Base64 `AccountID` for a `PsnAccount`.
pub async fn resolve_psn_account(input: &str) -> Result<PsnAccount, PsnAuthError> {
    match parse_psn_auth_input(input)? {
        ParsedAuthInput::DirectAccountId(account_id) => Ok(PsnAccount {
            user_id: Some(u64::from_le_bytes(account_id)),
            online_id: None,
            account_id,
            account_id_b64: BASE64_STANDARD.encode(account_id),
        }),
        ParsedAuthInput::OAuthCode(code) => {
            exchange_oauth_code_over_host(PSN_AUTH_HOST, 443, &code, true).await
        }
    }
}

/// Exchanges a Sony OAuth authorization code against a TLS host (overrideable for testing).
pub async fn exchange_oauth_code_over_host(
    host: &str,
    port: u16,
    code: &str,
    use_tls: bool,
) -> Result<PsnAccount, PsnAuthError> {
    let basic = BASE64_STANDARD.encode(format!("{}:{}", PSN_CLIENT_ID, PSN_CLIENT_SECRET));
    let form_body = format!(
        "grant_type=authorization_code&code={}&redirect_uri={}&",
        urlencoding::encode(code),
        urlencoding::encode(PSN_REDIRECT_URI),
    );
    let token_req = format!(
        "POST /2.0/oauth/token HTTP/1.1\r\n\
         Host: {host}\r\n\
         Authorization: Basic {basic}\r\n\
         Content-Type: application/x-www-form-urlencoded\r\n\
         Content-Length: {len}\r\n\
         Connection: close\r\n\r\n\
         {body}",
        host = host,
        basic = basic,
        len = form_body.len(),
        body = form_body,
    );

    let (status, body) = send_http_request(host, port, use_tls, token_req.as_bytes()).await?;
    let json: serde_json::Value = serde_json::from_slice(&body).map_err(|e| {
        if !(200..300).contains(&status) {
            PsnAuthError::HttpStatus {
                status,
                body: String::from_utf8_lossy(&body).to_string(),
            }
        } else {
            PsnAuthError::Json(e)
        }
    })?;

    if !(200..300).contains(&status) {
        let msg = json
            .get("error_description")
            .or_else(|| json.get("error"))
            .and_then(|v| v.as_str())
            .map(ToString::to_string)
            .unwrap_or_else(|| String::from_utf8_lossy(&body).to_string());
        return Err(PsnAuthError::HttpStatus { status, body: msg });
    }

    let access_token = json
        .get("access_token")
        .and_then(|v| v.as_str())
        .ok_or_else(|| PsnAuthError::ApiError("missing access_token in response".into()))?;

    let info_req = format!(
        "GET /2.0/oauth/token/{token} HTTP/1.1\r\n\
         Host: {host}\r\n\
         Authorization: Basic {basic}\r\n\
         Connection: close\r\n\r\n",
        token = urlencoding::encode(access_token),
        host = host,
        basic = basic,
    );

    let (info_status, info_body) =
        send_http_request(host, port, use_tls, info_req.as_bytes()).await?;
    let info_json: serde_json::Value = serde_json::from_slice(&info_body).map_err(|e| {
        if !(200..300).contains(&info_status) {
            PsnAuthError::HttpStatus {
                status: info_status,
                body: String::from_utf8_lossy(&info_body).to_string(),
            }
        } else {
            PsnAuthError::Json(e)
        }
    })?;

    if !(200..300).contains(&info_status) {
        return Err(PsnAuthError::HttpStatus {
            status: info_status,
            body: String::from_utf8_lossy(&info_body).to_string(),
        });
    }

    let user_id = match info_json.get("user_id") {
        Some(serde_json::Value::String(s)) => s
            .trim()
            .parse::<u64>()
            .map_err(|_| PsnAuthError::ApiError(format!("invalid user_id string: {s}")))?,
        Some(serde_json::Value::Number(n)) => n
            .as_u64()
            .ok_or_else(|| PsnAuthError::ApiError("user_id number is not a u64".into()))?,
        _ => return Err(PsnAuthError::ApiError("missing user_id in response".into())),
    };

    let online_id = info_json
        .get("online_id")
        .and_then(|v| v.as_str())
        .map(ToString::to_string);

    Ok(account_from_user_id(user_id, online_id))
}

fn build_tls_connector() -> TlsConnector {
    let roots = RootCertStore::from_iter(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    let config =
        ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
            .with_safe_default_protocol_versions()
            .expect("valid TLS versions")
            .with_root_certificates(roots)
            .with_no_client_auth();
    TlsConnector::from(Arc::new(config))
}

async fn send_http_request(
    host: &str,
    port: u16,
    use_tls: bool,
    request_bytes: &[u8],
) -> Result<(u16, Vec<u8>), PsnAuthError> {
    let tcp = TcpStream::connect((host, port)).await?;
    if use_tls {
        let server_name = ServerName::try_from(host.to_string())
            .map_err(|_| PsnAuthError::InvalidServerName(host.to_string()))?;
        let connector = build_tls_connector();
        let tls = connector.connect(server_name, tcp).await?;
        write_and_read_http(tls, request_bytes).await
    } else {
        write_and_read_http(tcp, request_bytes).await
    }
}

async fn write_and_read_http<S: AsyncRead + AsyncWrite + Unpin>(
    mut stream: S,
    request_bytes: &[u8],
) -> Result<(u16, Vec<u8>), PsnAuthError> {
    stream.write_all(request_bytes).await?;
    stream.flush().await?;

    let mut raw = Vec::new();
    stream.read_to_end(&mut raw).await?;
    parse_http_response(&raw)
}

pub fn parse_http_response(raw: &[u8]) -> Result<(u16, Vec<u8>), PsnAuthError> {
    let sep = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .ok_or(PsnAuthError::MalformedHttp)?;
    let header_bytes = &raw[..sep];
    let body_raw = &raw[sep + 4..];

    let header_text = std::str::from_utf8(header_bytes).map_err(|_| PsnAuthError::MalformedHttp)?;
    let mut lines = header_text.split("\r\n");
    let status_line = lines.next().ok_or(PsnAuthError::MalformedHttp)?;
    let status: u16 = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .ok_or(PsnAuthError::MalformedHttp)?;

    let mut is_chunked = false;
    let mut content_length: Option<usize> = None;
    for line in lines {
        if let Some((k, v)) = line.split_once(':') {
            let key = k.trim();
            let val = v.trim();
            if key.eq_ignore_ascii_case("transfer-encoding")
                && val.to_ascii_lowercase().contains("chunked")
            {
                is_chunked = true;
            } else if key.eq_ignore_ascii_case("content-length") {
                if let Ok(len) = val.parse::<usize>() {
                    content_length = Some(len);
                }
            }
        }
    }

    let body = if is_chunked {
        decode_chunked_body(body_raw)?
    } else if let Some(len) = content_length {
        if body_raw.len() < len {
            return Err(PsnAuthError::MalformedHttp);
        }
        body_raw[..len].to_vec()
    } else {
        body_raw.to_vec()
    };

    Ok((status, body))
}

fn decode_chunked_body(mut data: &[u8]) -> Result<Vec<u8>, PsnAuthError> {
    let mut out = Vec::new();
    loop {
        let Some(crlf) = data.windows(2).position(|w| w == b"\r\n") else {
            return Err(PsnAuthError::MalformedHttp);
        };
        let size_line =
            std::str::from_utf8(&data[..crlf]).map_err(|_| PsnAuthError::MalformedHttp)?;
        let size_hex = size_line.split(';').next().unwrap_or("").trim();
        let size = usize::from_str_radix(size_hex, 16).map_err(|_| PsnAuthError::MalformedHttp)?;
        data = &data[crlf + 2..];
        if size == 0 {
            break;
        }
        if data.len() < size + 2 {
            return Err(PsnAuthError::MalformedHttp);
        }
        out.extend_from_slice(&data[..size]);
        data = &data[size + 2..];
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::TcpListener;

    #[test]
    fn parses_redirect_url_code_and_direct_b64() {
        let url = "https://remoteplay.dl.playstation.net/remoteplay/redirect?code=v3.AbCdEf123456&cid=xyz";
        assert_eq!(
            parse_psn_auth_input(url).unwrap(),
            ParsedAuthInput::OAuthCode("v3.AbCdEf123456".to_string())
        );

        assert_eq!(
            parse_psn_auth_input("code=my_code&state=1").unwrap(),
            ParsedAuthInput::OAuthCode("my_code".to_string())
        );

        let expected = [0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88];
        let b64 = BASE64_STANDARD.encode(expected);
        assert_eq!(
            parse_psn_auth_input(&b64).unwrap(),
            ParsedAuthInput::DirectAccountId(expected)
        );
    }

    #[test]
    fn user_id_to_le_account_id() {
        let acct = account_from_user_id(0x0102_0304_0506_0708, Some("Player".to_string()));
        assert_eq!(
            acct.account_id,
            [0x08, 0x07, 0x06, 0x05, 0x04, 0x03, 0x02, 0x01]
        );
        assert_eq!(
            decode_account_id_b64(&acct.account_id_b64).unwrap(),
            acct.account_id
        );
    }

    #[test]
    fn decodes_chunked_http_response() {
        let raw = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n5\r\nhello\r\n6\r\n world\r\n0\r\n\r\n";
        let (status, body) = parse_http_response(raw).unwrap();
        assert_eq!(status, 200);
        assert_eq!(body, b"hello world");
    }

    #[tokio::test]
    async fn exchanges_code_against_local_mock_server() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let server = tokio::spawn(async move {
            // 1. POST /2.0/oauth/token
            let (mut s1, _) = listener.accept().await.unwrap();
            let mut buf = [0u8; 2048];
            let n = s1.read(&mut buf).await.unwrap();
            let req1 = std::str::from_utf8(&buf[..n]).unwrap();
            assert!(req1.starts_with("POST /2.0/oauth/token HTTP/1.1"));
            assert!(req1.contains("code=test_code_123"));
            let body1 = "{\"access_token\":\"tok_abc_456\"}";
            let resp1 = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body1.len(),
                body1
            );
            s1.write_all(resp1.as_bytes()).await.unwrap();
            s1.shutdown().await.unwrap();
            drop(s1);

            // 2. GET /2.0/oauth/token/tok_abc_456
            let (mut s2, _) = listener.accept().await.unwrap();
            let n2 = s2.read(&mut buf).await.unwrap();
            let req2 = std::str::from_utf8(&buf[..n2]).unwrap();
            assert!(req2.starts_with("GET /2.0/oauth/token/tok_abc_456 HTTP/1.1"));
            let json = r#"{"user_id":"123456789012345","online_id":"DualSenseFan"}"#;
            let chunk = format!(
                "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n{:x}\r\n{}\r\n0\r\n\r\n",
                json.len(),
                json
            );
            s2.write_all(chunk.as_bytes()).await.unwrap();
            s2.shutdown().await.unwrap();
            drop(s2);
        });

        let acct = exchange_oauth_code_over_host(
            &addr.ip().to_string(),
            addr.port(),
            "test_code_123",
            false,
        )
        .await
        .unwrap();
        assert_eq!(acct.user_id, Some(123456789012345));
        assert_eq!(acct.online_id.as_deref(), Some("DualSenseFan"));
        assert_eq!(acct.account_id, 123456789012345u64.to_le_bytes());
        server.await.unwrap();
    }
}
