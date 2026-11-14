//! Prost definitions matching Sony Takion (`tkproto`, proto2 wire format).

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, prost::Enumeration)]
#[repr(i32)]
pub enum PayloadType {
    Big = 0,
    Bang = 1,
    Info = 2,
    Heartbeat = 3,
    Packetloss = 4,
    Corruptframe = 5,
    Cursor = 6,
    Timer = 7,
    Disconnect = 8,
    Log = 9,
    Headerrequest = 10,
    Debug = 11,
    Senkusha = 12,
    Streaminfo = 13,
    Streaminfoack = 14,
    Xmbcommand = 15,
    Connectionquality = 16,
    Clientmetric = 17,
    Playtimeleft = 18,
    Servermessage = 19,
    Fpschange = 20,
    Controllerconnection = 21,
    Clientinfo = 22,
    Videocapture = 23,
    Audiocapture = 24,
    Idrrequest = 25,
    Gktrace = 26,
    Periodictimestamp = 27,
    Serversettings = 28,
    Directmessage = 29,
    Micconnection = 30,
    Takionprotocolrequest = 31,
    Takionprotocolrequestack = 32,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct TakionMessage {
    #[prost(enumeration = "PayloadType", required, tag = "1")]
    pub r#type: i32,
    #[prost(message, optional, tag = "2")]
    pub big_payload: Option<BigPayload>,
    #[prost(message, optional, tag = "3")]
    pub bang_payload: Option<BangPayload>,
    #[prost(message, optional, tag = "4")]
    pub info_payload: Option<InfoPayload>,
    #[prost(message, optional, tag = "5")]
    pub loss_payload: Option<PacketLossPayload>,
    #[prost(message, optional, tag = "6")]
    pub corrupt_payload: Option<CorruptFramePayload>,
    #[prost(message, optional, tag = "8")]
    pub cursor_payload: Option<CursorPayload>,
    #[prost(message, optional, tag = "9")]
    pub timer_payload: Option<TimerPayload>,
    #[prost(message, optional, tag = "10")]
    pub disconnect_payload: Option<DisconnectPayload>,
    #[prost(message, optional, tag = "11")]
    pub log_payload: Option<LogPayload>,
    #[prost(message, optional, tag = "12")]
    pub header_payload: Option<HeaderRequestPayload>,
    #[prost(message, optional, tag = "13")]
    pub debug_payload: Option<DebugOption>,
    #[prost(message, optional, tag = "14")]
    pub senkusha_payload: Option<SenkushaPayload>,
    #[prost(message, optional, tag = "15")]
    pub stream_info_payload: Option<StreamInfoPayload>,
    #[prost(message, optional, tag = "16")]
    pub xmb_command_payload: Option<XmbCommandPayload>,
    #[prost(message, optional, tag = "17")]
    pub connection_quality_payload: Option<ConnectionQualityPayload>,
    #[prost(message, optional, tag = "18")]
    pub client_metric_payload: Option<ClientMetricPayload>,
    #[prost(message, optional, tag = "19")]
    pub play_time_left_payload: Option<PlayTimeLeftPayload>,
    #[prost(message, optional, tag = "20")]
    pub server_message_payload: Option<ServerMessagePayload>,
    #[prost(message, optional, tag = "21")]
    pub fps_change_payload: Option<FpsChangePayload>,
    #[prost(message, optional, tag = "22")]
    pub controller_connection_payload: Option<ControllerConnectionPayload>,
    #[prost(message, optional, tag = "23")]
    pub client_info_payload: Option<ClientInfoPayload>,
    #[prost(message, optional, tag = "24")]
    pub video_capture_payload: Option<VideoCapturePayload>,
    #[prost(message, optional, tag = "25")]
    pub audio_capture_payload: Option<AudioCapturePayload>,
    #[prost(message, optional, tag = "26")]
    pub gk_trace_payload: Option<GkTracePayload>,
    #[prost(message, optional, tag = "27")]
    pub periodic_timestamp_payload: Option<PeriodicTimestampPayload>,
    #[prost(message, optional, tag = "28")]
    pub server_settings_payload: Option<ServerSettingsPayload>,
    #[prost(message, optional, tag = "29")]
    pub direct_message_payload: Option<ServerMessagePayload>,
    #[prost(message, optional, tag = "30")]
    pub mic_connection_payload: Option<MicConnectionPayload>,
    #[prost(message, optional, tag = "31")]
    pub takion_protocol_request: Option<TakionProtocolRequestPayload>,
    #[prost(message, optional, tag = "32")]
    pub takion_protocol_request_ack: Option<TakionProtocolRequestAckPayload>,
}

impl TakionMessage {
    pub fn with_type(payload_type: PayloadType) -> Self {
        Self {
            r#type: payload_type as i32,
            big_payload: None,
            bang_payload: None,
            info_payload: None,
            loss_payload: None,
            corrupt_payload: None,
            cursor_payload: None,
            timer_payload: None,
            disconnect_payload: None,
            log_payload: None,
            header_payload: None,
            debug_payload: None,
            senkusha_payload: None,
            stream_info_payload: None,
            xmb_command_payload: None,
            connection_quality_payload: None,
            client_metric_payload: None,
            play_time_left_payload: None,
            server_message_payload: None,
            fps_change_payload: None,
            controller_connection_payload: None,
            client_info_payload: None,
            video_capture_payload: None,
            audio_capture_payload: None,
            gk_trace_payload: None,
            periodic_timestamp_payload: None,
            server_settings_payload: None,
            direct_message_payload: None,
            mic_connection_payload: None,
            takion_protocol_request: None,
            takion_protocol_request_ack: None,
        }
    }

