use std::collections::HashMap;
use std::net::IpAddr;
use std::time::Duration;

use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use rand::Rng;
use thiserror::Error;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::{mpsc, watch};
use tokio::time::{timeout, Instant};

use crate::rpcrypt::Rpcrypt;

/// Default TCP port used by PS5 Remote Play control & registration.
pub const CTRL_PORT: u16 = 9295;

const HTTP_TIMEOUT: Duration = Duration::from_secs(10);
const SESSION_ID_TIMEOUT: Duration = Duration::from_secs(15);
const MAX_CTRL_PAYLOAD_SIZE: usize = 65_536;

// Control binary message types (host/ps5 wire protocol).
const MSG_LOGIN_PIN_REQ: u16 = 0x0004;
const MSG_SESSION_ACK_11: u16 = 0x0011;
const MSG_SESSION_ACK_13: u16 = 0x0013;
const MSG_GO_HOME: u16 = 0x0014;
const MSG_KEYBOARD: u16 = 0x0032;
const MSG_SESSION_ID: u16 = 0x0033;
const MSG_MICROPHONE: u16 = 0x0036;
const MSG_STANDBY: u16 = 0x0050;
const MSG_HEARTBEAT_REQ: u16 = 0x00fe;
const MSG_HEARTBEAT_ACK: u16 = 0x01fe;
const MSG_LOGIN_PIN_RESP: u16 = 0x0804;
const MSG_SESSION_ACK_910: u16 = 0x0910;

const INIT_ACK_13_PAYLOAD: [u8; 3] = [0x00, 0x40, 0x00];
const INIT_ACK_11_PAYLOAD: [u8; 16] = [
    0xa0, 0xab, 0x51, 0xbd, 0xd1, 0x7e, 0x00, 0x00, 0xff, 0xff, 0xff, 0xff, 0xff, 0x00, 0x00,
    0x00,
];
const INIT_ACK_910_PAYLOAD: [u8; 4] = [0x00, 0x00, 0x00, 0x00];

