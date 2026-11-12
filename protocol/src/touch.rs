//! Shared native-touch geometry and bounded, atomic input samples.
use crate::TouchAction;
use serde::{Deserialize, Serialize};

pub const MAX_CONTACTS: usize = 10;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Contact {
    pub action: TouchAction,
    pub pointer_id: u32,
    pub normalized_x: f32,
    pub normalized_y: f32,
    pub pressure: f32,
}

pub fn valid_frame(contacts: &[Contact]) -> bool {
    !contacts.is_empty()
        && contacts.len() <= MAX_CONTACTS
        && contacts.iter().enumerate().all(|(index, contact)| {
            contacts[..index]
                .iter()
                .all(|other| other.pointer_id != contact.pointer_id)
                && [contact.normalized_x, contact.normalized_y, contact.pressure]
                    .into_iter()
                    .all(|value| value.is_finite() && (0.0..=1.0).contains(&value))
        })
}

pub fn only_moves(events: &[crate::InputEvent]) -> Option<&[Contact]> {
    match events {
        [crate::InputEvent::TouchFrame(contacts)]
            if valid_frame(contacts) && contacts.iter().all(|c| c.action == TouchAction::Move) =>
        {
            Some(contacts)
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn samples_are_bounded_unique_and_finite() {
        let contact = Contact {
            action: TouchAction::Down,
            pointer_id: 7,
            normalized_x: 0.2,
            normalized_y: 0.8,
            pressure: 1.0,
        };
        assert!(valid_frame(&[contact]));
        assert!(!valid_frame(&[]));
        assert!(!valid_frame(&[contact, contact]));
        assert!(!valid_frame(&[Contact {
            normalized_x: f32::NAN,
            ..contact
        }]));
        assert!(!valid_frame(&[Contact {
            pressure: 1.1,
            ..contact
        }]));
        let ten: Vec<_> = (0..MAX_CONTACTS)
            .map(|id| Contact {
                pointer_id: id as u32,
                ..contact
            })
            .collect();
        assert!(valid_frame(&ten));
        let mut eleven = ten;
        eleven.push(Contact {
            pointer_id: 99,
            ..contact
        });
        assert!(!valid_frame(&eleven));
    }
}
