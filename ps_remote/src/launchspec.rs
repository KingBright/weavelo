//! PS5 Remote Play `LaunchSpec` JSON formatting & rpcrypt zero-keystream encryption.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;

use crate::rpcrypt::Rpcrypt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchSpecConfig {
    pub width: u32,
    pub height: u32,
    pub max_fps: u32,
    pub bw_kbps_sent: u32,
    pub mtu: u32,
    pub rtt_ms: u32,
    pub hdr: bool,
    pub handshake_key: [u8; 16],
}

impl Default for LaunchSpecConfig {
    fn default() -> Self {
        Self {
            width: 1920,
            height: 1080,
            max_fps: 60,
            bw_kbps_sent: 30_000,
            mtu: 1454,
            rtt_ms: 2,
            hdr: false,
            handshake_key: [0u8; 16],
        }
    }
}

/// Formats PS5 `LaunchSpec` JSON (always `hevc` + `adaptiveStreamMode: resize`).
pub fn format_launchspec_json(cfg: &LaunchSpecConfig) -> String {
    let handshake_key_b64 = BASE64_STANDARD.encode(cfg.handshake_key);
    let dynamic_range = if cfg.hdr { "HDR" } else { "SDR" };
    format!(
        concat!(
            "{{",
            "\"sessionId\":\"sessionId4321\",",
            "\"streamResolutions\":[{{",
            "\"resolution\":{{\"width\":{width},\"height\":{height}}},",
            "\"maxFps\":{max_fps},",
            "\"score\":10",
            "}}],",
            "\"network\":{{",
            "\"bwKbpsSent\":{bw_kbps_sent},",
            "\"bwLoss\":0.001000,",
            "\"mtu\":{mtu},",
            "\"rtt\":{rtt_ms},",
            "\"ports\":[53,2053]",
            "}},",
            "\"slotId\":1,",
            "\"appSpecification\":{{",
            "\"minFps\":30,",
            "\"minBandwidth\":0,",
            "\"extTitleId\":\"ps3\",",
            "\"version\":1,",
            "\"timeLimit\":1,",
            "\"startTimeout\":100,",
            "\"afkTimeout\":100,",
            "\"afkTimeoutDisconnect\":100",
            "}},",
            "\"konan\":{{",
            "\"ps3AccessToken\":\"accessToken\",",
            "\"ps3RefreshToken\":\"refreshToken\"",
            "}},",
            "\"requestGameSpecification\":{{",
            "\"model\":\"bravia_tv\",",
            "\"platform\":\"android\",",
            "\"audioChannels\":\"5.1\",",
            "\"language\":\"sp\",",
            "\"acceptButton\":\"X\",",
            "\"connectedControllers\":[\"xinput\",\"ds3\",\"ds4\"],",
            "\"yuvCoefficient\":\"bt601\",",
            "\"videoEncoderProfile\":\"hw4.1\",",
            "\"audioEncoderProfile\":\"audio1\",",
            "\"adaptiveStreamMode\": \"resize\"",
            "}},",
            "\"userProfile\":{{",
            "\"onlineId\":\"psnId\",",
            "\"npId\":\"npId\",",
            "\"region\":\"US\",",
            "\"languagesUsed\":[\"en\",\"jp\"]",
            "}},",
            "\"videoCodec\":\"hevc\",",
            "\"dynamicRange\":\"{dynamic_range}\",",
            "\"handshakeKey\":\"{handshake_key_b64}\"",
            "}}"
        ),
        width = cfg.width,
        height = cfg.height,
        max_fps = cfg.max_fps,
        bw_kbps_sent = cfg.bw_kbps_sent,
        mtu = cfg.mtu,
        rtt_ms = cfg.rtt_ms,
        dynamic_range = dynamic_range,
        handshake_key_b64 = handshake_key_b64,
    )
}

/// Encrypts `LaunchSpec` JSON with `rpcrypt` zero-keystream XOR (counter = 0) and Base64-encodes it.
pub fn encode_launchspec(cfg: &LaunchSpecConfig, crypt: &Rpcrypt) -> String {
    let mut plain = format_launchspec_json(cfg).into_bytes();
    plain.push(0);
    let cipher = crypt.encrypt_zero_keystream_xor(0, &plain);
    BASE64_STANDARD.encode(cipher)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn launchspec_json_and_zero_keystream_encrypt_roundtrip() {
        let cfg = LaunchSpecConfig {
            width: 1920,
            height: 1080,
            max_fps: 60,
            bw_kbps_sent: 25_000,
            mtu: 1454,
            rtt_ms: 2,
            hdr: false,
            handshake_key: [0x5a; 16],
        };
        let json = format_launchspec_json(&cfg);
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["videoCodec"], "hevc");
        assert_eq!(parsed["dynamicRange"], "SDR");
        assert_eq!(
            parsed["requestGameSpecification"]["adaptiveStreamMode"],
            "resize"
        );
        assert_eq!(parsed["streamResolutions"][0]["resolution"]["width"], 1920);

        let crypt = Rpcrypt::init_auth_ps5(&[0x11; 16], &[0x22; 16]);
        let b64 = encode_launchspec(&cfg, &crypt);
        let encrypted = BASE64_STANDARD.decode(b64).unwrap();
        let decrypted = crypt.encrypt_zero_keystream_xor(0, &encrypted);
        let mut expected = json.into_bytes();
        expected.push(0);
        assert_eq!(decrypted, expected);
    }
}
