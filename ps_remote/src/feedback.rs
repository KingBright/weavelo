use std::f32::consts::FRAC_1_SQRT_2;

/// DualSense / PS5 Remote Play controller state.
#[derive(Debug, Clone, PartialEq)]
pub struct Ps5ControllerState {
    pub buttons: u32,
    pub l2_state: u8,
    pub r2_state: u8,
    pub left_x: i16,
    pub left_y: i16,
    pub right_x: i16,
    pub right_y: i16,
    pub touches: [TouchPoint; 2],
    pub gyro_x: f32,
    pub gyro_y: f32,
    pub gyro_z: f32,
    pub accel_x: f32,
    pub accel_y: f32,
    pub accel_z: f32,
    /// Orientation quaternion `[x, y, z, w]`.
    pub quat: [f32; 4],
}

impl Default for Ps5ControllerState {
    fn default() -> Self {
        Self {
            buttons: 0,
            l2_state: 0,
            r2_state: 0,
            left_x: 0,
            left_y: 0,
            right_x: 0,
            right_y: 0,
            touches: [TouchPoint::default(), TouchPoint::default()],
            gyro_x: 0.0,
            gyro_y: 0.0,
            gyro_z: 0.0,
            accel_x: 0.0,
            accel_y: 0.0,
            accel_z: 0.0,
            quat: [0.0, 0.0, 0.0, 1.0],
        }
    }
}

/// Touchpad contact point (`1920x942` 12-bit DualSense coordinate space).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TouchPoint {
    /// `-1` when inactive, `0..=127` when active.
    pub id: i16,
    pub x: u16,
    pub y: u16,
}

impl Default for TouchPoint {
    fn default() -> Self {
        Self {
            id: -1,
            x: 0,
            y: 0,
        }
    }
}

/// Digital button bitmask constants matching PS5 Remote Play feedback state.
pub mod button {
    pub const CROSS: u32 = 1 << 0;
    pub const MOON: u32 = 1 << 1;
    pub const BOX: u32 = 1 << 2;
    pub const PYRAMID: u32 = 1 << 3;
    pub const DPAD_LEFT: u32 = 1 << 4;
    pub const DPAD_RIGHT: u32 = 1 << 5;
    pub const DPAD_UP: u32 = 1 << 6;
    pub const DPAD_DOWN: u32 = 1 << 7;
    pub const L1: u32 = 1 << 8;
    pub const R1: u32 = 1 << 9;
    pub const L3: u32 = 1 << 10;
    pub const R3: u32 = 1 << 11;
    pub const OPTIONS: u32 = 1 << 12;
    pub const SHARE: u32 = 1 << 13;
    pub const TOUCHPAD: u32 = 1 << 14;
    pub const PS: u32 = 1 << 15;
}

/// Inbound PS5 controller rumble motor intensities.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RumbleEvent {
    pub unknown: u8,
    pub left: u8,
    pub right: u8,
}

/// Inbound PS5 DualSense L2 / R2 adaptive trigger effect payload (10 bytes each).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TriggerEffectsEvent {
    pub left: [u8; 10],
    pub right: [u8; 10],
}

const GYRO_MIN: f32 = -30.0;
const GYRO_MAX: f32 = 30.0;
const ACCEL_MIN: f32 = -5.0;
const ACCEL_MAX: f32 = 5.0;

fn encode_imu_axis(val: f32, min: f32, max: f32) -> u16 {
    let clamped = val.clamp(min, max);
    ((u16::MAX as f32) * (clamped - min) / (max - min)) as u16
}

/// Compress a unit quaternion `[x, y, z, w]` into 32 bits (`feedback.c:compress_quat`).
pub fn compress_quat(q: [f32; 4]) -> u32 {
    let mut largest_i = 0usize;
    let mut largest_abs = -1.0_f32;
    for (i, &v) in q.iter().enumerate() {
        let av = v.abs();
        if av > largest_abs {
            largest_abs = av;
            largest_i = i;
        }
    }

    let sign_neg = u32::from(q[largest_i] < 0.0);
    let mut r = ((largest_i as u32) << 1) | sign_neg;
    let mut shift = 3u32;
    for (i, &v) in q.iter().enumerate() {
        if i == largest_i {
            continue;
        }
        let clamped = v.clamp(-FRAC_1_SQRT_2, FRAC_1_SQRT_2);
        let scaled = ((0x1ff as f32) * (clamped - (-FRAC_1_SQRT_2)) / (2.0 * FRAC_1_SQRT_2)) as u32;
        r |= (scaled & 0x1ff) << shift;
        shift += 9;
    }
    r
}

