use serde::{Deserialize, Serialize};

/// Gesture information associated with one scroll delta.
///
/// The serialized form is one byte: phase in bits 0..=2 and momentum in
/// bits 3..=4. Bits 5..=7 are reserved and rejected when decoding.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "u8", into = "u8")]
pub struct ScrollGesture {
    pub phase: ScrollPhase,
    pub momentum: MomentumPhase,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum ScrollPhase {
    #[default]
    None,
    MayBegin,
    Began,
    Changed,
    Ended,
    Cancelled,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum MomentumPhase {
    #[default]
    None,
    Began,
    Changed,
    Ended,
}

impl ScrollGesture {
    /// Only a new gesture or an unphased wheel delta may take input ownership.
    pub fn can_take_control(self, delta_x: i32, delta_y: i32) -> bool {
        self.momentum == MomentumPhase::None
            && match self.phase {
                ScrollPhase::Began => true,
                ScrollPhase::None => delta_x != 0 || delta_y != 0,
                _ => false,
            }
    }

    /// Close a gesture interrupted by focus, source, or input ownership changes.
    pub fn cancellation(self) -> Option<Self> {
        if matches!(self.momentum, MomentumPhase::Began | MomentumPhase::Changed) {
            Some(Self {
                phase: ScrollPhase::None,
                momentum: MomentumPhase::Ended,
            })
        } else if matches!(
            self.phase,
            ScrollPhase::MayBegin | ScrollPhase::Began | ScrollPhase::Changed
        ) {
            Some(Self {
                phase: ScrollPhase::Cancelled,
                momentum: MomentumPhase::None,
            })
        } else {
            None
        }
    }
}

impl From<ScrollGesture> for u8 {
    fn from(gesture: ScrollGesture) -> Self {
        let phase = match gesture.phase {
            ScrollPhase::None => 0,
            ScrollPhase::MayBegin => 1,
            ScrollPhase::Began => 2,
            ScrollPhase::Changed => 3,
            ScrollPhase::Ended => 4,
            ScrollPhase::Cancelled => 5,
        };
        let momentum = match gesture.momentum {
            MomentumPhase::None => 0,
            MomentumPhase::Began => 1,
            MomentumPhase::Changed => 2,
            MomentumPhase::Ended => 3,
        };
        phase | (momentum << 3)
    }
}

impl TryFrom<u8> for ScrollGesture {
    type Error = &'static str;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        if value & !0b1_1111 != 0 {
            return Err("reserved scroll gesture bits must be zero");
        }
        let phase = match value & 0b111 {
            0 => ScrollPhase::None,
            1 => ScrollPhase::MayBegin,
            2 => ScrollPhase::Began,
            3 => ScrollPhase::Changed,
            4 => ScrollPhase::Ended,
            5 => ScrollPhase::Cancelled,
            _ => return Err("invalid scroll phase"),
        };
        let momentum = match (value >> 3) & 0b11 {
            0 => MomentumPhase::None,
            1 => MomentumPhase::Began,
            2 => MomentumPhase::Changed,
            3 => MomentumPhase::Ended,
            _ => return Err("invalid momentum phase"),
        };
        Ok(Self { phase, momentum })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gesture_is_a_single_byte_and_roundtrips_all_valid_phases() {
        let phases = [
            ScrollPhase::None,
            ScrollPhase::MayBegin,
            ScrollPhase::Began,
            ScrollPhase::Changed,
            ScrollPhase::Ended,
            ScrollPhase::Cancelled,
        ];
        let momentum_phases = [
            MomentumPhase::None,
            MomentumPhase::Began,
            MomentumPhase::Changed,
            MomentumPhase::Ended,
        ];

        for phase in phases {
            for momentum in momentum_phases {
                let gesture = ScrollGesture { phase, momentum };
                let bytes = bincode::serialize(&gesture).unwrap();
                assert_eq!(bytes.len(), 1);
                assert_eq!(
                    bincode::deserialize::<ScrollGesture>(&bytes).unwrap(),
                    gesture
                );
                assert_eq!(u8::from(gesture) & !0b1_1111, 0);
            }
        }
        assert_eq!(bincode::serialize(&ScrollGesture::default()).unwrap(), [0]);
    }

    #[test]
    fn decoder_rejects_reserved_bits_and_undefined_phase_codes() {
        for value in [6, 7, 30, 31, 32, 128, 255] {
            assert!(
                bincode::deserialize::<ScrollGesture>(&[value]).is_err(),
                "accepted {value:#04x}"
            );
        }
    }
}
