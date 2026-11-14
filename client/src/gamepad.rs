//! Cross-platform controller capture + feedback engine for PS5 Remote Play.
//!
//! Prefers native Sony DualSense / DualSense Edge HID (touchpad, 6-axis IMU,
//! L2/R2 adaptive triggers, and rumble over USB or Bluetooth) while supporting
//! Xbox, Switch Pro, DualShock 4, and generic controllers through `gilrs`.

use crc32fast::Hasher as Crc32Hasher;
use protocol::gamepad::{ControllerKind, GamepadFeedback, GamepadState, TouchPoint, button};

pub const SONY_VID: u16 = 0x054C;
pub const DUALSENSE_PID: u16 = 0x0CE6;
pub const DUALSENSE_EDGE_PID: u16 = 0x0DF2;

const DUALSENSE_BT_OUTPUT_CRC_SEED: u8 = 0xA2;
const STICK_AXIS_MAX: f32 = 32767.0;

#[inline]
fn stick_u8_to_i16(raw: u8) -> i16 {
    let centered = i32::from(raw) - 128;
    let scaled = if centered >= 0 {
        (centered * 32767) / 127
    } else {
        (centered * 32767) / 128
    };
    scaled.clamp(-32767, 32767) as i16
}

#[inline]
fn decode_touch_point(bytes: &[u8]) -> TouchPoint {
    let contact = bytes[0];
    let active = (contact & 0x80) == 0;
    let id = contact & 0x7F;
    let x = (u16::from(bytes[1]) | (u16::from(bytes[2] & 0x0F) << 8)).min(1919);
    let y = ((u16::from(bytes[2]) >> 4) | (u16::from(bytes[3]) << 4)).min(941);
    TouchPoint { active, id, x, y }
}

fn map_hat_and_buttons(b0: u8, b1: u8, b2: u8) -> u32 {
    let mut mask = 0u32;
    match b0 & 0x0F {
        0 => mask |= button::DPAD_UP,
        1 => mask |= button::DPAD_UP | button::DPAD_RIGHT,
        2 => mask |= button::DPAD_RIGHT,
        3 => mask |= button::DPAD_RIGHT | button::DPAD_DOWN,
        4 => mask |= button::DPAD_DOWN,
        5 => mask |= button::DPAD_DOWN | button::DPAD_LEFT,
        6 => mask |= button::DPAD_LEFT,
        7 => mask |= button::DPAD_LEFT | button::DPAD_UP,
        _ => {}
    }
    if (b0 & 0x10) != 0 {
        mask |= button::SQUARE;
    }
    if (b0 & 0x20) != 0 {
        mask |= button::CROSS;
    }
    if (b0 & 0x40) != 0 {
        mask |= button::CIRCLE;
    }
    if (b0 & 0x80) != 0 {
        mask |= button::TRIANGLE;
    }
    if (b1 & 0x01) != 0 {
        mask |= button::L1;
    }
    if (b1 & 0x02) != 0 {
        mask |= button::R1;
    }
    if (b1 & 0x10) != 0 {
        mask |= button::SHARE;
    }
    if (b1 & 0x20) != 0 {
        mask |= button::OPTIONS;
    }
    if (b1 & 0x40) != 0 {
        mask |= button::L3;
    }
    if (b1 & 0x80) != 0 {
        mask |= button::R3;
    }
    if (b2 & 0x01) != 0 {
        mask |= button::PS;
    }
    if (b2 & 0x02) != 0 {
        mask |= button::TOUCHPAD;
    }
    mask
}

#[inline]
fn read_le_i16(bytes: &[u8]) -> i16 {
    i16::from_le_bytes([bytes[0], bytes[1]])
}