/// Format a 28-byte Takion v12 `FEEDBACK_STATE` payload (`feedback.c:16-85`).
pub fn format_feedback_state_v12(state: &Ps5ControllerState) -> [u8; 28] {
    let mut buf = [0u8; 28];
    buf[0] = 0xa0;

    buf[1..3].copy_from_slice(&encode_imu_axis(state.gyro_x, GYRO_MIN, GYRO_MAX).to_le_bytes());
    buf[3..5].copy_from_slice(&encode_imu_axis(state.gyro_y, GYRO_MIN, GYRO_MAX).to_le_bytes());
    buf[5..7].copy_from_slice(&encode_imu_axis(state.gyro_z, GYRO_MIN, GYRO_MAX).to_le_bytes());

    buf[7..9].copy_from_slice(&encode_imu_axis(state.accel_x, ACCEL_MIN, ACCEL_MAX).to_le_bytes());
    buf[9..11].copy_from_slice(&encode_imu_axis(state.accel_y, ACCEL_MIN, ACCEL_MAX).to_le_bytes());
    buf[11..13].copy_from_slice(&encode_imu_axis(state.accel_z, ACCEL_MIN, ACCEL_MAX).to_le_bytes());

    buf[13..17].copy_from_slice(&compress_quat(state.quat).to_le_bytes());

    buf[17..19].copy_from_slice(&(state.left_x as u16).to_be_bytes());
    buf[19..21].copy_from_slice(&(state.left_y as u16).to_be_bytes());
    buf[21..23].copy_from_slice(&(state.right_x as u16).to_be_bytes());
    buf[23..25].copy_from_slice(&(state.right_y as u16).to_be_bytes());

    buf[25] = 0x00;
    buf[26] = 0x00;
    buf[27] = 0x01;
    buf
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FeedbackHistoryEvent {
    Button3 { opcode: u8, value: u8 },
    Button2 { opcode: u8 },
    Touch { down: bool, id: u8, x: u16, y: u16 },
}

impl FeedbackHistoryEvent {
    fn encode_into(&self, out: &mut Vec<u8>) {
        match *self {
            Self::Button3 { opcode, value } => {
                out.extend_from_slice(&[0x80, opcode, value]);
            }
            Self::Button2 { opcode } => {
                out.extend_from_slice(&[0x80, opcode]);
            }
            Self::Touch { down, id, x, y } => {
                let x = x.min(1919);
                let y = y.min(941);
                out.push(if down { 0xd0 } else { 0xc0 });
                out.push(id & 0x7f);
                out.push((x >> 4) as u8);
                out.push((((x & 0x0f) << 4) as u8) | ((y >> 8) as u8));
                out.push((y & 0xff) as u8);
            }
        }
    }
}

const DIGITAL_BUTTONS_3BYTE: &[(u32, u8)] = &[
    (button::DPAD_UP, 0x80),
    (button::DPAD_DOWN, 0x81),
    (button::DPAD_LEFT, 0x82),
    (button::DPAD_RIGHT, 0x83),
    (button::L1, 0x84),
    (button::R1, 0x85),
    (button::CROSS, 0x88),
    (button::MOON, 0x89),
    (button::BOX, 0x8a),
    (button::PYRAMID, 0x8b),
];

const DIGITAL_BUTTONS_2BYTE: &[(u32, u8, u8)] = &[
    (button::OPTIONS, 0xac, 0x8c),
    (button::SHARE, 0xad, 0x8d),
    (button::PS, 0xae, 0x8e),
    (button::L3, 0xaf, 0x8f),
    (button::R3, 0xb0, 0x90),
    (button::TOUCHPAD, 0xb1, 0x91),
];

const OPCODE_L2: u8 = 0x86;
const OPCODE_R2: u8 = 0x87;

/// Ring buffer storing discrete controller transitions newest-first for Takion `FEEDBACK_HISTORY`.
#[derive(Debug, Clone)]
pub struct FeedbackHistoryBuffer {
    events: Vec<Option<FeedbackHistoryEvent>>,
    begin: usize,
    len: usize,
}

impl Default for FeedbackHistoryBuffer {
    fn default() -> Self {
        Self::new(16)
    }
}

impl FeedbackHistoryBuffer {
    pub fn new(capacity: usize) -> Self {
        let cap = capacity.max(1);
        Self {
            events: vec![None; cap],
            begin: 0,
            len: 0,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn push(&mut self, event: FeedbackHistoryEvent) {
        let cap = self.events.len();
        self.begin = (self.begin + cap - 1) % cap;
        self.len = (self.len + 1).min(cap);
        self.events[self.begin] = Some(event);
    }

    /// Record discrete state changes between `prev` and `now` in exact Chiaki order:
    /// 1. released digital buttons
    /// 2. touch events (up then down/move)
    /// 3. pressed digital buttons
    /// 4. L2 analog change
    /// 5. R2 analog change
    pub fn record_state(&mut self, prev: &Ps5ControllerState, now: &Ps5ControllerState) -> bool {
        let before = self.len;
        let diff = prev.buttons ^ now.buttons;

        // 1. Released digital buttons
        if diff != 0 {
            for &(mask, opcode) in DIGITAL_BUTTONS_3BYTE {
                if (diff & mask) != 0 && (now.buttons & mask) == 0 {
                    self.push(FeedbackHistoryEvent::Button3 {
                        opcode,
                        value: 0x00,
                    });
                }
            }
            for &(mask, _on, off) in DIGITAL_BUTTONS_2BYTE {
                if (diff & mask) != 0 && (now.buttons & mask) == 0 {
                    self.push(FeedbackHistoryEvent::Button2 { opcode: off });
                }
            }
        }

        // 2. Touches
        for i in 0..2 {
            let p = prev.touches[i];
            let n = now.touches[i];
            if p.id >= 0 && p.id != n.id {
                self.push(FeedbackHistoryEvent::Touch {
                    down: false,
                    id: p.id as u8,
                    x: p.x,
                    y: p.y,
                });
            }
            if n.id >= 0 && n != p {
                self.push(FeedbackHistoryEvent::Touch {
                    down: true,
                    id: n.id as u8,
                    x: n.x,
                    y: n.y,
                });
            }
        }

        // 3. Pressed digital buttons
        if diff != 0 {
            for &(mask, opcode) in DIGITAL_BUTTONS_3BYTE {
                if (diff & mask) != 0 && (now.buttons & mask) != 0 {
                    self.push(FeedbackHistoryEvent::Button3 {
                        opcode,
                        value: 0xff,
                    });
                }
            }
            for &(mask, on, _off) in DIGITAL_BUTTONS_2BYTE {
                if (diff & mask) != 0 && (now.buttons & mask) != 0 {
                    self.push(FeedbackHistoryEvent::Button2 { opcode: on });
                }
            }
        }

        // 4. L2
        if prev.l2_state != now.l2_state {
            self.push(FeedbackHistoryEvent::Button3 {
                opcode: OPCODE_L2,
                value: now.l2_state,
            });
        }

        // 5. R2
        if prev.r2_state != now.r2_state {
            self.push(FeedbackHistoryEvent::Button3 {
                opcode: OPCODE_R2,
                value: now.r2_state,
            });
        }

        self.len != before || prev.l2_state != now.l2_state || prev.r2_state != now.r2_state
    }

    /// Encode buffered events newest-first, returning `(event_count, payload_bytes)`.
    pub fn format(&self) -> (usize, Vec<u8>) {
        let cap = self.events.len();
        let mut out = Vec::with_capacity(self.len * 4);
        for i in 0..self.len {
            let idx = (self.begin + i) % cap;
            if let Some(ev) = &self.events[idx] {
                ev.encode_into(&mut out);
            }
        }
        (self.len, out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn feedback_state_v12_layout_matches_spec() {
        let mut st = Ps5ControllerState::default();
        st.left_x = 0x1234;
        st.left_y = -1; // 0xffff
        st.right_x = -0x8000;
        st.right_y = 0x7fff;
        let bytes = format_feedback_state_v12(&st);

        assert_eq!(bytes[0], 0xa0);
        // Zero gyro -> 0x7fff LE
        assert_eq!(&bytes[1..3], &0x7fff_u16.to_le_bytes());
        assert_eq!(&bytes[7..9], &0x7fff_u16.to_le_bytes());
        // Quaternion [0, 0, 0, 1] -> 6 | (0xff << 3) | (0xff << 12) | (0xff << 21)
        let expected_q: u32 = 6 | (0xff << 3) | (0xff << 12) | (0xff << 21);
        assert_eq!(&bytes[13..17], &expected_q.to_le_bytes());
        // Sticks BE
        assert_eq!(&bytes[17..19], &0x1234_u16.to_be_bytes());
        assert_eq!(&bytes[19..21], &0xffff_u16.to_be_bytes());
        assert_eq!(&bytes[21..23], &0x8000_u16.to_be_bytes());
        assert_eq!(&bytes[23..25], &0x7fff_u16.to_be_bytes());
        // v12 trailer
        assert_eq!(&bytes[25..28], &[0x00, 0x00, 0x01]);
    }

    #[test]
    fn feedback_history_records_in_newest_first_ring_order() {
        let mut hist = FeedbackHistoryBuffer::new(16);
        let prev = Ps5ControllerState {
            buttons: button::OPTIONS,
            touches: [
                TouchPoint { id: 1, x: 100, y: 200 },
                TouchPoint::default(),
            ],
            ..Default::default()
        };
        let now = Ps5ControllerState {
            buttons: button::CROSS,
            l2_state: 0x80,
            r2_state: 0xff,
            touches: [TouchPoint::default(), TouchPoint::default()],
            ..Default::default()
        };

        hist.record_state(&prev, &now);
        let (count, encoded) = hist.format();
        // Events pushed in order:
        // 1. release OPTIONS [0x80, 0x8c]
        // 2. touch up id=1 x=100 y=200 [0xc0, 0x01, 0x06, 0x40, 0xc8]
        // 3. press CROSS [0x80, 0x88, 0xff]
        // 4. L2=0x80 [0x80, 0x86, 0x80]
        // 5. R2=0xff [0x80, 0x87, 0xff]
        // Ring outputs newest-first: R2, L2, CROSS, touch_up, OPTIONS
        assert_eq!(count, 5);
        assert_eq!(
            encoded,
            vec![
                0x80, 0x87, 0xff, // R2
                0x80, 0x86, 0x80, // L2
                0x80, 0x88, 0xff, // CROSS
                0xc0, 0x01, 0x06, 0x40, 0xc8, // touch up
                0x80, 0x8c, // release OPTIONS
            ]
        );
    }
}
