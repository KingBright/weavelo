use super::*;

#[test]
fn unknown_keys_and_buttons_never_become_printable_keys_or_left_clicks() {
    assert_eq!(key_code(0x00), Ok(30));
    assert_eq!(key_code(0x7a), Ok(59));
    assert_eq!(key_code(0x6f), Ok(88));
    assert_eq!(key_code(0xffff), Err(PortalInputError::InvalidEvent));
    assert_eq!(button_code(5), Err(PortalInputError::InvalidEvent));
}

#[test]
fn portal_scroll_axis_matches_wayland_direction_and_skips_zero_phase_markers() {
    assert_eq!(portal_scroll_axis(0, 0), None);
    assert_eq!(portal_scroll_axis(0, -120), Some((0.0, 40.0)));
    assert_eq!(portal_scroll_axis(30, 60), Some((-10.0, -20.0)));
}