/// Parses a DualSense / DualSense Edge HID input report (USB `0x01`, BT extended `0x31`, or BT minimal `0x01`).
pub fn parse_dualsense_input_report(report: &[u8], controller_id: u8) -> Option<GamepadState> {
    let &report_id = report.first()?;
    if report_id == 0x01 && report.len() >= 10 && report.len() < 64 {
        // Minimal Bluetooth HID input report before feature report 0x05/0x31 calibration unlock.
        let s = &report[1..];
        let mut l2 = s[7];
        let mut r2 = s[8];
        if l2 == 0 && (s[5] & 0x04) != 0 {
            l2 = 255;
        }
        if r2 == 0 && (s[5] & 0x08) != 0 {
            r2 = 255;
        }
        return Some(GamepadState {
            controller_id,
            kind: ControllerKind::DualSense,
            buttons: map_hat_and_buttons(s[4], s[5], s[6]),
            left_trigger: l2,
            right_trigger: r2,
            left_stick_x: stick_u8_to_i16(s[0]),
            left_stick_y: stick_u8_to_i16(s[1]),
            right_stick_x: stick_u8_to_i16(s[2]),
            right_stick_y: stick_u8_to_i16(s[3]),
            touches: [TouchPoint::default(), TouchPoint::default()],
            has_imu: false,
            gyro: [0.0; 3],
            accel: [0.0; 3],
            quat: [0.0, 0.0, 0.0, 1.0],
        });
    }

    let s = if report_id == 0x01 && report.len() >= 64 {
        &report[1..]
    } else if report_id == 0x31 && report.len() >= 78 {
        &report[2..]
    } else {
        return None;
    };

    let mut l2 = s[4];
    let mut r2 = s[5];
    if l2 == 0 && (s[8] & 0x04) != 0 {
        l2 = 255;
    }
    if r2 == 0 && (s[8] & 0x08) != 0 {
        r2 = 255;
    }

    let (has_imu, gyro, accel) = if s.len() >= 27 {
        let gyro_scale = 2000.0_f32.to_radians() / 32768.0;
        let accel_scale = 1.0_f32 / 8192.0;
        (
            true,
            [
                f32::from(read_le_i16(&s[15..17])) * gyro_scale,
                f32::from(read_le_i16(&s[17..19])) * gyro_scale,
                f32::from(read_le_i16(&s[19..21])) * gyro_scale,
            ],
            [
                f32::from(read_le_i16(&s[21..23])) * accel_scale,
                f32::from(read_le_i16(&s[23..25])) * accel_scale,
                f32::from(read_le_i16(&s[25..27])) * accel_scale,
            ],
        )
    } else {
        (false, [0.0; 3], [0.0; 3])
    };

    let touches = if s.len() >= 40 {
        [
            decode_touch_point(&s[32..36]),
            decode_touch_point(&s[36..40]),
        ]
    } else {
        [TouchPoint::default(), TouchPoint::default()]
    };

    Some(GamepadState {
        controller_id,
        kind: ControllerKind::DualSense,
        buttons: map_hat_and_buttons(s[7], s[8], s[9]),
        left_trigger: l2,
        right_trigger: r2,
        left_stick_x: stick_u8_to_i16(s[0]),
        left_stick_y: stick_u8_to_i16(s[1]),
        right_stick_x: stick_u8_to_i16(s[2]),
        right_stick_y: stick_u8_to_i16(s[3]),
        touches,
        has_imu,
        gyro,
        accel,
        quat: [0.0, 0.0, 0.0, 1.0],
    })
}

/// Combined rumble + adaptive trigger state sent to a physical DualSense controller.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DualSenseFeedbackState {
    pub left_large: u8,
    pub right_small: u8,
    pub left_trigger: [u8; 10],
    pub right_trigger: [u8; 10],
    bt_seq: u8,
}

impl DualSenseFeedbackState {
    /// Updates cached rumble/adaptive-trigger payloads from a PS5 `GamepadFeedback` event.
    /// Returns `true` when the HID output state changed and should be transmitted.
    pub fn apply(&mut self, feedback: &GamepadFeedback) -> bool {
        match feedback {
            GamepadFeedback::Rumble {
                left_large,
                right_small,
                ..
            } => {
                if self.left_large == *left_large && self.right_small == *right_small {
                    return false;
                }
                self.left_large = *left_large;
                self.right_small = *right_small;
                true
            }
            GamepadFeedback::TriggerEffects { left, right, .. } => {
                if self.left_trigger == *left && self.right_trigger == *right {
                    return false;
                }
                self.left_trigger = *left;
                self.right_trigger = *right;
                true
            }
            GamepadFeedback::AudioHaptics { .. } => false,
        }
    }

    pub fn reset(&mut self) {
        self.left_large = 0;
        self.right_small = 0;
        self.left_trigger = [0; 10];
        self.right_trigger = [0; 10];
    }

    fn write_common_body(&self, body: &mut [u8]) {
        // valid_flag0: 0x01 compatible vibration + 0x02 haptics select
        //             + 0x04 right trigger effect + 0x08 left trigger effect
        body[0] = 0x0F;
        body[1] = 0x00;
        body[2] = self.right_small;
        body[3] = self.left_large;
        body[10..20].copy_from_slice(&self.right_trigger);
        body[21..31].copy_from_slice(&self.left_trigger);
    }

