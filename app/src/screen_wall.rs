//! Multi-device / multi-window interactive Screen Wall ("屏幕墙") and unified
//! titlebar tab strip coordination. Reuses each stream window's live decoded
//! frames and `OriginalOwner` without duplicating network sessions or decoders.
use super::*;
use std::collections::BTreeMap;

static WALL_VISIBLE: AtomicBool = AtomicBool::new(false);
static WALL_FOCUSED_WINDOW: Mutex<Option<AnyWindowHandle>> = Mutex::new(None);

pub(super) fn is_wall_visible() -> bool {
    WALL_VISIBLE.load(Ordering::Acquire)
}

pub(super) fn is_wall_focused_window(handle: AnyWindowHandle) -> bool {
    WALL_VISIBLE.load(Ordering::Acquire)
        && *WALL_FOCUSED_WINDOW
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            == Some(handle)
}

fn set_wall_focused_window(handle: Option<AnyWindowHandle>) {
    *WALL_FOCUSED_WINDOW
        .lock()
        .unwrap_or_else(|e| e.into_inner()) = handle;
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum WallLayoutMode {
    #[default]
    AutoGrid,
    Spotlight,
}

#[derive(Clone, PartialEq, Eq)]
pub(super) struct StreamWallEntry {
    pub entity_id: EntityId,
    pub view: WeakEntity<RestoredDashboard>,
    pub window_handle: AnyWindowHandle,
    pub device_id: String,
    pub device_name: String,
    pub title: String,
    pub is_window: bool,
    pub video_id: Option<u32>,
}

#[derive(Default)]
pub(super) struct ScreenWallRegistry {
    entries: Vec<StreamWallEntry>,
    pub wall_window: Option<AnyWindowHandle>,
    pub spotlight_entity: Option<EntityId>,
}

impl Global for ScreenWallRegistry {}

impl ScreenWallRegistry {
    pub(super) fn register_or_update(&mut self, entry: StreamWallEntry) -> bool {
        if let Some(existing) = self
            .entries
            .iter_mut()
            .find(|item| item.entity_id == entry.entity_id)
        {
            if *existing == entry {
                return false;
            }
            *existing = entry;
            return true;
        }
        self.entries.push(entry);
        true
    }

    pub(super) fn unregister(&mut self, entity_id: EntityId) {
        self.entries.retain(|item| item.entity_id != entity_id);
        if self.spotlight_entity == Some(entity_id) {
            self.spotlight_entity = self.entries.first().map(|item| item.entity_id);
        }
    }

    pub(super) fn entries(&self) -> &[StreamWallEntry] {
        &self.entries
    }

    pub(super) fn move_entry(&mut self, entity_id: EntityId, delta: isize) -> bool {
        let Some(pos) = self.entries.iter().position(|e| e.entity_id == entity_id) else {
            return false;
        };
        let next = pos as isize + delta;
        if next < 0 || next >= self.entries.len() as isize {
            return false;
        }
        self.entries.swap(pos, next as usize);
        true
    }

    pub(super) fn promote_spotlight(&mut self, entity_id: EntityId) {
        if self.entries.iter().any(|e| e.entity_id == entity_id) {
            self.spotlight_entity = Some(entity_id);
        }
    }

    pub(super) fn resolved_spotlight(&self) -> Option<EntityId> {
        self.spotlight_entity
            .filter(|id| self.entries.iter().any(|e| e.entity_id == *id))
            .or_else(|| self.entries.first().map(|e| e.entity_id))
    }

    pub(super) fn same_device_window_batches(&self) -> Vec<(String, EntityId, Vec<u32>)> {
        let items: Vec<(String, EntityId, bool, Option<u32>)> = self
            .entries
            .iter()
            .map(|e| (e.device_id.clone(), e.entity_id, e.is_window, e.video_id))
            .collect();
        group_same_device_batches(&items)
    }
}

pub(super) fn registry_entries(cx: &App) -> Vec<StreamWallEntry> {
    cx.try_global::<ScreenWallRegistry>()
        .map(|reg| reg.entries().to_vec())
        .unwrap_or_default()
}

pub(super) fn register_stream_entry(cx: &mut App, entry: StreamWallEntry) {
    if !cx.has_global::<ScreenWallRegistry>() {
        cx.set_global(ScreenWallRegistry::default());
    }
    cx.update_global::<ScreenWallRegistry, _>(|reg, _| {
        reg.register_or_update(entry);
    });
}

pub(super) fn unregister_stream_entry(cx: &mut App, entity_id: EntityId) {
    if !cx.has_global::<ScreenWallRegistry>() {
        return;
    }
    cx.update_global::<ScreenWallRegistry, _>(|reg, _| {
        reg.unregister(entity_id);
    });
}

pub(super) fn device_color_index(device_id: &str) -> usize {
    let mut hash = 2166136261u32;
    for byte in device_id.as_bytes() {
        hash ^= u32::from(*byte);
        hash = hash.wrapping_mul(16777619);
    }
    (hash as usize) % 6
}

fn device_accent_color(device_id: &str, cx: &App) -> Hsla {
    match device_color_index(device_id) {
        0 => theme_accent_brand(cx),
        1 => theme_accent_cyan(cx),
        2 => theme_accent_emerald(cx),
        3 => theme_accent_indigo(cx),
        4 => theme_accent_amber(cx),
        _ => theme_accent_rose(cx),
    }
}

/// Compute `(columns, rows)` for N live video cards inside the Screen Wall.
pub(super) fn wall_grid_dimensions(count: usize) -> (usize, usize) {
    match count {
        0 | 1 => (1, 1),
        2 => (2, 1),
        3 | 4 => (2, 2),
        5 | 6 => (3, 2),
        n => {
            let cols = (n as f64).sqrt().ceil() as usize;
            let rows = n.div_ceil(cols);
            (cols, rows)
        }
    }
}

pub(super) fn group_same_device_batches<Id: Copy>(
    items: &[(String, Id, bool, Option<u32>)],
) -> Vec<(String, Id, Vec<u32>)> {
    let mut by_device: BTreeMap<String, Vec<(Id, u32)>> = BTreeMap::new();
    for (device, id, is_window, video_id) in items {
        if device.is_empty() || !*is_window {
            continue;
        }
        let Some(vid) = *video_id else {
            continue;
        };
        by_device.entry(device.clone()).or_default().push((*id, vid));
    }
    by_device
        .into_iter()
        .filter_map(|(device, entries)| {
            if entries.len() < 2 {
                return None;
            }
            let primary_id = entries[0].0;
            let video_ids = entries.into_iter().map(|(_, vid)| vid).collect();
            Some((device, primary_id, video_ids))
        })
        .collect()
}

pub(super) fn cycle_index(count: usize, current: usize, delta: isize) -> Option<usize> {
    if count == 0 {
        return None;
    }
    let cur = current.min(count - 1) as isize;
    Some((cur + delta).rem_euclid(count as isize) as usize)
}

pub(super) fn activate_stream_tab(entry: &StreamWallEntry, cx: &mut App) {
    cx.activate(true);
    let _ = entry.window_handle.update(cx, |_, window, _| {
        window.activate_window();
    });
    let _ = entry.view.update(cx, |this, cx| {
        this.request_remote_source_focus();
        cx.notify();
    });
}

pub(super) fn switch_tab_by_index(index: usize, cx: &mut App) -> bool {
    let entries = registry_entries(cx);
    let Some(entry) = entries.get(index).cloned() else {
        return false;
    };
    activate_stream_tab(&entry, cx);
    true
}

pub(super) fn switch_tab_by_delta(
    current_entity: EntityId,
    delta: isize,
    cx: &mut App,
) -> bool {
    let entries = registry_entries(cx);
    if entries.len() <= 1 {
        return false;
    }
    let cur = entries
        .iter()
        .position(|e| e.entity_id == current_entity)
        .unwrap_or(0);
    let Some(next_idx) = cycle_index(entries.len(), cur, delta) else {
        return false;
    };
    activate_stream_tab(&entries[next_idx], cx);
    true
}

pub(super) fn open_or_focus_screen_wall(cx: &mut App) {
    if !cx.has_global::<ScreenWallRegistry>() {
        cx.set_global(ScreenWallRegistry::default());
    }
    let existing = cx
        .global::<ScreenWallRegistry>()
        .wall_window
        .filter(|handle| {
            handle
                .update(cx, |_, window, _| {
                    window.activate_window();
                })
                .is_ok()
        });
    if existing.is_some() {
        cx.activate(true);
        WALL_VISIBLE.store(true, Ordering::Release);
        return;
    }

    let options = restored_window_options(cx, WindowPurpose::ScreenWall);
    let result = cx.open_window(options, |window, cx| {
        window.set_window_title(&format!("Weavelo · {}", crate::i18n::tr("Screen Wall")));
        window_icon::apply(window, cx);
        let window_handle = window.window_handle();
        let initial_focus = registry_entries(cx).first().map(|e| e.entity_id);
        let view = cx.new(|cx| ScreenWallView {
            window_handle,
            input_focus: cx.focus_handle(),
            root_focus: cx.focus_handle(),
            layout_mode: WallLayoutMode::AutoGrid,
            focused_card: initial_focus,
            active_owner: None,
            card_bounds: Arc::new(Mutex::new(BTreeMap::new())),
            last_notice: None,
            window_active: window.is_window_active(),
            window_visible: window.is_window_visible().unwrap_or(true),
        });
        cx.update_global::<ScreenWallRegistry, _>(|reg, _| {
            reg.wall_window = Some(window_handle);
        });
        WALL_VISIBLE.store(true, Ordering::Release);

        view.update(cx, |_, cx| {
            cx.observe_window_activation(window, |this, window, cx| {
                this.window_active = window.is_window_active();
                this.window_visible = window.is_window_visible().unwrap_or(true);
                WALL_VISIBLE.store(this.window_visible, Ordering::Release);
                if !this.window_active {
                    this.release_active_card_input();
                } else if let Some(focused) = this.focused_card
                    && let Some(entry) = registry_entries(cx)
                        .into_iter()
                        .find(|e| e.entity_id == focused)
                {
                    this.activate_card_focus(&entry, window, cx);
                }
                cx.notify();
            })
            .detach();
        });

        window.on_window_should_close(cx, {
            let weak = view.downgrade();
            move |_, cx| {
                let _ = weak.update(cx, |this, cx| {
                    this.release_active_card_input();
                    WALL_VISIBLE.store(false, Ordering::Release);
                    set_wall_focused_window(None);
                    if cx.has_global::<ScreenWallRegistry>() {
                        cx.update_global::<ScreenWallRegistry, _>(|reg, _| {
                            if reg.wall_window == Some(this.window_handle) {
                                reg.wall_window = None;
                            }
                        });
                    }
                });
                true
            }
        });

        window.focus(&view.read(cx).input_focus);
        cx.spawn({
            let weak = view.downgrade();
            async move |cx| loop {
                let Ok(delay) = weak.update(&mut *cx, |this, cx| {
                    let entries = registry_entries(cx);
                    let any_streaming = entries.iter().any(|e| {
                        e.view
                            .upgrade()
                            .is_some_and(|v| v.read(cx).current_frame.is_some())
                    });
                    cx.notify();
                    if this.window_visible && any_streaming {
                        Duration::from_millis(16)
                    } else {
                        Duration::from_millis(100)
                    }
                }) else {
                    break;
                };
                Timer::after(delay).await;
            }
        })
        .detach();
        view
    });
    if let Err(err) = result {
        eprintln!("Could not open Screen Wall window: {err}");
    } else {
        cx.activate(true);
    }
}

pub(super) fn render_titlebar_tab_strip(
    current_entity: EntityId,
    cx: &App,
) -> Option<AnyElement> {
    let entries = registry_entries(cx);
    if entries.len() < 2 {
        return None;
    }
    let theme = cx.theme().clone();
    let max_tabs = 6usize;
    let extra = entries.len().saturating_sub(max_tabs);
    let mut strip = div()
        .id("titlebar-window-tab-strip")
        .occlude()
        .h(px(24.0))
        .flex()
        .items_center()
        .gap_1()
        .px_1()
        .rounded_full()
        .bg(theme.surface.sunken)
        .border_1()
        .border_color(theme.border.divider)
        .on_any_mouse_down(|_, _, cx| cx.stop_propagation());

    for entry in entries.into_iter().take(max_tabs) {
        let active = entry.entity_id == current_entity;
        let accent = device_accent_color(&entry.device_id, cx);
        let tip = if entry.device_name.is_empty() {
            entry.title.clone()
        } else {
            format!("{} · {}", entry.device_name, entry.title)
        };
        let label = entry.title.clone();
        let entry_for_click = entry.clone();
        strip = strip.child(
            div()
                .id(SharedString::from(format!("titlebar-tab-{}", entry.entity_id.as_u64())))
                .h(px(20.0))
                .max_w(px(140.0))
                .flex()
                .items_center()
                .gap_1()
                .px_2()
                .rounded_full()
                .cursor_pointer()
                .when(active, |pill| {
                    pill.bg(theme.surface.raised)
                        .border_1()
                        .border_color(accent)
                        .text_color(theme.content.primary)
                        .font_weight(FontWeight::SEMIBOLD)
                })
                .when(!active, |pill| {
                    pill.text_color(theme.content.secondary)
                        .hover(|s| s.bg(theme.surface.base).text_color(theme.content.primary))
                })
                .text_size(px(10.5))
                .tooltip(tooltip(tip).build())
                .on_click(move |_, _, cx| {
                    activate_stream_tab(&entry_for_click, cx);
                })
                .child(div().size(px(6.0)).flex_none().rounded_full().bg(accent))
                .child(div().min_w_0().truncate().child(label)),
        );
    }

    if extra > 0 {
        strip = strip.child(
            div()
                .id("titlebar-tab-more-wall")
                .h(px(20.0))
                .px_1p5()
                .flex()
                .items_center()
                .rounded_full()
                .cursor_pointer()
                .text_size(px(10.0))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(theme_accent_brand(cx))
                .tooltip(tooltip(crate::i18n::tr("Open Screen Wall to manage all views")).build())
                .on_click(|_, _, cx| open_or_focus_screen_wall(cx))
                .child(format!("+{extra}")),
        );
    }

    Some(strip.into_any_element())
}

#[derive(Clone)]
struct WallCardSnapshot {
    entry: StreamWallEntry,
    frame: Option<Arc<MacDecodedVideoFrame>>,
    owner: Arc<OriginalOwner>,
    connection_id: Option<u32>,
    supports_input: bool,
    input_locked: bool,
    scale_mode: ViewportScaleMode,
}

pub(super) struct ScreenWallView {
    window_handle: AnyWindowHandle,
    input_focus: FocusHandle,
    root_focus: FocusHandle,
    layout_mode: WallLayoutMode,
    focused_card: Option<EntityId>,
    active_owner: Option<(EntityId, Arc<OriginalOwner>, PointerInputTracker)>,
    card_bounds: Arc<Mutex<BTreeMap<EntityId, Bounds<Pixels>>>>,
    last_notice: Option<(String, bool, Instant)>,
    window_active: bool,
    window_visible: bool,
}

impl ScreenWallView {
    fn release_active_card_input(&mut self) {
        set_wall_focused_window(None);
        if let Some((_, owner, mut tracker)) = self.active_owner.take() {
            if let Some((target, cancel_ev)) = tracker.cancel_scroll() {
                owner.queue_viewport_input(&target, cancel_ev);
            }
            for ev in tracker.release_events() {
                owner.queue_viewing_input(ev);
            }
            owner.release_input();
            owner.set_view_demand(WALL_VISIBLE.load(Ordering::Acquire), false);
        }
    }

    fn activate_card_focus(
        &mut self,
        card_entry: &StreamWallEntry,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Arc<OriginalOwner>> {
        let view = card_entry.view.upgrade()?;
        let (owner, supports_input, locked) = {
            let v = view.read(cx);
            (
                v.owner.clone(),
                v.capture_supports_input,
                v.input_locked,
            )
        };
        let switching = self
            .active_owner
            .as_ref()
            .is_none_or(|(id, _, _)| *id != card_entry.entity_id);
        if switching {
            self.release_active_card_input();
            self.focused_card = Some(card_entry.entity_id);
            set_wall_focused_window(Some(card_entry.window_handle));
            owner.set_view_demand(true, true);
            if supports_input && !locked {
                let _ = owner.retry_viewing_input();
                let _ = owner.focus_viewing_source();
            }
            self.active_owner = Some((
                card_entry.entity_id,
                owner.clone(),
                PointerInputTracker::default(),
            ));
        } else {
            set_wall_focused_window(Some(card_entry.window_handle));
            owner.set_view_demand(true, true);
        }
        window.focus(&self.input_focus);
        Some(owner)
    }

    fn card_pointer_event(
        &self,
        entity_id: EntityId,
        frame: Option<&MacDecodedVideoFrame>,
        scale_mode: ViewportScaleMode,
        position: Point<Pixels>,
    ) -> Option<protocol::InputEvent> {
        let frame = frame?;
        let bounds = *self
            .card_bounds
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&entity_id)?;
        frame_source_pointer(
            frame,
            absolute_pointer_event(
                (f32::from(position.x), f32::from(position.y)),
                (
                    f32::from(bounds.origin.x),
                    f32::from(bounds.origin.y),
                    f32::from(bounds.size.width),
                    f32::from(bounds.size.height),
                ),
                (frame.width(), frame.height()),
                scale_mode,
            ),
        )
    }

    fn trigger_arrange_all(
        &mut self,
        layout: protocol::session::WindowArrangeLayout,
        cx: &mut Context<Self>,
    ) {
        let batches = cx
            .try_global::<ScreenWallRegistry>()
            .map(|r| r.same_device_window_batches())
            .unwrap_or_default();
        if batches.is_empty() {
            self.last_notice = Some((
                crate::i18n::tr("Open at least 2 app windows on the same device to tile").into(),
                true,
                Instant::now(),
            ));
            cx.notify();
            return;
        }
        let entries = registry_entries(cx);
        let mut requested = 0usize;
        let mut last_err: Option<String> = None;
        for (_, primary_entity, video_ids) in batches {
            if let Some(entry) = entries.iter().find(|e| e.entity_id == primary_entity)
                && let Some(view) = entry.view.upgrade()
            {
                let owner = view.read(cx).owner.clone();
                match owner.arrange_windows(video_ids, layout) {
                    Ok(_) => requested += 1,
                    Err(err) => last_err = Some(err),
                }
            }
        }
        if requested > 0 {
            let msg = match layout {
                protocol::session::WindowArrangeLayout::Restore => {
                    crate::i18n::tr("Restoring host windows…").into()
                }
                _ => crate::i18n::tr("Arranging host windows…").into(),
            };
            self.last_notice = Some((msg, false, Instant::now()));
        } else if let Some(err) = last_err {
            self.last_notice = Some((err, true, Instant::now()));
        }
        cx.notify();
    }

    fn render_card(&self, card: WallCardSnapshot, spotlight_primary: bool, cx: &mut Context<Self>) -> Div {
        let theme = cx.theme().clone();
        let entry = card.entry.clone();
        let entity_id = entry.entity_id;
        let focused = self.focused_card == Some(entity_id);
        let accent = device_accent_color(&entry.device_id, cx);
        let owner_for_ack = card.owner.clone();
        let frame_for_ack = card.frame.clone();
        let conn_id = card.connection_id;
        let frame_for_move = card.frame.clone();
        let frame_for_down = card.frame.clone();
        let frame_for_up = card.frame.clone();
        let frame_for_scroll = card.frame.clone();
        let scale_mode = card.scale_mode;
        let card_bounds = self.card_bounds.clone();

        let title_label = if entry.device_name.is_empty() {
            entry.title.clone()
        } else {
            format!("{} · {}", entry.device_name, entry.title)
        };
        let entry_move_left = entry.clone();
        let entry_move_right = entry.clone();
        let entry_spotlight = entry.clone();
        let entry_popout = entry.clone();
        let entry_close = entry.clone();
        let entry_down = entry.clone();
        let entry_scroll = entry.clone();

        let video_element = stream_video_canvas_surface(
            card.frame.as_deref(),
            "Connecting to remote host...",
            "Waiting for the first decoded video frame",
            scale_mode,
            &theme,
        );

        let bounds_probe = canvas(
            move |bounds, _, _| {
                card_bounds
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .insert(entity_id, bounds);
            },
            move |_, (), _, _| {
                if let (Some(id), Some(frame)) = (conn_id, frame_for_ack.as_ref()) {
                    owner_for_ack.acknowledge_paint(id, frame);
                }
            },
        )
        .absolute()
        .top_0()
        .left_0()
        .size_full();

        let header = div()
            .h(px(26.0))
            .w_full()
            .flex_none()
            .flex()
            .items_center()
            .justify_between()
            .gap_1()
            .px_2()
            .bg(if focused {
                theme.surface.raised
            } else {
                theme.surface.base
            })
            .border_b_1()
            .border_color(theme.border.divider)
            .on_any_mouse_down(cx.listener(|_, _, _, cx| cx.stop_propagation()))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_1p5()
                    .min_w_0()
                    .flex_1()
                    .child(div().size(px(7.0)).flex_none().rounded_full().bg(accent))
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .text_size(px(11.0))
                            .font_weight(if focused {
                                FontWeight::SEMIBOLD
                            } else {
                                FontWeight::MEDIUM
                            })
                            .text_color(if focused {
                                theme.content.primary
                            } else {
                                theme.content.secondary
                            })
                            .child(title_label),
                    )
                    .when(focused && card.supports_input && !card.input_locked, |h| {
                        h.child(
                            div()
                                .flex_none()
                                .px_1p5()
                                .py_0p5()
                                .rounded_full()
                                .bg(theme_brand_soft(cx))
                                .text_size(px(9.0))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(theme_accent_brand(cx))
                                .child(crate::i18n::tr("Focused")),
                        )
                    }),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .flex_none()
                    .child(
                        command_button(
                            format!("wall-card-left-{}", entity_id.as_u64()),
                            ActionVariantKind::Neutral,
                            cx,
                        )
                        .size(px(20.0))
                        .p_0()
                        .rounded(px(5.0))
                        .tooltip(tooltip(crate::i18n::tr("Move card earlier")).build())
                        .on_click(move |_, _, cx| {
                            cx.update_global::<ScreenWallRegistry, _>(|reg, _| {
                                reg.move_entry(entry_move_left.entity_id, -1);
                            });
                            cx.refresh_windows();
                        })
                        .child(div().text_size(px(10.0)).child("◀")),
                    )
                    .child(
                        command_button(
                            format!("wall-card-right-{}", entity_id.as_u64()),
                            ActionVariantKind::Neutral,
                            cx,
                        )
                        .size(px(20.0))
                        .p_0()
                        .rounded(px(5.0))
                        .tooltip(tooltip(crate::i18n::tr("Move card later")).build())
                        .on_click(move |_, _, cx| {
                            cx.update_global::<ScreenWallRegistry, _>(|reg, _| {
                                reg.move_entry(entry_move_right.entity_id, 1);
                            });
                            cx.refresh_windows();
                        })
                        .child(div().text_size(px(10.0)).child("▶")),
                    )
                    .when(self.layout_mode == WallLayoutMode::Spotlight && !spotlight_primary, |bar| {
                        bar.child(
                            command_button(
                                format!("wall-card-spotlight-{}", entity_id.as_u64()),
                                ActionVariantKind::Neutral,
                                cx,
                            )
                            .h(px(20.0))
                            .px_1p5()
                            .rounded(px(5.0))
                            .text_size(px(9.5))
                            .tooltip(tooltip(crate::i18n::tr("Promote to primary stage")).build())
                            .on_click(move |_, _, cx| {
                                cx.update_global::<ScreenWallRegistry, _>(|reg, _| {
                                    reg.promote_spotlight(entry_spotlight.entity_id);
                                });
                                cx.refresh_windows();
                            })
                            .child(crate::i18n::tr("Pin")),
                        )
                    })
                    .child(
                        command_button(
                            format!("wall-card-popout-{}", entity_id.as_u64()),
                            ActionVariantKind::Neutral,
                            cx,
                        )
                        .size(px(20.0))
                        .p_0()
                        .rounded(px(5.0))
                        .tooltip(tooltip(crate::i18n::tr("Focus standalone window")).build())
                        .on_click(move |_, _, cx| {
                            activate_stream_tab(&entry_popout, cx);
                        })
                        .child(icon(IconName::Window).size(px(11.0))),
                    )
                    .child(
                        command_button(
                            format!("wall-card-close-{}", entity_id.as_u64()),
                            ActionVariantKind::Neutral,
                            cx,
                        )
                        .size(px(20.0))
                        .p_0()
                        .rounded(px(5.0))
                        .tooltip(tooltip(crate::i18n::tr("Close stream")).build())
                        .on_click(move |_, _, cx| {
                            if let Some(view) = entry_close.view.upgrade() {
                                let _ = view.update(cx, |v, cx| v.end_stream_window(cx));
                            }
                        })
                        .child(icon(IconName::Close).size(px(10.5))),
                    ),
            );

        let interaction_area = div()
            .id(SharedString::from(format!("wall-card-video-{}", entity_id.as_u64())))
            .relative()
            .flex_1()
            .w_full()
            .min_h_0()
            .bg(rgb(0x050706))
            .on_mouse_move(cx.listener(move |this, event: &MouseMoveEvent, _, _cx| {
                if this.focused_card != Some(entity_id) {
                    return;
                }
                let Some(ev) = this.card_pointer_event(
                    entity_id,
                    frame_for_move.as_deref(),
                    scale_mode,
                    event.position,
                ) else {
                    return;
                };
                if let Some((active_id, owner, _)) = &this.active_owner
                    && *active_id == entity_id
                {
                    owner.queue_viewing_input(ev);
                }
            }))
            .on_any_mouse_down(cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                let Some(owner) = this.activate_card_focus(&entry_down, window, cx) else {
                    return;
                };
                if let Some(pointer_ev) = this.card_pointer_event(
                    entity_id,
                    frame_for_down.as_deref(),
                    scale_mode,
                    event.position,
                ) {
                    owner.queue_viewing_input(pointer_ev);
                }
                if let Some(btn) = protocol_mouse_button(event.button)
                    && owner.queue_viewing_input(protocol::InputEvent::MouseDown(btn))
                    && let Some((_, _, tracker)) = &mut this.active_owner
                {
                    tracker.button_changed(btn, true);
                }
                cx.stop_propagation();
                cx.notify();
            }))
            .capture_any_mouse_up(cx.listener(move |this, event: &MouseUpEvent, _, _cx| {
                let Some(btn) = protocol_mouse_button(event.button) else {
                    return;
                };
                if this
                    .active_owner
                    .as_ref()
                    .is_none_or(|(id, _, tracker)| *id != entity_id || !tracker.pressed_buttons.contains(&btn))
                {
                    return;
                }
                let pointer_ev = this.card_pointer_event(
                    entity_id,
                    frame_for_up.as_deref(),
                    scale_mode,
                    event.position,
                );
                let Some((_, owner, tracker)) = &mut this.active_owner else {
                    return;
                };
                if let Some(pointer_ev) = pointer_ev {
                    owner.queue_viewing_input(pointer_ev);
                }
                if owner.queue_viewing_input(protocol::InputEvent::MouseUp(btn)) {
                    tracker.button_changed(btn, false);
                }
            }))
            .on_scroll_wheel(cx.listener(move |this, event: &ScrollWheelEvent, window, cx| {
                let gesture = protocol_scroll_gesture(event);
                let delta = event.delta.pixel_delta(px(40.0));
                let switching = this.focused_card != Some(entity_id);
                let (dx, dy) = (f32::from(delta.x), f32::from(delta.y));
                let active_scroll = gesture.can_take_control(dx.trunc() as i32, dy.trunc() as i32);
                if switching && !active_scroll {
                    return;
                }
                let Some(owner) = (if switching {
                    this.activate_card_focus(&entry_scroll, window, cx)
                } else {
                    this.active_owner
                        .as_ref()
                        .filter(|(id, _, _)| *id == entity_id)
                        .map(|(_, o, _)| o.clone())
                }) else {
                    return;
                };
                let pointer_ev = this.card_pointer_event(
                    entity_id,
                    frame_for_scroll.as_deref(),
                    scale_mode,
                    event.position,
                );
                let Some(target) = owner.viewport_target() else {
                    return;
                };
                let Some((_, _, tracker)) = &mut this.active_owner else {
                    return;
                };
                if !tracker.accepts_scroll(gesture, &target) {
                    if let Some((t, cancel)) = tracker.cancel_scroll() {
                        owner.queue_viewport_input(&t, cancel);
                    }
                    tracker.reset_pointer();
                    return;
                }
                if gesture.phase == protocol::ScrollPhase::Began
                    || (gesture == protocol::ScrollGesture::default()
                        && tracker.scroll_binding.is_some())
                {
                    if let Some((t, cancel)) = tracker.cancel_scroll() {
                        owner.queue_viewport_input(&t, cancel);
                    }
                    tracker.reset_pointer();
                }
                if let Some(scroll_ev) = tracker.scrolled((dx, dy), gesture) {
                    if let protocol::InputEvent::MouseScroll {
                        delta_x,
                        delta_y,
                        gesture,
                    } = &scroll_ev
                        && gesture.can_take_control(*delta_x, *delta_y)
                    {
                        if let Some(p_ev) = pointer_ev {
                            let _ = owner.queue_viewport_input(&target, p_ev);
                        }
                    }
                    if owner.queue_viewport_input(&target, scroll_ev) {
                        tracker.scroll_queued(gesture, target);
                    }
                }
                cx.stop_propagation();
            }))
            .child(video_element)
            .child(bounds_probe);

        div()
            .flex()
            .flex_col()
            .w_full()
            .h_full()
            .min_w_0()
            .min_h_0()
            .rounded(px(10.0))
            .overflow_hidden()
            .bg(theme.surface.base)
            .when(focused, |d| d.border_2())
            .when(!focused, |d| d.border_1())
            .border_color(if focused {
                theme_accent_brand(cx)
            } else {
                theme.border.divider
            })
            .child(header)
            .child(interaction_area)
    }
}