    pub fn payload_type(&self) -> Option<PayloadType> {
        PayloadType::try_from(self.r#type).ok()
    }
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct EventCode {
    #[prost(string, required, tag = "1")]
    pub event_code: String,
    #[prost(string, optional, tag = "2")]
    pub name: Option<String>,
    #[prost(string, optional, tag = "3")]
    pub description: Option<String>,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct BigPayload {
    #[prost(uint32, required, tag = "1")]
    pub client_version: u32,
    #[prost(string, required, tag = "2")]
    pub session_key: String,
    #[prost(string, required, tag = "3")]
    pub launch_spec: String,
    #[prost(bytes = "vec", required, tag = "4")]
    pub encrypted_key: Vec<u8>,
    #[prost(bytes = "vec", optional, tag = "5")]
    pub ecdh_pub_key: Option<Vec<u8>>,
    #[prost(bytes = "vec", optional, tag = "6")]
    pub ecdh_sig: Option<Vec<u8>>,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct BangPayload {
    #[prost(uint32, required, tag = "1")]
    pub server_version: u32,
    #[prost(uint32, required, tag = "2")]
    pub token: u32,
    #[prost(bool, required, tag = "3")]
    pub encrypted_key_accepted: bool,
    #[prost(bool, required, tag = "4")]
    pub version_accepted: bool,
    #[prost(string, required, tag = "5")]
    pub session_key: String,
    #[prost(message, optional, tag = "6")]
    pub extended_info: Option<EventCode>,
    #[prost(string, optional, tag = "7")]
    pub server_version_string: Option<String>,
    #[prost(bytes = "vec", optional, tag = "8")]
    pub ecdh_pub_key: Option<Vec<u8>>,
    #[prost(bytes = "vec", optional, tag = "9")]
    pub ecdh_sig: Option<Vec<u8>>,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct InfoPayload {
    #[prost(uint32, required, tag = "1")]
    pub effective_bw: u32,
    #[prost(uint32, required, tag = "2")]
    pub bitrate: u32,
    #[prost(uint32, required, tag = "3")]
    pub min_bitrate: u32,
    #[prost(uint32, required, tag = "4")]
    pub target_bitrate: u32,
    #[prost(uint32, required, tag = "5")]
    pub corrupt_frame_freq: u32,
    #[prost(uint32, required, tag = "6")]
    pub mtu: u32,
    #[prost(uint32, required, tag = "7")]
    pub fps: u32,
    #[prost(uint32, required, tag = "8")]
    pub monitor_interval: u32,
    #[prost(uint32, required, tag = "9")]
    pub ext_overhead: u32,
    #[prost(uint32, required, tag = "10")]
    pub int_overhead: u32,
    #[prost(uint32, required, tag = "11")]
    pub timeout_interval: u32,
    #[prost(uint32, required, tag = "12")]
    pub min_resume_quality: u32,
    #[prost(uint32, required, tag = "13")]
    pub min_fps: u32,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct CursorPayload {
    #[prost(uint32, required, tag = "1")]
    pub id: u32,
    #[prost(uint32, required, tag = "2")]
    pub width: u32,
    #[prost(uint32, required, tag = "3")]
    pub height: u32,
    #[prost(uint32, required, tag = "4")]
    pub hotspot_x: u32,
    #[prost(uint32, required, tag = "5")]
    pub hotspot_y: u32,
    #[prost(bytes = "vec", optional, tag = "6")]
    pub pixels: Option<Vec<u8>>,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct TimerPayload {
    #[prost(uint32, required, tag = "1")]
    pub seconds_left: u32,
    #[prost(uint32, required, tag = "2")]
    pub total_seconds: u32,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct CorruptFramePayload {
    #[prost(uint32, required, tag = "1")]
    pub start: u32,
    #[prost(uint32, required, tag = "2")]
    pub end: u32,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct PacketLossPayload {
    #[prost(uint32, required, tag = "1")]
    pub start: u32,
    #[prost(uint32, required, tag = "2")]
    pub end: u32,
    #[prost(uint32, required, tag = "3")]
    pub packets_received: u32,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct FpsChangePayload {
    #[prost(uint32, required, tag = "1")]
    pub fps: u32,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct DisconnectPayload {
    #[prost(string, required, tag = "1")]
    pub reason: String,
    #[prost(message, optional, tag = "2")]
    pub extended_info: Option<EventCode>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, prost::Enumeration)]
#[repr(i32)]
pub enum LogLevel {
    Critical = 0,
    Error = 1,
    Warning = 2,
    Info = 3,
    Debug = 4,
    Trace = 5,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct LogMessage {
    #[prost(int64, required, tag = "1")]
    pub timestamp: i64,
    #[prost(enumeration = "LogLevel", required, tag = "2")]
    pub level: i32,
    #[prost(string, required, tag = "3")]
    pub component: String,
    #[prost(string, required, tag = "4")]
    pub message: String,
    #[prost(double, optional, tag = "5")]
    pub timestampdouble: Option<f64>,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct LogPayload {
    #[prost(message, repeated, tag = "1")]
    pub messages: Vec<LogMessage>,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct ClientMetric {
    #[prost(int32, required, tag = "1")]
    pub category: i32,
    #[prost(int32, required, tag = "2")]
    pub start_frame: i32,
    #[prost(int32, required, tag = "3")]
    pub end_frame: i32,
    #[prost(int32, required, tag = "5")]
    pub count: i32,
    #[prost(float, required, tag = "6")]
    pub average: f32,
    #[prost(float, required, tag = "7")]
    pub std_dev: f32,
    #[prost(float, required, tag = "8")]
    pub min_val: f32,
    #[prost(float, required, tag = "9")]
    pub max_val: f32,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct ClientMetricPayload {
    #[prost(message, repeated, tag = "1")]
    pub messages: Vec<ClientMetric>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, prost::Enumeration)]
#[repr(i32)]
pub enum HeaderType {
    Audio = 0,
    Video = 1,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct HeaderRequestPayload {
    #[prost(enumeration = "HeaderType", required, tag = "1")]
    pub r#type: i32,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct DebugOption {
    #[prost(int32, required, tag = "1")]
    pub r#type: i32,
    #[prost(message, optional, tag = "2")]
    pub overlay: Option<OverlayPayload>,
    #[prost(message, optional, tag = "3")]
    pub bitrate: Option<BitratePayload>,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct OverlayPayload {
    #[prost(bool, optional, tag = "1")]
    pub enable: Option<bool>,
    #[prost(bool, optional, tag = "2")]
    pub toggle: Option<bool>,
    #[prost(int32, optional, tag = "3")]
    pub location: Option<i32>,
    #[prost(uint32, optional, tag = "4")]
    pub level: Option<u32>,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct BitratePayload {
    #[prost(bool, optional, tag = "1")]
    pub toggle: Option<bool>,
    #[prost(float, optional, tag = "2")]
    pub bitrate_change: Option<f32>,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct ResolutionPayload {
    #[prost(uint32, required, tag = "1")]
    pub width: u32,
    #[prost(uint32, required, tag = "2")]
    pub height: u32,
    #[prost(bytes = "vec", required, tag = "3")]
    pub video_header: Vec<u8>,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct AudioChannelPayload {
    #[prost(uint32, required, tag = "1")]
    pub audio_channel_type: u32,
    #[prost(bytes = "vec", required, tag = "2")]
    pub audio_header: Vec<u8>,
    #[prost(bool, optional, tag = "3")]
    pub is_raw_pcm: Option<bool>,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct StreamInfoPayload {
    #[prost(message, repeated, tag = "1")]
    pub resolution: Vec<ResolutionPayload>,
    #[prost(bytes = "vec", required, tag = "2")]
    pub audio_header: Vec<u8>,
    #[prost(uint32, optional, tag = "3")]
    pub start_timeout: Option<u32>,
    #[prost(uint32, optional, tag = "4")]
    pub afk_timeout: Option<u32>,
    #[prost(uint32, optional, tag = "5")]
    pub afk_timeout_disconnect: Option<u32>,
    #[prost(uint32, optional, tag = "6")]
    pub congestion_control_interval: Option<u32>,
    #[prost(message, repeated, tag = "7")]
    pub audio_channel: Vec<AudioChannelPayload>,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct XmbCommandPayload {
    #[prost(int32, required, tag = "1")]
    pub command: i32,
    #[prost(uint32, optional, tag = "2")]
    pub player_id: Option<u32>,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct ConnectionQualityPayload {
    #[prost(uint32, optional, tag = "1")]
    pub target_bitrate: Option<u32>,
    #[prost(uint32, optional, tag = "2")]
    pub upstream_bitrate: Option<u32>,
    #[prost(float, optional, tag = "3")]
    pub upstream_loss: Option<f32>,
    #[prost(bool, optional, tag = "4")]
    pub disable_upstream_audio: Option<bool>,
    #[prost(double, optional, tag = "5")]
    pub rtt: Option<f64>,
    #[prost(uint64, optional, tag = "6")]
    pub loss: Option<u64>,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct PlayTimeLeftPayload {
    #[prost(uint32, optional, tag = "1")]
    pub time_left: Option<u32>,
    #[prost(string, optional, tag = "2")]
    pub reason: Option<String>,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct ServerMessagePayload {
    #[prost(string, optional, tag = "1")]
    pub payload: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, prost::Enumeration)]
#[repr(i32)]
pub enum ControllerType {
    Notset = 0,
    Dualshock3 = 1,
    Dualshock4 = 2,
    Vita = 3,
    Xinput = 4,
    Mobile = 5,
    Dualsense = 6,
    Vr2sense = 7,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct ControllerConnectionPayload {
    #[prost(int32, optional, tag = "1")]
    pub controller_id: Option<i32>,
    #[prost(bool, optional, tag = "2")]
    pub connected: Option<bool>,
    #[prost(enumeration = "ControllerType", optional, tag = "3")]
    pub controller_type: Option<i32>,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct ClientInfoPayload {
    #[prost(string, required, tag = "1")]
    pub session_key: String,
    #[prost(uint32, optional, tag = "2")]
    pub gcm_tag: Option<u32>,
    #[prost(uint32, optional, tag = "3")]
    pub key_pos: Option<u32>,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct VideoCapturePayload {
    #[prost(bool, optional, tag = "1")]
    pub server_side: Option<bool>,
    #[prost(uint32, optional, tag = "2")]
    pub idr_period: Option<u32>,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct AudioCapturePayload {
    #[prost(bool, optional, tag = "1")]
    pub server_side: Option<bool>,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct ServerSettingsPayload {
    #[prost(uint32, optional, tag = "1")]
    pub idr_period: Option<u32>,
    #[prost(bool, optional, tag = "2")]
    pub send_periodic_timestamp: Option<bool>,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct GkTracePayload {
    #[prost(uint32, required, tag = "1")]
    pub seq_no: u32,
    #[prost(uint32, required, tag = "2")]
    pub gcm_tag: u32,
    #[prost(uint32, required, tag = "3")]
    pub key_pos: u32,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct PeriodicTimestampPayload {
    #[prost(bytes = "vec", required, tag = "1")]
    pub periodic_ts_info: Vec<u8>,
    #[prost(uint32, required, tag = "2")]
    pub version: u32,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct MicConnectionPayload {
    #[prost(int32, required, tag = "1")]
    pub controller_id: i32,
    #[prost(bool, required, tag = "2")]
    pub connected: bool,
    #[prost(bool, optional, tag = "3")]
    pub result: Option<bool>,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct TakionProtocolRequestPayload {
    #[prost(uint32, repeated, packed = "false", tag = "1")]
    pub supported_takion_versions: Vec<u32>,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct TakionProtocolRequestAckPayload {
    #[prost(uint32, optional, tag = "1")]
    pub takion_protocol_version: Option<u32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, prost::Enumeration)]
#[repr(i32)]
pub enum SenkushaCommand {
    EchoCommand = 0,
    MtuCommand = 1,
    BandwidthCommand = 3,
    ClientMtuCommand = 4,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct SenkushaPayload {
    #[prost(enumeration = "SenkushaCommand", required, tag = "1")]
    pub command: i32,
    #[prost(message, optional, tag = "2")]
    pub echo_command: Option<SenkushaEchoCommand>,
    #[prost(message, optional, tag = "3")]
    pub mtu_command: Option<SenkushaMtuCommand>,
    #[prost(message, optional, tag = "4")]
    pub bandwidth_command: Option<SenkushaBandwidthCommand>,
    #[prost(message, optional, tag = "5")]
    pub client_mtu_command: Option<SenkushaClientMtuCommand>,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct SenkushaEchoCommand {
    #[prost(bool, required, tag = "1")]
    pub state: bool,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct SenkushaMtuCommand {
    #[prost(uint32, required, tag = "1")]
    pub id: u32,
    #[prost(uint32, required, tag = "2")]
    pub mtu_req: u32,
    #[prost(uint32, optional, tag = "3")]
    pub mtu_sent: Option<u32>,
    #[prost(uint32, optional, tag = "4")]
    pub num: Option<u32>,
    #[prost(uint32, optional, tag = "5")]
    pub send_delay: Option<u32>,
    #[prost(uint32, optional, tag = "6")]
    pub delta: Option<u32>,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct SenkushaBandwidthCommand {
    #[prost(uint32, required, tag = "1")]
    pub id: u32,
    #[prost(uint32, required, tag = "2")]
    pub time: u32,
    #[prost(uint32, optional, tag = "3")]
    pub bandwidth: Option<u32>,
    #[prost(uint32, optional, tag = "4")]
    pub fps: Option<u32>,
    #[prost(uint32, optional, tag = "5")]
    pub mtu: Option<u32>,
    #[prost(uint32, optional, tag = "6")]
    pub nalu_count: Option<u32>,
    #[prost(uint32, optional, tag = "7")]
    pub total_sent: Option<u32>,
    #[prost(bool, optional, tag = "8")]
    pub upstream: Option<bool>,
    #[prost(double, optional, tag = "9")]
    pub loss: Option<f64>,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct SenkushaClientMtuCommand {
    #[prost(uint32, required, tag = "1")]
    pub id: u32,
    #[prost(uint32, required, tag = "2")]
    pub mtu_req: u32,
    #[prost(bool, required, tag = "3")]
    pub state: bool,
    #[prost(uint32, optional, tag = "4")]
    pub mtu_down: Option<u32>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use prost::Message;

    #[test]
    fn big_and_bang_proto_roundtrip() {
        let mut msg = TakionMessage::with_type(PayloadType::Big);
        msg.big_payload = Some(BigPayload {
            client_version: 12,
            session_key: "sess-key".to_string(),
            launch_spec: "YWJj".to_string(),
            encrypted_key: vec![1, 2, 3, 4],
            ecdh_pub_key: Some(vec![0x04; 65]),
            ecdh_sig: Some(vec![0xaa; 32]),
        });
        let encoded = msg.encode_to_vec();
        // Field 1 (`type = BIG = 0`) MUST be emitted explicitly in proto2 wire format.
        assert_eq!(&encoded[..2], &[0x08, 0x00]);
        let decoded = TakionMessage::decode(encoded.as_slice()).unwrap();
        assert_eq!(decoded, msg);
    }

    #[test]
    fn stream_info_and_controller_connection_roundtrip() {
        let mut ctrl = TakionMessage::with_type(PayloadType::Controllerconnection);
        ctrl.controller_connection_payload = Some(ControllerConnectionPayload {
            controller_id: Some(0),
            connected: Some(true),
            controller_type: Some(ControllerType::Dualsense as i32),
        });
        let encoded = ctrl.encode_to_vec();
        let decoded = TakionMessage::decode(encoded.as_slice()).unwrap();
        assert_eq!(decoded, ctrl);

        let mut corrupt = TakionMessage::with_type(PayloadType::Corruptframe);
        corrupt.corrupt_payload = Some(CorruptFramePayload { start: 10, end: 12 });
        let enc2 = corrupt.encode_to_vec();
        assert_eq!(TakionMessage::decode(enc2.as_slice()).unwrap(), corrupt);
    }
}
