use serde::{Deserialize, Serialize};

/// Canonical PlayStation / universal controller bitmask flags (aligned with `ps_remote::feedback::button`).
pub mod button {
    pub const CROSS: u32 = 1 << 0;
    pub const CIRCLE: u32 = 1 << 1;
    pub const MOON: u32 = CIRCLE;
    pub const SQUARE: u32 = 1 << 2;
    pub const BOX: u32 = SQUARE;
    pub const TRIANGLE: u32 = 1 << 3;
    pub const PYRAMID: u32 = TRIANGLE;
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
    pub const CREATE: u32 = SHARE;
    pub const TOUCHPAD: u32 = 1 << 14;
    pub const PS: u32 = 1 << 15;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum ControllerKind {
    DualSense,
    DualShock4,
    Xbox,
    SwitchPro,
    #[default]
    Generic,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TouchPoint {
    pub active: bool,
    pub id: u8,
    pub x: u16,
    pub y: u16,
}

impl Default for TouchPoint {
    fn default() -> Self {
        Self {
            active: false,
            id: 0,
            x: 0,
            y: 0,
        }
    }
}

/// Cross-platform gamepad snapshot sent over `InputEvent::Gamepad` from client to host.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GamepadState {
    pub controller_id: u8,
    pub kind: ControllerKind,
    pub buttons: u32,
    pub left_trigger: u8,
    pub right_trigger: u8,
    pub left_stick_x: i16,
    pub left_stick_y: i16,
    pub right_stick_x: i16,
    pub right_stick_y: i16,
    pub touches: [TouchPoint; 2],
    pub has_imu: bool,
    pub gyro: [f32; 3],
    pub accel: [f32; 3],
    pub quat: [f32; 4],
}

impl Default for GamepadState {
    fn default() -> Self {
        Self {
            controller_id: 0,
            kind: ControllerKind::Generic,
            buttons: 0,
            left_trigger: 0,
            right_trigger: 0,
            left_stick_x: 0,
            left_stick_y: 0,
            right_stick_x: 0,
            right_stick_y: 0,
            touches: [TouchPoint::default(), TouchPoint::default()],
            has_imu: false,
            gyro: [0.0; 3],
            accel: [0.0; 3],
            quat: [0.0, 0.0, 0.0, 1.0],
        }
    }
}

/// Host -> client haptic rumble & DualSense adaptive trigger effect commands.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum GamepadFeedback {
    Rumble {
        controller_id: u8,
        left_large: u8,
        right_small: u8,
    },
    TriggerEffects {
        controller_id: u8,
        left: [u8; 10],
        right: [u8; 10],
    },
    AudioHaptics {
        controller_id: u8,
        payload: Vec<u8>,
    },
}