impl Render for ScreenWallView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let entries = registry_entries(cx);
        let mut cards = Vec::with_capacity(entries.len());
        let mut latest_arrange: Option<(Result<u32, String>, Instant)> = None;
        for entry in entries {
            let Some(view) = entry.view.upgrade() else {
                continue;
            };
            let v = view.read(cx);
            let owner = v.owner.clone();
            let conn_id = v.bound_source.map(|(id, _)| id);
            if let Some((outcome, elapsed)) = owner.window_arrange_notice() {
                let ts = Instant::now() - elapsed;
                if latest_arrange.as_ref().is_none_or(|(_, prev)| ts > *prev) {
                    latest_arrange = Some((outcome, ts));
                }
            }
            cards.push(WallCardSnapshot {
                entry,
                frame: v.current_frame.clone(),
                owner,
                connection_id: conn_id,
                supports_input: v.capture_supports_input,
                input_locked: v.input_locked,
                scale_mode: v.scale_mode,
            });
        }
        if let Some((outcome, ts)) = latest_arrange
            && self
                .last_notice
                .as_ref()
                .is_none_or(|(_, _, prev)| ts >= *prev)
        {
            self.last_notice = Some(match outcome {
                Ok(n) => (
                    crate::i18n::tr_format!("Arranged {} host windows", n),
                    false,
                    Instant::now(),
                ),
                Err(err) => (err, true, Instant::now()),
            });
        }
        if let Some(focused) = self.focused_card
            && !cards.iter().any(|c| c.entry.entity_id == focused)
        {
            self.focused_card = cards.first().map(|c| c.entry.entity_id);
            if self.active_owner.as_ref().is_some_and(|(id, _, _)| *id == focused) {
                self.release_active_card_input();
            }
        } else if self.focused_card.is_none() {
            self.focused_card = cards.first().map(|c| c.entry.entity_id);
        }

        let batches = cx
            .try_global::<ScreenWallRegistry>()
            .map(|r| r.same_device_window_batches())
            .unwrap_or_default();
        let unique_devices: BTreeSet<String> =
            cards.iter().map(|c| c.entry.device_id.clone()).collect();
        let can_arrange = !batches.is_empty();
        let spotlight_id = cx
            .try_global::<ScreenWallRegistry>()
            .and_then(|r| r.resolved_spotlight())
            .filter(|id| cards.iter().any(|c| c.entry.entity_id == *id))
            .or_else(|| self.focused_card)
            .or_else(|| cards.first().map(|c| c.entry.entity_id));

        let notice_banner = self
            .last_notice
            .as_ref()
            .filter(|(_, _, at)| at.elapsed() < Duration::from_secs(4))
            .map(|(text, err, _)| (text.clone(), *err));

        let top_bar = div()
            .h(px(36.0))
            .w_full()
            .flex_none()
            .flex()
            .items_center()
            .justify_between()
            .gap_2()
            .px_2p5()
            .bg(theme.surface.base)
            .border_b_1()
            .border_color(theme.border.divider)
            .on_any_mouse_down(|_, _, cx| cx.stop_propagation())
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .when(cfg!(target_os = "macos"), |row| {
                        row.child(div().w(px(64.0)).h_full().on_mouse_down(
                            MouseButton::Left,
                            |_, window, _| {
                                if !cfg!(test) {
                                    window.start_window_move();
                                }
                            },
                        ))
                    })
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_1p5()
                            .text_size(px(12.5))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(theme.content.primary)
                            .child(icon(IconName::Display).size(px(14.0)).color(theme_accent_brand(cx)))
                            .child(crate::i18n::tr("Screen Wall")),
                    )
                    .child(
                        div()
                            .px_2()
                            .py_0p5()
                            .rounded_full()
                            .bg(theme.surface.sunken)
                            .text_size(px(10.5))
                            .text_color(theme.content.secondary)
                            .child(crate::i18n::tr_format!(
                                "{} views · {} devices",
                                cards.len(),
                                unique_devices.len()
                            )),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .h_full()
                    .on_mouse_down(MouseButton::Left, |event, window, _| {
                        if event.click_count >= 2 {
                            window.titlebar_double_click();
                        } else if !cfg!(test) {
                            window.start_window_move();
                        }
                    }),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_1p5()
                    .child(
                        command_button(
                            "wall-mode-grid",
                            if self.layout_mode == WallLayoutMode::AutoGrid {
                                ActionVariantKind::Primary
                            } else {
                                ActionVariantKind::Neutral
                            },
                            cx,
                        )
                        .h(px(24.0))
                        .px_2()
                        .rounded(px(7.0))
                        .text_size(px(11.0))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.layout_mode = WallLayoutMode::AutoGrid;
                            cx.notify();
                        }))
                        .child(crate::i18n::tr("Grid")),
                    )
                    .child(
                        command_button(
                            "wall-mode-spotlight",
                            if self.layout_mode == WallLayoutMode::Spotlight {
                                ActionVariantKind::Primary
                            } else {
                                ActionVariantKind::Neutral
                            },
                            cx,
                        )
                        .h(px(24.0))
                        .px_2()
                        .rounded(px(7.0))
                        .text_size(px(11.0))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.layout_mode = WallLayoutMode::Spotlight;
                            cx.notify();
                        }))
                        .child(crate::i18n::tr("Focus 1+N")),
                    )
                    .when(can_arrange, |bar| {
                        bar.child(div().w(px(1.0)).h(px(14.0)).bg(theme.border.divider))
                            .child(
                                command_button("wall-arrange-tile", ActionVariantKind::Neutral, cx)
                                    .h(px(24.0))
                                    .px_2()
                                    .rounded(px(7.0))
                                    .text_size(px(11.0))
                                    .tooltip(tooltip(crate::i18n::tr("Tile open windows side-by-side on each host")).build())
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.trigger_arrange_all(
                                            protocol::session::WindowArrangeLayout::TileHorizontal,
                                            cx,
                                        );
                                    }))
                                    .child(crate::i18n::tr("Tile host windows")),
                            )
                            .child(
                                command_button("wall-arrange-grid", ActionVariantKind::Neutral, cx)
                                    .h(px(24.0))
                                    .px_2()
                                    .rounded(px(7.0))
                                    .text_size(px(11.0))
                                    .tooltip(tooltip(crate::i18n::tr("Arrange host windows into a neat grid")).build())
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.trigger_arrange_all(
                                            protocol::session::WindowArrangeLayout::TileGrid,
                                            cx,
                                        );
                                    }))
                                    .child(crate::i18n::tr("Grid tile")),
                            )
                            .child(
                                command_button(
                                    "wall-arrange-restore",
                                    ActionVariantKind::Neutral,
                                    cx,
                                )
                                .h(px(24.0))
                                .px_2()
                                .rounded(px(7.0))
                                .text_size(px(11.0))
                                .tooltip(tooltip(crate::i18n::tr("Restore previous host window bounds")).build())
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.trigger_arrange_all(
                                        protocol::session::WindowArrangeLayout::Restore,
                                        cx,
                                    );
                                }))
                                .child(crate::i18n::tr("Restore")),
                            )
                    })
                    .child(div().w(px(1.0)).h(px(14.0)).bg(theme.border.divider))
                    .child(
                        command_button("wall-add-window", ActionVariantKind::Primary, cx)
                            .h(px(24.0))
                            .px_2p5()
                            .rounded(px(7.0))
                            .text_size(px(11.0))
                            .on_click(|_, _, cx| show_management_window(cx))
                            .child(crate::i18n::tr("+ Add window")),
                    ),
            );

        let body: AnyElement = if cards.is_empty() {
            div()
                .flex_1()
                .w_full()
                .flex()
                .items_center()
                .justify_center()
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .items_center()
                        .gap_3()
                        .p_6()
                        .rounded(px(16.0))
                        .bg(theme_glass_card(cx))
                        .border_1()
                        .border_color(theme_border_fine(cx))
                        .child(icon(IconName::Display).size(px(28.0)).color(theme_accent_brand(cx)))
                        .child(
                            div()
                                .text_size(px(14.0))
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(crate::i18n::tr("No active screen or window streams")),
                        )
                        .child(
                            div()
                                .text_size(px(11.5))
                                .text_color(theme.content.secondary)
                                .child(crate::i18n::tr(
                                    "Open any device screen or application window to view and control them together here.",
                                )),
                        )
                        .child(
                            command_button("wall-empty-add", ActionVariantKind::Primary, cx)
                                .h(px(30.0))
                                .px_3()
                                .rounded_full()
                                .on_click(|_, _, cx| show_management_window(cx))
                                .child(crate::i18n::tr("Open Devices")),
                        ),
                )
                .into_any_element()
        } else if self.layout_mode == WallLayoutMode::Spotlight && cards.len() > 1 {
            let primary_idx = spotlight_id
                .and_then(|sid| cards.iter().position(|c| c.entry.entity_id == sid))
                .unwrap_or(0);
            let mut rest = cards;
            let primary = rest.remove(primary_idx);
            let mut side_col = div()
                .w(px(340.0))
                .max_w(relative(0.32))
                .h_full()
                .flex()
                .flex_col()
                .gap_2()
                .min_h_0();
            for side in rest {
                side_col = side_col.child(
                    div()
                        .flex_1()
                        .w_full()
                        .min_h_0()
                        .child(self.render_card(side, false, cx)),
                );
            }
            div()
                .flex_1()
                .w_full()
                .min_h_0()
                .p_2()
                .flex()
                .gap_2()
                .child(
                    div()
                        .flex_1()
                        .h_full()
                        .min_w_0()
                        .min_h_0()
                        .child(self.render_card(primary, true, cx)),
                )
                .child(side_col)
                .into_any_element()
        } else {
            let (cols, _rows) = wall_grid_dimensions(cards.len());
            let mut grid = div()
                .flex_1()
                .w_full()
                .min_h_0()
                .p_2()
                .flex()
                .flex_col()
                .gap_2();
            let mut iter = cards.into_iter().peekable();
            while iter.peek().is_some() {
                let mut row_div = div().flex_1().w_full().min_h_0().flex().gap_2();
                for _ in 0..cols {
                    if let Some(card) = iter.next() {
                        row_div = row_div.child(
                            div()
                                .flex_1()
                                .h_full()
                                .min_w_0()
                                .min_h_0()
                                .child(self.render_card(card, false, cx)),
                        );
                    }
                }
                grid = grid.child(row_div);
            }
            grid.into_any_element()
        };

        let container = div()
            .id("screen-wall-root")
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .bg(theme.surface.canvas)
            .text_color(theme.content.primary)
            .font_family(crate::design_system::product_ui_font())
            .track_focus(&self.input_focus)
            .capture_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                let Some(keystroke) = fresh_key_down(event) else {
                    return;
                };
                let cmd = keystroke.modifiers.control || keystroke.modifiers.platform;
                if cmd {
                    if keystroke.key.eq_ignore_ascii_case("g") {
                        this.layout_mode = match this.layout_mode {
                            WallLayoutMode::AutoGrid => WallLayoutMode::Spotlight,
                            WallLayoutMode::Spotlight => WallLayoutMode::AutoGrid,
                        };
                        cx.stop_propagation();
                        cx.notify();
                        return;
                    }
                    if keystroke.key == "[" || keystroke.key == "]" {
                        let entries = registry_entries(cx);
                        if !entries.is_empty() {
                            let cur = this
                                .focused_card
                                .and_then(|id| entries.iter().position(|e| e.entity_id == id))
                                .unwrap_or(0);
                            if let Some(next) = cycle_index(
                                entries.len(),
                                cur,
                                if keystroke.key == "]" { 1 } else { -1 },
                            ) {
                                let target = entries[next].clone();
                                this.activate_card_focus(&target, window, cx);
                                cx.stop_propagation();
                                cx.notify();
                                return;
                            }
                        }
                    }
                    if let Ok(num) = keystroke.key.parse::<usize>()
                        && (1..=9).contains(&num)
                    {
                        let entries = registry_entries(cx);
                        if let Some(target) = entries.get(num - 1).cloned() {
                            if keystroke.modifiers.shift {
                                cx.update_global::<ScreenWallRegistry, _>(|reg, _| {
                                    reg.promote_spotlight(target.entity_id);
                                });
                            }
                            this.activate_card_focus(&target, window, cx);
                            cx.stop_propagation();
                            cx.notify();
                            return;
                        }
                    }
                }
                if this.input_focus.is_focused(window)
                    && let Some((_, owner, tracker)) = &mut this.active_owner
                    && let Some(ev) = protocol_key_event(keystroke, true)
                {
                    if let protocol::InputEvent::Key { key_code, .. } = ev
                        && owner.queue_viewing_input(ev)
                    {
                        tracker.key_changed(key_code, true);
                    }
                    cx.stop_propagation();
                }
            }))
            .capture_key_up(cx.listener(|this, event: &KeyUpEvent, window, cx| {
                if this.input_focus.is_focused(window)
                    && let Some((_, owner, tracker)) = &mut this.active_owner
                    && let Some(ev) = protocol_key_event(&event.keystroke, false)
                    && let protocol::InputEvent::Key { key_code, .. } = ev
                    && tracker.pressed_keys.contains(&key_code)
                {
                    if owner.queue_viewing_input(ev) {
                        tracker.key_changed(key_code, false);
                    }
                    cx.stop_propagation();
                }
            }))
            .on_modifiers_changed(cx.listener(|this, event: &ModifiersChangedEvent, window, _cx| {
                if this.input_focus.is_focused(window)
                    && let Some((_, owner, tracker)) = &mut this.active_owner
                {
                    let modifiers = protocol_modifiers(event.modifiers);
                    if owner.queue_viewing_input(protocol::InputEvent::ModifiersChanged(modifiers)) {
                        tracker.modifiers = modifiers;
                    }
                }
            }))
            .child(top_bar)
            .child(body)
            .when_some(notice_banner, |tree, (text, is_err)| {
                tree.child(
                    div()
                        .absolute()
                        .bottom(px(14.0))
                        .right(px(16.0))
                        .px_3()
                        .py_1p5()
                        .rounded_full()
                        .bg(theme_glass_floating(cx))
                        .border_1()
                        .border_color(if is_err {
                            theme.status.error.bg
                        } else {
                            theme_accent_brand(cx)
                        })
                        .shadow_lg()
                        .text_size(px(11.5))
                        .child(text),
                )
            });

        ely_gpui_component::primitives::FocusScope::new(&self.root_focus)
            .root()
            .size_full()
            .child(container)
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::{
        cycle_index, device_color_index, group_same_device_batches, wall_grid_dimensions,
    };

    #[test]
    fn wall_grid_dimensions_scale_cleanly_up_to_nine_cards() {
        assert_eq!(wall_grid_dimensions(1), (1, 1));
        assert_eq!(wall_grid_dimensions(2), (2, 1));
        assert_eq!(wall_grid_dimensions(3), (2, 2));
        assert_eq!(wall_grid_dimensions(4), (2, 2));
        assert_eq!(wall_grid_dimensions(5), (3, 2));
        assert_eq!(wall_grid_dimensions(6), (3, 2));
        assert_eq!(wall_grid_dimensions(9), (3, 3));
    }

    #[test]
    fn same_device_batches_group_only_devices_with_two_or_more_windows() {
        let items = vec![
            ("dev-a".to_string(), 1u32, true, Some(1)),
            ("dev-b".to_string(), 2u32, false, Some(1)), // whole screen display, not window
            ("dev-b".to_string(), 3u32, true, Some(2)),
            ("dev-a".to_string(), 4u32, true, Some(2)),
        ];
        let batches = group_same_device_batches(&items);
        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0], ("dev-a".to_string(), 1u32, vec![1, 2]));
    }

    #[test]
    fn cycle_index_wraps_both_directions() {
        assert_eq!(cycle_index(0, 0, 1), None);
        assert_eq!(cycle_index(3, 0, -1), Some(2));
        assert_eq!(cycle_index(3, 2, 1), Some(0));
        assert_eq!(cycle_index(3, 1, 1), Some(2));
    }

    #[test]
    fn device_color_index_is_deterministic_and_bounded() {
        let idx = device_color_index("mac-studio-01");
        assert!(idx < 6);
        assert_eq!(idx, device_color_index("mac-studio-01"));
    }
}