    /// Builds a 48-byte DualSense USB HID output report (`0x02`).
    pub fn build_usb_output_report(&self) -> [u8; 48] {
        let mut out = [0u8; 48];
        out[0] = 0x02;
        self.write_common_body(&mut out[1..]);
        out
    }

    /// Builds a 78-byte DualSense Bluetooth HID output report (`0x31`) with `0xA2`-seeded CRC32.
    pub fn build_bt_output_report(&mut self) -> [u8; 78] {
        let mut out = [0u8; 78];
        out[0] = 0x31;
        out[1] = ((self.bt_seq & 0x0F) << 4) | 0x02;
        self.bt_seq = (self.bt_seq + 1) & 0x0F;
        self.write_common_body(&mut out[2..]);

        let mut hasher = Crc32Hasher::new();
        hasher.update(&[DUALSENSE_BT_OUTPUT_CRC_SEED]);
        hasher.update(&out[..74]);
        let crc = hasher.finalize();
        out[74..78].copy_from_slice(&crc.to_le_bytes());
        out
    }
}

#[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]
mod desktop {
    use super::*;
    use gilrs::{Axis, Button, Gilrs};
    use hidapi::{HidApi, HidDevice};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering::Relaxed};
    use std::time::{Duration, Instant};
    use tokio::sync::broadcast;

    const RESCAN_INTERVAL: Duration = Duration::from_secs(2);

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum DualSenseTransport {
        Unknown,
        Usb,
        Bluetooth,
    }

    struct ActiveDualSense {
        device: HidDevice,
        transport: DualSenseTransport,
        last_state: Option<GamepadState>,
    }

    /// Cross-platform desktop controller manager (native DualSense HID + universal `gilrs` fallback).
    pub struct GamepadManager {
        hid: Option<HidApi>,
        dualsense: Option<ActiveDualSense>,
        gilrs: Option<Gilrs>,
        feedback: DualSenseFeedbackState,
        last_scan: Instant,
    }

    impl GamepadManager {
        pub fn new() -> Self {
            let hid = HidApi::new().ok();
            let gilrs = Gilrs::new().ok();
            let mut mgr = Self {
                hid,
                dualsense: None,
                gilrs,
                feedback: DualSenseFeedbackState::default(),
                last_scan: Instant::now() - RESCAN_INTERVAL,
            };
            mgr.probe_dualsense();
            mgr
        }

        fn probe_dualsense(&mut self) {
            self.last_scan = Instant::now();
            if let Some(hid) = self.hid.as_mut() {
                let _ = hid.refresh_devices();
                for info in hid.device_list() {
                    if info.vendor_id() == SONY_VID
                        && matches!(info.product_id(), DUALSENSE_PID | DUALSENSE_EDGE_PID)
                        && let Ok(device) = info.open_device(hid)
                    {
                        let _ = device.set_blocking_mode(false);
                        // Reading Feature Report 0x05 unlocks extended 0x31 BT reports on DualSense.
                        let mut calib = [0u8; 41];
                        calib[0] = 0x05;
                        let is_bt = info.interface_number() < 0;
                        if is_bt {
                            let _ = device.get_feature_report(&mut calib);
                        }
                        self.dualsense = Some(ActiveDualSense {
                            device,
                            transport: if is_bt {
                                DualSenseTransport::Bluetooth
                            } else {
                                DualSenseTransport::Usb
                            },
                            last_state: None,
                        });
                        return;
                    }
                }
            }
        }

        /// Polls the latest DualSense or universal `gilrs` state.
        pub fn poll(&mut self) -> Option<GamepadState> {
            if self.dualsense.is_none() && self.last_scan.elapsed() >= RESCAN_INTERVAL {
                self.probe_dualsense();
            }

            if let Some(ds) = self.dualsense.as_mut() {
                let mut buf = [0u8; 128];
                let mut latest = None;
                let mut disconnected = false;
                for _ in 0..8 {
                    match ds.device.read(&mut buf) {
                        Ok(0) => break,
                        Ok(n) => {
                            let pkt = &buf[..n];
                            if pkt[0] == 0x31 || n > 64 {
                                ds.transport = DualSenseTransport::Bluetooth;
                            } else if pkt[0] == 0x01 && n == 64 && ds.transport == DualSenseTransport::Unknown {
                                ds.transport = DualSenseTransport::Usb;
                            }
                            if let Some(state) = parse_dualsense_input_report(pkt, 0) {
                                latest = Some(state);
                            }
                        }
                        Err(_) => {
                            disconnected = true;
                            break;
                        }
                    }
                }
                if disconnected {
                    self.dualsense = None;
                } else if let Some(state) = latest {
                    ds.last_state = Some(state.clone());
                    return Some(state);
                } else if let Some(cached) = &ds.last_state {
                    return Some(cached.clone());
                }
            }

            self.poll_gilrs()
        }

        fn poll_gilrs(&mut self) -> Option<GamepadState> {
            let gilrs = self.gilrs.as_mut()?;
            while gilrs.next_event().is_some() {}

            let (_, gp) = gilrs
                .gamepads()
                .filter(|(_, gp)| gp.is_connected())
                .max_by_key(|(_, gp)| {
                    if gp.vendor_id() == Some(SONY_VID) {
                        2u8
                    } else {
                        1u8
                    }
                })?;

            let kind = match gp.vendor_id() {
                Some(SONY_VID) => match gp.product_id() {
                    Some(DUALSENSE_PID | DUALSENSE_EDGE_PID) => ControllerKind::DualSense,
                    _ => ControllerKind::DualShock4,
                },
                Some(0x057E) => ControllerKind::SwitchPro,
                Some(0x045E) => ControllerKind::Xbox,
                _ => {
                    let name = gp.name().to_ascii_lowercase();
                    if name.contains("dualsense") {
                        ControllerKind::DualSense
                    } else if name.contains("dualshock") {
                        ControllerKind::DualShock4
                    } else if name.contains("switch") || name.contains("nintendo") {
                        ControllerKind::SwitchPro
                    } else if name.contains("xbox") || name.contains("xinput") {
                        ControllerKind::Xbox
                    } else {
                        ControllerKind::Generic
                    }
                }
            };

            let mut buttons = 0u32;
            let map = [
                (Button::South, button::CROSS),
                (Button::East, button::CIRCLE),
                (Button::West, button::SQUARE),
                (Button::North, button::TRIANGLE),
                (Button::DPadLeft, button::DPAD_LEFT),
                (Button::DPadRight, button::DPAD_RIGHT),
                (Button::DPadUp, button::DPAD_UP),
                (Button::DPadDown, button::DPAD_DOWN),
                (Button::LeftTrigger, button::L1),
                (Button::RightTrigger, button::R1),
                (Button::LeftThumb, button::L3),
                (Button::RightThumb, button::R3),
                (Button::Select, button::SHARE),
                (Button::Start, button::OPTIONS),
                (Button::Mode, button::PS),
            ];
            for &(src, dst) in &map {
                if gp.is_pressed(src) {
                    buttons |= dst;
                }
            }

            let dpad_x = gp.value(Axis::DPadX);
            let dpad_y = gp.value(Axis::DPadY);
            if dpad_x < -0.5 {
                buttons |= button::DPAD_LEFT;
            } else if dpad_x > 0.5 {
                buttons |= button::DPAD_RIGHT;
            }
            if dpad_y > 0.5 {
                buttons |= button::DPAD_UP;
            } else if dpad_y < -0.5 {
                buttons |= button::DPAD_DOWN;
            }

            let trigger_u8 = |btn: Button, axis: Axis| -> u8 {
                if let Some(data) = gp.button_data(btn) {
                    return (data.value().clamp(0.0, 1.0) * 255.0).round() as u8;
                }
                let ax = gp.value(axis);
                if ax > 0.01 {
                    return (ax.clamp(0.0, 1.0) * 255.0).round() as u8;
                }
                if gp.is_pressed(btn) { 255 } else { 0 }
            };

            let axis_i16 = |val: f32, invert: bool| -> i16 {
                let scaled = if invert { -val } else { val } * STICK_AXIS_MAX;
                scaled.clamp(-STICK_AXIS_MAX, STICK_AXIS_MAX).round() as i16
            };

            Some(GamepadState {
                controller_id: 0,
                kind,
                buttons,
                left_trigger: trigger_u8(Button::LeftTrigger2, Axis::LeftZ),
                right_trigger: trigger_u8(Button::RightTrigger2, Axis::RightZ),
                left_stick_x: axis_i16(gp.value(Axis::LeftStickX), false),
                left_stick_y: axis_i16(gp.value(Axis::LeftStickY), true),
                right_stick_x: axis_i16(gp.value(Axis::RightStickX), false),
                right_stick_y: axis_i16(gp.value(Axis::RightStickY), true),
                touches: [TouchPoint::default(), TouchPoint::default()],
                has_imu: false,
                gyro: [0.0; 3],
                accel: [0.0; 3],
                quat: [0.0, 0.0, 0.0, 1.0],
            })
        }

        /// Applies PS5 rumble + adaptive triggers to an attached DualSense controller.
        pub fn apply_feedback(&mut self, feedback: &GamepadFeedback) {
            if !self.feedback.apply(feedback) {
                return;
            }
            self.flush_dualsense_feedback();
        }

        pub fn reset_feedback(&mut self) {
            self.feedback.reset();
            self.flush_dualsense_feedback();
        }

        fn flush_dualsense_feedback(&mut self) {
            let Some(ds) = self.dualsense.as_mut() else {
                return;
            };
            let res = match ds.transport {
                DualSenseTransport::Bluetooth => {
                    let report = self.feedback.build_bt_output_report();
                    ds.device.write(&report)
                }
                DualSenseTransport::Usb | DualSenseTransport::Unknown => {
                    let report = self.feedback.build_usb_output_report();
                    ds.device.write(&report)
                }
            };
            if res.is_err() {
                self.dualsense = None;
            }
        }
    }

    impl Default for GamepadManager {
        fn default() -> Self {
            Self::new()
        }
    }

    /// Background RAII controller polling & feedback worker bound to a PS5 console stream.
    pub struct GamepadWorker {
        stop: Arc<AtomicBool>,
        join: Option<std::thread::JoinHandle<()>>,
    }

    impl GamepadWorker {
        pub fn spawn(
            can_send: Arc<AtomicBool>,
            mut feedback_rx: broadcast::Receiver<GamepadFeedback>,
            mut send_state: impl FnMut(GamepadState) -> bool + Send + 'static,
        ) -> Self {
            let stop = Arc::new(AtomicBool::new(false));
            let stop_flag = stop.clone();
            let join = std::thread::Builder::new()
                .name("weavelo-gamepad".into())
                .spawn(move || {
                    let mut manager = GamepadManager::new();
                    let mut last_sent: Option<GamepadState> = None;
                    let mut last_sent_at = Instant::now();
                    let mut was_allowed = false;

                    while !stop_flag.load(Relaxed) {
                        loop {
                            match feedback_rx.try_recv() {
                                Ok(fb) => manager.apply_feedback(&fb),
                                Err(broadcast::error::TryRecvError::Lagged(_)) => continue,
                                Err(_) => break,
                            }
                        }

                        let allowed = can_send.load(Relaxed);
                        if !allowed {
                            if was_allowed {
                                manager.reset_feedback();
                                let _ = send_state(GamepadState::default());
                                last_sent = None;
                            }
                            was_allowed = false;
                            std::thread::sleep(Duration::from_millis(16));
                            continue;
                        }
                        was_allowed = true;

                        if let Some(state) = manager.poll() {
                            let neutral = state == GamepadState::default();
                            let changed = last_sent.as_ref() != Some(&state);
                            let keepalive = !neutral && last_sent_at.elapsed() >= Duration::from_millis(40);
                            if (changed || keepalive) && send_state(state.clone()) {
                                last_sent = Some(state);
                                last_sent_at = Instant::now();
                            }
                        }
                        std::thread::sleep(Duration::from_millis(8));
                    }

                    manager.reset_feedback();
                    if was_allowed {
                        let _ = send_state(GamepadState::default());
                    }
                })
                .ok();
            Self { stop, join }
        }
    }

    impl Drop for GamepadWorker {
        fn drop(&mut self) {
            self.stop.store(true, Relaxed);
            if let Some(join) = self.join.take() {
                let _ = join.join();
            }
        }
    }
}

