use super::*;
use crate::preferences::{DeviceBadge, DeviceBadgePreset as Preset, UserPreferences};
use std::{
    io::{Cursor, Read},
    path::Path,
};

trait PresetUi: Sized {
    const CHOICES: [Self; 5];
    fn label(self) -> &'static str;
    fn image(self, cx: &App) -> Option<Arc<RenderImage>>;
    fn platform_default() -> Self;
}
impl PresetUi for Preset {
    const CHOICES: [Self; 5] = [Self::MacBook, Self::MacStudio, Self::Windows, Self::Linux, Self::Phone];

    fn label(self) -> &'static str {
        match self {
            Self::Computer => "Computer",
            Self::MacBook => "MacBook",
            Self::MacStudio => "Mac Studio",
            Self::Windows => "Windows",
            Self::Linux => "Linux",
            Self::Phone => "Android phone",
        }
    }

    fn image(self, cx: &App) -> Option<Arc<RenderImage>> {
        crate::product_components::device_badge_image(self, cx)
    }

    fn platform_default() -> Self {
        if cfg!(target_os = "windows") {
            Self::Windows
        } else if cfg!(target_os = "linux") {
            Self::Linux
        } else {
            Self::Computer
        }
    }
}

pub(super) use crate::product_components::device_badge_svg as preset_svg;

pub(super) struct Editor {
    selection: DeviceBadge,
    automatic: Preset,
    custom_preview: Option<Arc<RenderImage>>,
    picking: bool,
    error: Option<String>,
}

impl Editor {
    pub(super) fn image(&self, cx: &App) -> Option<Arc<RenderImage>> {
        match &self.selection {
            DeviceBadge::Automatic => self.automatic.image(cx),
            DeviceBadge::Preset(preset) => preset.image(cx),
            DeviceBadge::Custom(_) => self.custom_preview.clone(),
        }
    }

    pub(super) fn new(selection: DeviceBadge) -> Self {
        let custom_preview = match &selection {
            DeviceBadge::Custom(icon) => custom_image(icon).ok(),
            _ => None,
        };
        Self {
            selection,
            automatic: Preset::platform_default(),
            custom_preview,
            picking: false,
            error: None,
        }
    }

    pub(super) fn detect_hardware(cx: &mut Context<RestoredDashboard>) {
        #[cfg(target_os = "macos")]
        if !cfg!(test) {
            let task = tokio::task::spawn_blocking(remote_core::device_artwork::automatic);
            cx.spawn(async move |view, cx| {
                if let Ok(preset) = task.await {
                    let _ = view.update(cx, |this, cx| {
                        this.device_badge_editor.automatic = preset;
                        cx.notify();
                    });
                }
            })
            .detach();
        }
        #[cfg(not(target_os = "macos"))]
        let _ = cx;
    }

    fn save(&mut self, selection: DeviceBadge) -> Result<(), String> {
        let preview = match &selection {
            DeviceBadge::Custom(icon) => Some(custom_image(icon)?),
            _ => None,
        };
        UserPreferences::update(|prefs| prefs.ui.device_badge = selection.clone())
            .map_err(|error| format!("Could not save the device badge. {error}"))?;
        self.selection = selection;
        self.custom_preview = preview;
        Ok(())
    }
}

fn custom_image(icon: &protocol::interaction::ApplicationIcon) -> Result<Arc<RenderImage>, String> {
    let mut cache = remote_core::application_icons::IconCache::default();
    cache.update(std::slice::from_ref(icon), [icon.id])?;
    let assets = cache.assets();
    Ok(render_artwork(&assets[&icon.id]))
}

#[cfg(test)]
mod tests {
    use super::{DeviceBadge, import_image};

    #[test]
    fn imported_badge_preserves_transparent_padding_and_rejects_large_dimensions() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("badge.png");
        let color = image::Rgba([45, 70, 250, 192]);
        image::RgbaImage::from_pixel(128, 48, color)
            .save(&path)
            .unwrap();
        let DeviceBadge::Custom(icon) = import_image(&path).unwrap() else {
            panic!("expected a custom badge")
        };
        let image = image::load_from_memory(&icon.png).unwrap().into_rgba8();
        assert_eq!(image.dimensions(), (64, 64));
        assert_eq!(image.get_pixel(32, 32), &color);
        assert_eq!(image.get_pixel(32, 0)[3], 0);
        assert_eq!(image.get_pixel(32, 63)[3], 0);
        image::RgbaImage::from_pixel(1025, 1, color)
            .save(&path)
            .unwrap();
        assert!(import_image(&path).is_err());
    }
}

fn import_image(path: &Path) -> Result<DeviceBadge, String> {
    const MAX_BYTES: u64 = 2 * 1024 * 1024;
    const MAX_EDGE: u32 = 1024;
    let read_error = "Could not read the selected image.";
    let format_error = "Choose a readable PNG, JPEG or WebP image.";
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NONBLOCK);
    }
    let file = options.open(path).map_err(|_| read_error)?;
    if !file.metadata().map_err(|_| read_error)?.is_file() {
        return Err(read_error.into());
    }
    let mut bytes = Vec::new();
    file.take(MAX_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| read_error)?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err("Choose an image under 2 MB.".into());
    }
    let mut reader = image::ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|_| format_error)?;
    if !matches!(
        reader.format(),
        Some(image::ImageFormat::Png | image::ImageFormat::Jpeg | image::ImageFormat::WebP)
    ) {
        return Err(format_error.into());
    }
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_EDGE);
    limits.max_image_height = Some(MAX_EDGE);
    limits.max_alloc = Some(16 * 1024 * 1024);
    reader.limits(limits);
    let image = reader
        .decode()
        .map_err(|_| "Choose a readable image at most 1024 pixels on each side.")?;
    let fitted = image
        .resize(56, 56, image::imageops::FilterType::Lanczos3)
        .into_rgba8();
    let mut square = image::RgbaImage::new(64, 64);
    image::imageops::replace(
        &mut square,
        &fitted,
        i64::from((64 - fitted.width()) / 2),
        i64::from((64 - fitted.height()) / 2),
    );
    remote_core::application_icons::encode_rgba(64, 64, square.as_raw()).map(DeviceBadge::Custom)
}