#[derive(Debug, Error)]
pub enum CtrlError {
    #[error("I/O error during PS5 ctrl communication: {0}")]
    Io(#[from] std::io::Error),
    #[error("PS5 ctrl operation timed out: {0}")]
    Timeout(&'static str),
    #[error("Malformed HTTP response from PS5 ctrl endpoint: {0}")]
    HttpParse(String),
    #[error("PS5 ctrl HTTP error status {status}: reason={reason:?}")]
    HttpStatus { status: u16, reason: Option<String> },
    #[error("Missing or invalid header `{0}` in PS5 ctrl response")]
    InvalidHeader(&'static str),
    #[error("Unsupported console type {0}: only PS5 (RP-Server-Type == 2) is supported")]
    UnsupportedConsole(u8),
    #[error("PS5 console is requesting a login PIN before remote play can start")]
    LoginPinRequired,
    #[error("Invalid PS5 ctrl binary frame: {0}")]
    InvalidFrame(String),
    #[error("PS5 ctrl connection closed: {0}")]
    Closed(String),
}

/// Configuration required to connect to a PS5 ctrl session on TCP 9295.
#[derive(Debug, Clone)]
pub struct CtrlConfig {
    pub host: IpAddr,
    pub port: u16,
    pub rp_regist_key: [u8; 16],
    pub rp_key: [u8; 16],
    pub hdr: bool,
}

impl CtrlConfig {
    pub fn new(host: IpAddr, rp_regist_key: [u8; 16], rp_key: [u8; 16], hdr: bool) -> Self {
        Self {
            host,
            port: CTRL_PORT,
            rp_regist_key,
            rp_key,
            hdr,
        }
    }
}

#[derive(Debug)]
enum CtrlCommand {
    Standby,
    GoHome,
    SetMicrophone(bool),
    LoginPin(Vec<u8>),
    Shutdown,
}

/// Handle to an active PS5 TCP 9295 control channel.
#[derive(Clone)]
pub struct CtrlHandle {
    tx: mpsc::Sender<CtrlCommand>,
    closed_rx: watch::Receiver<Option<String>>,
}

impl CtrlHandle {
    /// Put the PS5 into rest/standby mode.
    pub async fn send_standby(&self) -> Result<(), CtrlError> {
        self.tx
            .send(CtrlCommand::Standby)
            .await
            .map_err(|_| CtrlError::Closed("control channel closed".into()))
    }

    /// Press the PlayStation Home action via ctrl message `0x14`.
    pub async fn send_go_home(&self) -> Result<(), CtrlError> {
        self.tx
            .send(CtrlCommand::GoHome)
            .await
            .map_err(|_| CtrlError::Closed("control channel closed".into()))
    }

    /// Toggle DualSense/remote microphone mute state via ctrl message `0x36`.
    pub async fn set_microphone(&self, unmuted: bool) -> Result<(), CtrlError> {
        self.tx
            .send(CtrlCommand::SetMicrophone(unmuted))
            .await
            .map_err(|_| CtrlError::Closed("control channel closed".into()))
    }

    /// Respond to a console login PIN prompt via ctrl message `0x804`.
    pub async fn send_login_pin(&self, pin_bytes: Vec<u8>) -> Result<(), CtrlError> {
        self.tx
            .send(CtrlCommand::LoginPin(pin_bytes))
            .await
            .map_err(|_| CtrlError::Closed("control channel closed".into()))
    }

    /// Gracefully close the TCP control connection.
    pub async fn shutdown(&self) {
        let _ = self.tx.send(CtrlCommand::Shutdown).await;
    }

    /// Returns `Some(reason)` once the control connection has terminated.
    pub fn closed_reason(&self) -> Option<String> {
        self.closed_rx.borrow().clone()
    }

    /// Wait until the control connection terminates.
    pub async fn wait_closed(&mut self) -> Option<String> {
        if let Some(reason) = self.closed_rx.borrow().clone() {
            return Some(reason);
        }
        while self.closed_rx.changed().await.is_ok() {
            if let Some(reason) = self.closed_rx.borrow().clone() {
                return Some(reason);
            }
        }
        Some("control channel dropped".into())
    }
}

/// Active PS5 ctrl session state wrapped before handing off the background loop.
pub struct CtrlSession {
    pub session_id: String,
    pub rpcrypt: Rpcrypt,
    pub handle: CtrlHandle,
}

#[derive(Debug)]
struct HttpResponseHead {
    status: u16,
    headers: HashMap<String, String>,
    leftover: Vec<u8>,
}

/// Format `RP-Registkey` for `/sie/ps5/rp/sess/init`: lowercase hex of `rp_regist_key`
/// truncated at the first `0x00` byte (matching `session.c:904-914`).
pub fn format_rp_registkey_header(rp_regist_key: &[u8; 16]) -> String {
    let end = rp_regist_key
        .iter()
        .position(|&b| b == 0)
        .unwrap_or(rp_regist_key.len());
    hex::encode(&rp_regist_key[..end])
}

/// Build the 32-byte `RP-Did` payload prior to encryption.
fn build_did_payload() -> [u8; 32] {
    let mut did = [0u8; 32];
    did[..10].copy_from_slice(&[0x00, 0x18, 0x00, 0x00, 0x00, 0x07, 0x00, 0x40, 0x00, 0x80]);
    rand::rng().fill_bytes(&mut did[10..26]);
    did
}

async fn read_http_head(stream: &mut TcpStream) -> Result<HttpResponseHead, CtrlError> {
    let mut buf = Vec::with_capacity(2048);
    let mut tmp = [0u8; 1024];
    let header_end = loop {
        if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break pos;
        }
        if buf.len() >= 16_384 {
            return Err(CtrlError::HttpParse(
                "HTTP response header exceeded 16 KiB".into(),
            ));
        }
        let n = timeout(HTTP_TIMEOUT, stream.read(&mut tmp))
            .await
            .map_err(|_| CtrlError::Timeout("reading HTTP response headers"))??;
        if n == 0 {
            return Err(CtrlError::HttpParse(
                "unexpected EOF before end of HTTP headers".into(),
            ));
        }
        buf.extend_from_slice(&tmp[..n]);
    };

    let head_bytes = &buf[..header_end];
    let leftover = buf[header_end + 4..].to_vec();
    let head_str = std::str::from_utf8(head_bytes)
        .map_err(|e| CtrlError::HttpParse(format!("non-UTF8 HTTP headers: {e}")))?;

    let mut lines = head_str.split("\r\n");
    let status_line = lines
        .next()
        .ok_or_else(|| CtrlError::HttpParse("empty HTTP response".into()))?;
    let mut status_parts = status_line.split_whitespace();
    let _version = status_parts
        .next()
        .ok_or_else(|| CtrlError::HttpParse(format!("bad status line: {status_line}")))?;
    let status_code = status_parts
        .next()
        .and_then(|s| s.parse::<u16>().ok())
        .ok_or_else(|| CtrlError::HttpParse(format!("invalid status code: {status_line}")))?;

    let mut headers = HashMap::new();
    for line in lines {
        if let Some((name, val)) = line.split_once(':') {
            headers.insert(name.trim().to_ascii_lowercase(), val.trim().to_string());
        }
    }

    Ok(HttpResponseHead {
        status: status_code,
        headers,
        leftover,
    })
}

/// Step 1: `GET /sie/ps5/rp/sess/init` to fetch the 16-byte `RP-Nonce`.
async fn fetch_session_nonce(config: &CtrlConfig) -> Result<[u8; 16], CtrlError> {
    let addr = std::net::SocketAddr::new(config.host, config.port);
    let mut stream = timeout(HTTP_TIMEOUT, TcpStream::connect(addr))
        .await
        .map_err(|_| CtrlError::Timeout("connecting for /sess/init"))??;
    let _ = stream.set_nodelay(true);

    let regist_hex = format_rp_registkey_header(&config.rp_regist_key);
    let req = format!(
        "GET /sie/ps5/rp/sess/init HTTP/1.1\r\n\
         Host: {}:{}\r\n\
         User-Agent: remoteplay Windows\r\n\
         Connection: close\r\n\
         RP-Registkey: {}\r\n\
         RP-Version: 1.0\r\n\
         \r\n",
        config.host, config.port, regist_hex
    );

    timeout(HTTP_TIMEOUT, stream.write_all(req.as_bytes()))
        .await
        .map_err(|_| CtrlError::Timeout("writing /sess/init request"))??;

    let head = read_http_head(&mut stream).await?;
    if head.status != 200 {
        return Err(CtrlError::HttpStatus {
            status: head.status,
            reason: head.headers.get("rp-application-reason").cloned(),
        });
    }

    let nonce_b64 = head
        .headers
        .get("rp-nonce")
        .ok_or(CtrlError::InvalidHeader("RP-Nonce"))?;
    let nonce_bytes = B64
        .decode(nonce_b64.as_bytes())
        .map_err(|_| CtrlError::InvalidHeader("RP-Nonce"))?;
    let nonce: [u8; 16] = nonce_bytes
        .try_into()
        .map_err(|_| CtrlError::InvalidHeader("RP-Nonce"))?;
    Ok(nonce)
}

async fn read_ctrl_frame(
    stream: &mut TcpStream,
    leftover: &mut Vec<u8>,
    rpcrypt: &Rpcrypt,
    remote_counter: &mut u64,
) -> Result<(u16, Vec<u8>), CtrlError> {
    while leftover.len() < 8 {
        let mut tmp = [0u8; 2048];
        let n = stream.read(&mut tmp).await?;
        if n == 0 {
            return Err(CtrlError::Closed("EOF reading ctrl header".into()));
        }
        leftover.extend_from_slice(&tmp[..n]);
    }

    let payload_size = u32::from_be_bytes(leftover[0..4].try_into().unwrap()) as usize;
    let msg_type = u16::from_be_bytes(leftover[4..6].try_into().unwrap());
    leftover.drain(..8);

    if payload_size > MAX_CTRL_PAYLOAD_SIZE {
        return Err(CtrlError::InvalidFrame(format!(
            "ctrl frame payload size {payload_size} exceeds limit"
        )));
    }

    while leftover.len() < payload_size {
        let mut tmp = [0u8; 4096];
        let n = stream.read(&mut tmp).await?;
        if n == 0 {
            return Err(CtrlError::Closed("EOF reading ctrl payload".into()));
        }
        leftover.extend_from_slice(&tmp[..n]);
    }

    let payload = if payload_size == 0 {
        Vec::new()
    } else {
        let enc: Vec<u8> = leftover.drain(..payload_size).collect();
        let ctr = *remote_counter;
        *remote_counter = remote_counter.wrapping_add(1);
        rpcrypt.decrypt(ctr, &enc)
    };

    Ok((msg_type, payload))
}

async fn write_ctrl_frame(
    stream: &mut TcpStream,
    rpcrypt: &Rpcrypt,
    local_counter: &mut u64,
    msg_type: u16,
    payload: &[u8],
) -> Result<(), CtrlError> {
    let mut frame = Vec::with_capacity(8 + payload.len());
    frame.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    frame.extend_from_slice(&msg_type.to_be_bytes());
    frame.extend_from_slice(&0u16.to_be_bytes());

    if !payload.is_empty() {
        let ctr = *local_counter;
        *local_counter = local_counter.wrapping_add(1);
        let enc = rpcrypt.encrypt(ctr, payload);
        frame.extend_from_slice(&enc);
    }

    stream.write_all(&frame).await?;
    Ok(())
}

async fn send_initial_session_acks(
    stream: &mut TcpStream,
    rpcrypt: &Rpcrypt,
    local_counter: &mut u64,
) -> Result<(), CtrlError> {
    write_ctrl_frame(stream, rpcrypt, local_counter, MSG_SESSION_ACK_13, &INIT_ACK_13_PAYLOAD)
        .await?;
    write_ctrl_frame(stream, rpcrypt, local_counter, MSG_SESSION_ACK_11, &INIT_ACK_11_PAYLOAD)
        .await?;
    let mic_muted = [0x00, 0x01, 0x01, 89];
    write_ctrl_frame(stream, rpcrypt, local_counter, MSG_MICROPHONE, &mic_muted).await?;
    write_ctrl_frame(stream, rpcrypt, local_counter, MSG_MICROPHONE, &mic_muted).await?;
    write_ctrl_frame(stream, rpcrypt, local_counter, MSG_SESSION_ACK_910, &INIT_ACK_910_PAYLOAD)
        .await?;
    Ok(())
}

fn parse_session_id_payload(payload: &[u8]) -> Result<String, CtrlError> {
    if payload.len() < 2 || payload[0] != 0x4a {
        return Err(CtrlError::InvalidFrame(format!(
            "unexpected 0x33 SESSION_ID payload prefix/len: len={}",
            payload.len()
        )));
    }
    let raw = &payload[1..];
    let trimmed = match raw.iter().position(|&b| b == 0) {
        Some(idx) => &raw[..idx],
        None => raw,
    };
    if !(24..=64).contains(&trimmed.len()) || !trimmed.iter().all(|b| b.is_ascii_alphanumeric()) {
        return Err(CtrlError::InvalidFrame(
            "SESSION_ID payload must be 24..=64 ASCII alphanumeric characters".into(),
        ));
    }
    Ok(String::from_utf8_lossy(trimmed).into_owned())
}

/// Connect to a PS5 TCP 9295 ctrl endpoint, authenticate exclusively as PS5,
/// retrieve the `SESSION_ID`, and spawn the background heartbeat/command task.
pub async fn connect_ctrl(config: CtrlConfig) -> Result<CtrlSession, CtrlError> {
    let nonce = fetch_session_nonce(&config).await?;
    let rpcrypt = Rpcrypt::init_auth_ps5(&nonce, &config.rp_key);

    let addr = std::net::SocketAddr::new(config.host, config.port);
    let mut stream = timeout(HTTP_TIMEOUT, TcpStream::connect(addr))
        .await
        .map_err(|_| CtrlError::Timeout("connecting for /sess/ctrl"))??;
    let _ = stream.set_nodelay(true);

    let auth_b64 = B64.encode(rpcrypt.encrypt(0, &config.rp_regist_key));
    let did = build_did_payload();
    let did_b64 = B64.encode(rpcrypt.encrypt(1, &did));
    let os_type_b64 = B64.encode(rpcrypt.encrypt(2, b"Win10.0.0\0"));
    let start_bitrate_b64 = B64.encode(rpcrypt.encrypt(3, &[0u8; 4]));
    let streaming_type: u32 = if config.hdr { 3 } else { 2 };
    let streaming_type_b64 = B64.encode(rpcrypt.encrypt(4, &streaming_type.to_le_bytes()));
    let mut local_counter: u64 = 5;

    let req = format!(
        "GET /sie/ps5/rp/sess/ctrl HTTP/1.1\r\n\
         Host: {}:{}\r\n\
         User-Agent: remoteplay Windows\r\n\
         Connection: keep-alive\r\n\
         RP-Auth: {auth_b64}\r\n\
         RP-Version: 1.0\r\n\
         RP-Did: {did_b64}\r\n\
         RP-ControllerType: 3\r\n\
         RP-ClientType: 11\r\n\
         RP-OSType: {os_type_b64}\r\n\
         RP-ConPath: 1\r\n\
         RP-PadProcNo: 3\r\n\
         RP-StartBitrate: {start_bitrate_b64}\r\n\
         RP-StreamingType: {streaming_type_b64}\r\n\
         \r\n",
        config.host, config.port
    );

    timeout(HTTP_TIMEOUT, stream.write_all(req.as_bytes()))
        .await
        .map_err(|_| CtrlError::Timeout("writing /sess/ctrl request"))??;

    let head = read_http_head(&mut stream).await?;
    if head.status != 200 {
        return Err(CtrlError::HttpStatus {
            status: head.status,
            reason: head.headers.get("rp-application-reason").cloned(),
        });
    }

    let server_type_b64 = head
        .headers
        .get("rp-server-type")
        .ok_or(CtrlError::InvalidHeader("RP-Server-Type"))?;
    let server_type_enc = B64
        .decode(server_type_b64.as_bytes())
        .map_err(|_| CtrlError::InvalidHeader("RP-Server-Type"))?;
    let mut remote_counter: u64 = 0;
    let server_type_plain = rpcrypt.decrypt(remote_counter, &server_type_enc);
    remote_counter += 1;

    let server_type = *server_type_plain
        .first()
        .ok_or(CtrlError::InvalidHeader("RP-Server-Type"))?;
    if server_type != 2 {
        return Err(CtrlError::UnsupportedConsole(server_type));
    }

    let mut leftover = head.leftover;
    let deadline = Instant::now() + SESSION_ID_TIMEOUT;
    let session_id = loop {
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .ok_or(CtrlError::Timeout("waiting for 0x33 SESSION_ID"))?;
        let (msg_type, payload) = timeout(
            remaining,
            read_ctrl_frame(&mut stream, &mut leftover, &rpcrypt, &mut remote_counter),
        )
        .await
        .map_err(|_| CtrlError::Timeout("waiting for 0x33 SESSION_ID"))??;

        match msg_type {
            MSG_SESSION_ID => {
                let sid = parse_session_id_payload(&payload)?;
                send_initial_session_acks(&mut stream, &rpcrypt, &mut local_counter).await?;
                break sid;
            }
            MSG_HEARTBEAT_REQ => {
                write_ctrl_frame(&mut stream, &rpcrypt, &mut local_counter, MSG_HEARTBEAT_ACK, &[])
                    .await?;
            }
            MSG_LOGIN_PIN_REQ => {
                return Err(CtrlError::LoginPinRequired);
            }
            MSG_KEYBOARD => {}
            _ => {}
        }
    };

    let (tx, rx) = mpsc::channel(32);
    let (closed_tx, closed_rx) = watch::channel(None);

    let rpcrypt_copy = Rpcrypt {
        bright: rpcrypt.bright,
        ambassador: rpcrypt.ambassador,
    };

    tokio::spawn(run_ctrl_loop(
        stream,
        leftover,
        rpcrypt,
        local_counter,
        remote_counter,
        rx,
        closed_tx,
    ));

    Ok(CtrlSession {
        session_id,
        rpcrypt: rpcrypt_copy,
        handle: CtrlHandle { tx, closed_rx },
    })
}

async fn run_ctrl_loop(
    mut stream: TcpStream,
    mut leftover: Vec<u8>,
    rpcrypt: Rpcrypt,
    mut local_counter: u64,
    mut remote_counter: u64,
    mut rx: mpsc::Receiver<CtrlCommand>,
    closed_tx: watch::Sender<Option<String>>,
) {
    let reason = loop {
        tokio::select! {
            frame_res = read_ctrl_frame(&mut stream, &mut leftover, &rpcrypt, &mut remote_counter) => {
                match frame_res {
                    Ok((msg_type, _payload)) => {
                        if msg_type == MSG_HEARTBEAT_REQ {
                            if let Err(e) = write_ctrl_frame(
                                &mut stream,
                                &rpcrypt,
                                &mut local_counter,
                                MSG_HEARTBEAT_ACK,
                                &[],
                            )
                            .await
                            {
                                break format!("heartbeat ack write failed: {e}");
                            }
                        }
                    }
                    Err(e) => break format!("ctrl read error: {e}"),
                }
            }
            cmd_opt = rx.recv() => {
                match cmd_opt {
                    Some(CtrlCommand::Standby) => {
                        if let Err(e) = write_ctrl_frame(
                            &mut stream,
                            &rpcrypt,
                            &mut local_counter,
                            MSG_STANDBY,
                            &[],
                        )
                        .await
                        {
                            break format!("standby write failed: {e}");
                        }
                    }
                    Some(CtrlCommand::GoHome) => {
                        let mut payload = [0u8; 16];
                        payload[1] = 0xff;
                        if let Err(e) = write_ctrl_frame(
                            &mut stream,
                            &rpcrypt,
                            &mut local_counter,
                            MSG_GO_HOME,
                            &payload,
                        )
                        .await
                        {
                            break format!("go_home write failed: {e}");
                        }
                    }
                    Some(CtrlCommand::SetMicrophone(unmuted)) => {
                        let payload = [0x00, 0x01, if unmuted { 0x00 } else { 0x01 }, 89];
                        if let Err(e) = write_ctrl_frame(
                            &mut stream,
                            &rpcrypt,
                            &mut local_counter,
                            MSG_MICROPHONE,
                            &payload,
                        )
                        .await
                        {
                            break format!("microphone write failed: {e}");
                        }
                    }
                    Some(CtrlCommand::LoginPin(pin_bytes)) => {
                        if let Err(e) = write_ctrl_frame(
                            &mut stream,
                            &rpcrypt,
                            &mut local_counter,
                            MSG_LOGIN_PIN_RESP,
                            &pin_bytes,
                        )
                        .await
                        {
                            break format!("login pin write failed: {e}");
                        }
                    }
                    Some(CtrlCommand::Shutdown) | None => {
                        break "shutdown requested".into();
                    }
                }
            }
        }
    };

    let _ = stream.shutdown().await;
    let _ = closed_tx.send(Some(reason));
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::TcpListener;

    #[test]
    fn formats_registkey_header_up_to_first_null() {
        let mut key = [0u8; 16];
        key[..8].copy_from_slice(b"1a2b3c4d");
        assert_eq!(format_rp_registkey_header(&key), "3161326233633464");
    }

    #[test]
    fn validates_session_id_prefix_and_length() {
        let mut raw = vec![0x4a];
        raw.extend_from_slice(b"AbCdEf0123456789AbCdEf0123456789");
        assert_eq!(
            parse_session_id_payload(&raw).unwrap(),
            "AbCdEf0123456789AbCdEf0123456789"
        );
        assert!(parse_session_id_payload(b"\x4bshort").is_err());
        assert!(parse_session_id_payload(b"\x4ashort").is_err());
    }

    async fn read_http_req(stream: &mut TcpStream) -> HashMap<String, String> {
        let mut buf = Vec::new();
        let mut tmp = [0u8; 1024];
        loop {
            let n = stream.read(&mut tmp).await.unwrap();
            assert!(n > 0);
            buf.extend_from_slice(&tmp[..n]);
            if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                let text = std::str::from_utf8(&buf[..pos]).unwrap();
                let mut map = HashMap::new();
                for line in text.split("\r\n").skip(1) {
                    if let Some((k, v)) = line.split_once(':') {
                        map.insert(k.trim().to_ascii_lowercase(), v.trim().to_string());
                    }
                }
                return map;
            }
        }
    }

    #[tokio::test]
    async fn loopback_ps5_ctrl_handshake_and_heartbeat() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let nonce = [0x11u8; 16];
        let rp_key = [0x22u8; 16];
        let mut rp_regist_key = [0u8; 16];
        rp_regist_key[..8].copy_from_slice(b"deadbeef");

        let server = tokio::spawn(async move {
            // Connection 1: /sie/ps5/rp/sess/init
            let (mut s1, _) = listener.accept().await.unwrap();
            let req1 = read_http_req(&mut s1).await;
            assert_eq!(
                req1.get("rp-registkey").unwrap(),
                &hex::encode(b"deadbeef")
            );
            let nonce_b64 = B64.encode(nonce);
            let resp1 = format!(
                "HTTP/1.1 200 OK\r\nRP-Nonce: {nonce_b64}\r\nContent-Length: 0\r\n\r\n"
            );
            s1.write_all(resp1.as_bytes()).await.unwrap();
            drop(s1);

            // Connection 2: /sie/ps5/rp/sess/ctrl
            let (mut s2, _) = listener.accept().await.unwrap();
            let req2 = read_http_req(&mut s2).await;
            let server_crypt = Rpcrypt::init_auth_ps5(&nonce, &rp_key);
            let auth_plain =
                server_crypt.decrypt(0, &B64.decode(req2.get("rp-auth").unwrap()).unwrap());
            assert_eq!(auth_plain.as_slice(), &rp_regist_key);
            assert_eq!(req2.get("rp-padprocno").unwrap(), "3");

            let st_b64 = B64.encode(server_crypt.encrypt(0, &[2u8; 16]));
            let resp2 = format!(
                "HTTP/1.1 200 OK\r\nRP-Server-Type: {st_b64}\r\nContent-Length: 0\r\n\r\n"
            );
            s2.write_all(resp2.as_bytes()).await.unwrap();

            // Send heartbeat (0xfe) then SESSION_ID (0x33)
            let mut local_ctr = 1u64;
            let mut remote_ctr = 5u64;

            let hb_hdr = [0u8, 0, 0, 0, 0x00, 0xfe, 0, 0];
            s2.write_all(&hb_hdr).await.unwrap();

            let mut ack_hdr = [0u8; 8];
            s2.read_exact(&mut ack_hdr).await.unwrap();
            assert_eq!(u16::from_be_bytes([ack_hdr[4], ack_hdr[5]]), MSG_HEARTBEAT_ACK);

            let mut sid_payload = vec![0x4a];
            sid_payload.extend_from_slice(b"PS5SessionId0123456789ABCDEF");
            let sid_enc = server_crypt.encrypt(local_ctr, &sid_payload);
            let _ = &mut local_ctr;
            let mut sid_frame = Vec::new();
            sid_frame.extend_from_slice(&(sid_enc.len() as u32).to_be_bytes());
            sid_frame.extend_from_slice(&MSG_SESSION_ID.to_be_bytes());
            sid_frame.extend_from_slice(&0u16.to_be_bytes());
            sid_frame.extend_from_slice(&sid_enc);
            s2.write_all(&sid_frame).await.unwrap();

            // Expect initial session acks: 0x13, 0x11, 0x36, 0x36, 0x910
            let expected = [
                MSG_SESSION_ACK_13,
                MSG_SESSION_ACK_11,
                MSG_MICROPHONE,
                MSG_MICROPHONE,
                MSG_SESSION_ACK_910,
            ];
            for exp_ty in expected {
                let mut h = [0u8; 8];
                s2.read_exact(&mut h).await.unwrap();
                let len = u32::from_be_bytes([h[0], h[1], h[2], h[3]]) as usize;
                let ty = u16::from_be_bytes([h[4], h[5]]);
                assert_eq!(ty, exp_ty);
                let mut body = vec![0u8; len];
                s2.read_exact(&mut body).await.unwrap();
                let _plain = server_crypt.decrypt(remote_ctr, &body);
                remote_ctr += 1;
            }
        });

        let mut cfg = CtrlConfig::new("127.0.0.1".parse().unwrap(), rp_regist_key, rp_key, false);
        cfg.port = port;
        let session = connect_ctrl(cfg).await.unwrap();
        assert_eq!(session.session_id, "PS5SessionId0123456789ABCDEF");
        session.handle.shutdown().await;
        server.await.unwrap();
    }