#[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]
pub use desktop::{GamepadManager, GamepadWorker};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_dualsense_usb_report_with_touchpad_and_imu() {
        let mut report = [0u8; 64];
        report[0] = 0x01;
        // Sticks + triggers (offset +1 from report[0])
        report[1] = 0; // left stick X full left -> -32767 (since (0-128)*257 = -32896 clamped -32767)
        report[2] = 255; // left stick Y full down -> +32767
        report[3] = 128; // right stick centered -> 0
        report[4] = 128;
        report[5] = 180; // L2 analog
        report[6] = 240; // R2 analog
        report[8] = 0x20 | 0x02; // Cross + DPAD Right
        report[9] = 0x01 | 0x20; // L1 + Options
        report[10] = 0x01 | 0x02; // PS + Touchpad click

        // Gyro X = 1000 LE at s[15..17] -> report[16..18]
        report[16..18].copy_from_slice(&1000i16.to_le_bytes());
        // Accel Y = 8192 LE (1g) at s[23..25] -> report[24..26]
        report[24..26].copy_from_slice(&8192i16.to_le_bytes());

        // Touch point 0 at s[32..36] -> report[33..37]: active, id=9, x=0x345 (837), y=0x120 (288)
        report[33] = 9;
        report[34] = 0x45;
        report[35] = 0x03 | (0x00 << 4);
        report[36] = 0x12;

        // Touch point 1 inactive
        report[37] = 0x80;

        let gs = parse_dualsense_input_report(&report, 0).expect("valid usb report");
        assert_eq!(gs.kind, ControllerKind::DualSense);
        assert_eq!(gs.left_stick_x, -32767);
        assert_eq!(gs.left_stick_y, 32767);
        assert_eq!(gs.right_stick_x, 0);
        assert_eq!(gs.right_stick_y, 0);
        assert_eq!(gs.left_trigger, 180);
        assert_eq!(gs.right_trigger, 240);
        assert_eq!(
            gs.buttons,
            button::DPAD_RIGHT | button::CROSS | button::L1 | button::OPTIONS | button::PS | button::TOUCHPAD
        );
        assert!(gs.has_imu);
        assert!((gs.accel[1] - 1.0).abs() < 1e-4);
        assert!(gs.gyro[0] > 0.5);
        assert!(gs.touches[0].active);
        assert_eq!(gs.touches[0].id, 9);
        assert_eq!((gs.touches[0].x, gs.touches[0].y), (0x345, 0x120));
        assert!(!gs.touches[1].active);
    }

    #[test]
    fn parses_dualsense_bt_extended_report() {
        let mut report = [0u8; 78];
        report[0] = 0x31;
        report[1] = 0x01;
        report[2] = 128;
        report[3] = 128;
        report[4] = 128;
        report[5] = 128;
        report[6] = 99;
        report[7] = 200;
        report[9] = 0x80 | 0x08; // Triangle + hat neutral (8)
        report[10] = 0x02; // R1
        report[11] = 0x01; // PS

        let gs = parse_dualsense_input_report(&report, 1).expect("valid bt extended report");
        assert_eq!(gs.controller_id, 1);
        assert_eq!(gs.left_trigger, 99);
        assert_eq!(gs.right_trigger, 200);
        assert_eq!(gs.buttons, button::TRIANGLE | button::R1 | button::PS);
    }

    #[test]
    fn builds_usb_and_bt_output_reports_with_crc32() {
        let mut fb = DualSenseFeedbackState::default();
        assert!(fb.apply(&GamepadFeedback::Rumble {
            controller_id: 0,
            left_large: 0xAA,
            right_small: 0x55,
        }));
        let left_fx = [0x26, 0x90, 0x03, 0xFF, 0, 0, 0, 0, 0, 0];
        let right_fx = [0x02, 0xFF, 0, 0, 0, 0, 0, 0, 0, 0];
        assert!(fb.apply(&GamepadFeedback::TriggerEffects {
            controller_id: 0,
            left: left_fx,
            right: right_fx,
        }));

        let usb = fb.build_usb_output_report();
        assert_eq!(usb[0], 0x02);
        assert_eq!(usb[1], 0x0F);
        assert_eq!(usb[3], 0x55);
        assert_eq!(usb[4], 0xAA);
        assert_eq!(&usb[11..21], &right_fx);
        assert_eq!(&usb[22..32], &left_fx);

        let bt = fb.build_bt_output_report();
        assert_eq!(bt[0], 0x31);
        assert_eq!(bt[1], 0x02);
        assert_eq!(&bt[2..49], &usb[1..48]);

        let mut hasher = Crc32Hasher::new();
        hasher.update(&[0xA2]);
        hasher.update(&bt[..74]);
        let expected_crc = hasher.finalize().to_le_bytes();
        assert_eq!(&bt[74..78], &expected_crc);

        let bt2 = fb.build_bt_output_report();
        assert_eq!(bt2[1], 0x12);
    }
}
