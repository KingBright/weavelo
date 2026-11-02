//! A disposable native window exercises minimize → activate on this platform.
//! No product profile, networking, capture, input injection or OS consent.
use gpui::{prelude::*, *};
use std::sync::{Arc, atomic::{AtomicBool, Ordering}};
use std::time::Duration;

struct ActivationCheck;

impl Render for ActivationCheck {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size_full().bg(rgb(0x101318)).text_color(rgb(0xeaf0f6))
            .flex().items_center().justify_center()
            .child("Native minimize and restore check")
    }
}

fn main() {
    let passed = Arc::new(AtomicBool::new(false));
    let result = passed.clone();
    Application::new().run(move |cx| {
        let handle = cx.open_window(WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                None, size(px(400.), px(180.)), cx,
            ))),
            titlebar: Some(TitlebarOptions {
                title: Some("Weavelo native activation check".into()),
                ..Default::default()
            }),
            ..Default::default()
        }, |_, cx| cx.new(|_| ActivationCheck)).unwrap();
        cx.activate(true);
        cx.spawn(async move |cx| {
            Timer::after(Duration::from_millis(400)).await;
            let visible_before = handle.update(cx, |_, window, _| window.is_window_visible()).unwrap();
            handle.update(cx, |_, window, _| window.minimize_window()).unwrap();
            Timer::after(Duration::from_millis(800)).await;
            let visible_minimized = handle.update(cx, |_, window, _| window.is_window_visible()).unwrap();
            handle.update(cx, |_, window, _| window.activate_window()).unwrap();
            Timer::after(Duration::from_millis(800)).await;
            let visible_restored = handle.update(cx, |_, window, _| window.is_window_visible()).unwrap();
            let active = handle.update(cx, |_, window, _| window.is_window_active()).unwrap();
            let ok = visible_before == Some(true) && visible_minimized == Some(false)
                && visible_restored == Some(true) && active;
            result.store(ok, Ordering::Release);
            println!("{}", serde_json::json!({
                "native_window_activation": ok, "visible_before": visible_before,
                "visible_minimized": visible_minimized, "visible_restored": visible_restored,
                "active_after_restore": active, "product_runtime_started": false,
            }));
            cx.update(|cx| cx.quit()).unwrap();
        }).detach();
    });
    if !passed.load(Ordering::Acquire) { std::process::exit(1); }
}