    #[tokio::test]
    async fn rejects_ps4_server_type() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let nonce = [0x33u8; 16];
        let rp_key = [0x44u8; 16];

        let server = tokio::spawn(async move {
            let (mut s1, _) = listener.accept().await.unwrap();
            let _ = read_http_req(&mut s1).await;
            let resp1 = format!(
                "HTTP/1.1 200 OK\r\nRP-Nonce: {}\r\n\r\n",
                B64.encode(nonce)
            );
            s1.write_all(resp1.as_bytes()).await.unwrap();
            drop(s1);

            let (mut s2, _) = listener.accept().await.unwrap();
            let _ = read_http_req(&mut s2).await;
            let crypt = Rpcrypt::init_auth_ps5(&nonce, &rp_key);
            // Server type 1 = PS4
            let resp2 = format!(
                "HTTP/1.1 200 OK\r\nRP-Server-Type: {}\r\n\r\n",
                B64.encode(crypt.encrypt(0, &[1u8; 16]))
            );
            s2.write_all(resp2.as_bytes()).await.unwrap();
        });

        let mut cfg = CtrlConfig::new("127.0.0.1".parse().unwrap(), [0u8; 16], rp_key, false);
        cfg.port = port;
        let err = connect_ctrl(cfg).await.err().unwrap();
        assert!(matches!(err, CtrlError::UnsupportedConsole(1)));
        server.await.unwrap();
    }
}
