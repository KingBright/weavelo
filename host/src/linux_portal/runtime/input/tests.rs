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
    assert_eq!(portal_scroll_axis(0, 0, true, false), None);
    assert_eq!(
        portal_scroll_axis(12, -24, false, false),
        Some((12.0, -24.0))
    );
    assert_eq!(
        portal_scroll_axis(30, -60, true, false),
        Some((-30.0, 60.0))
    );
    assert_eq!(
        portal_scroll_axis(30, -60, true, true),
        Some((-30.0, -60.0))
    );
}

#[cfg(target_os = "linux")]
#[test]
fn eis_v120_accumulates_single_unit_remainders_and_avoids_continuous_detent_snap() {
    let mut rem = 0;
    assert_eq!(eis::step_eis_v120(-1, &mut rem, true), 0);
    assert_eq!(rem, 1);
    assert_eq!(eis::step_eis_v120(-1, &mut rem, true), 2);
    assert_eq!(rem, 0);
    assert_eq!(eis::step_eis_v120(-120, &mut rem, true), 119);
    assert_eq!(eis::step_eis_v120(-120, &mut rem, false), 120);
}