fn choose_image(view: &mut RestoredDashboard, cx: &mut Context<RestoredDashboard>) {
    if view.device_badge_editor.picking {
        return;
    }
    view.device_badge_editor.picking = true;
    view.device_badge_editor.error = None;
    let task = tokio::spawn(async {
        let file = rfd::AsyncFileDialog::new()
            .set_title(crate::i18n::tr("Choose a device badge"))
            .add_filter(crate::i18n::tr("Device badge image"), &["png", "jpg", "jpeg", "webp"])
            .pick_file()
            .await?;
        let path = file.path().to_path_buf();
        Some(
            tokio::task::spawn_blocking(move || import_image(&path))
                .await
                .unwrap_or_else(|_| Err("Could not read the selected image.".into())),
        )
    });
    cx.spawn(async move |view, cx| {
        let result = task
            .await
            .unwrap_or(Some(Err("The image chooser did not finish.".into())));
        let _ = view.update(cx, |this, cx| {
            let editor = &mut this.device_badge_editor;
            editor.picking = false;
            editor.error =
                result.and_then(|result| result.and_then(|badge| editor.save(badge)).err());
            cx.notify();
        });
    })
    .detach();
    cx.notify();
}

pub(super) fn controls(editor: &Editor, cx: &mut Context<RestoredDashboard>) -> Div {
    let theme = cx.theme().clone();
    let (label, preview) = match &editor.selection {
        DeviceBadge::Automatic => (
            crate::i18n::tr_format!("Automatic · {}", crate::i18n::tr(editor.automatic.label())),
            editor.automatic.image(cx),
        ),
        DeviceBadge::Preset(preset) => (crate::i18n::tr(preset.label()).into(), preset.image(cx)),
        DeviceBadge::Custom(_) => (crate::i18n::tr("Custom image").into(), editor.custom_preview.clone()),
    };
    let choice =
        |id, label: &'static str, selection: DeviceBadge, cx: &mut Context<RestoredDashboard>| {
            let is_selected = editor.selection == selection;
            ely_gpui_component::buttons::Button::new(id, "")
                .variant(if is_selected {
                    ely_gpui_component::buttons::ButtonVariant::Primary
                } else {
                    ely_gpui_component::buttons::ButtonVariant::Ghost
                })
                .control_size(ely_gpui_component::theme::ControlSize::Sm)
                .h(px(28.0))
                .px_2p5()
                .rounded(px(7.))
                .text_size(px(10.5))
                .disabled(editor.picking)
                .tooltip(tooltip(crate::i18n::tr(label)).build())
                .on_click(cx.listener(move |this, _, _, cx| {
                    let editor = &mut this.device_badge_editor;
                    editor.error = editor.save(selection.clone()).err();
                    cx.notify();
                }))
        };
    let mut choices = crate::product_components::segmented_track(&theme)
        .flex_wrap()
        .child(
            choice(
                "device-badge-auto",
                "Use automatic device badge",
                DeviceBadge::Automatic,
                cx,
            )
            .child(crate::i18n::tr("Auto")),
        );
    for preset in Preset::CHOICES {
        let button = choice(
            preset.label(),
            preset.label(),
            DeviceBadge::Preset(preset),
            cx,
        );
        choices = choices.child(button.when_some(preset.image(cx), |button, image| {
            button.child(gpui::img(image).size(px(18.0)))
        }));
    }
    let amber_ink = crate::design_system::theme_accent_amber(cx);
    div()
        .flex()
        .flex_col()
        .gap_2p5()
        .p_3()
        .rounded(px(12.0))
        .bg(theme.surface.base)
        .border_1()
        .border_color(theme.border.default)
        .child(
            div()
                .flex()
                .items_center()
                .gap_2p5()
                .when_some(preview, |row, image| {
                    row.child(
                        crate::product_components::tonal_icon_well(36.0, true, cx)
                            .child(gpui::img(image).size(px(24.0)).flex_none()),
                    )
                })
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .child(
                            div()
                                .text_size(px(11.5))
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(crate::i18n::tr("Device badge")),
                        )
                        .child(
                            div()
                                .text_size(px(10.5))
                                .text_color(theme.content.secondary)
                                .child(label),
                        ),
                )
                .child(
                    command_button("choose-device-badge", ActionVariantKind::Neutral, cx)
                        .h(px(28.0))
                        .px_2p5()
                        .rounded_full()
                        .text_size(px(10.5))
                        .disabled(editor.picking)
                        .child(crate::i18n::tr("Choose image…"))
                        .on_click(cx.listener(|this, _, _, cx| choose_image(this, cx))),
                ),
        )
        .child(choices)
        .when_some(editor.error.clone(), |panel, error| {
            panel.child(
                div()
                    .text_size(px(10.5))
                    .text_color(amber_ink)
                    .child(crate::i18n::message(&error)),
            )
        })
}
