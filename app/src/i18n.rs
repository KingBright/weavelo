//! Product language is local UI state, independent of session/network identity.
use std::{collections::BTreeMap, sync::{LazyLock, atomic::{AtomicU8, Ordering}}};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Language {
    #[serde(rename = "en")]
    English,
    #[serde(rename = "zh-CN")]
    Chinese,
}
impl Language {
    pub fn from_locale(locale: &str) -> Self {
        if locale.split(['-', '_', '.', ':']).next().is_some_and(|s| s.eq_ignore_ascii_case("zh")) {
            Self::Chinese
        } else { Self::English }
    }
    pub fn native_name(self) -> &'static str {
        match self { Self::English => "English", Self::Chinese => "中文" }
    }
}

static LANGUAGE: AtomicU8 = AtomicU8::new(0);
static CATALOG: LazyLock<BTreeMap<String, String>> = LazyLock::new(||
    serde_json::from_str(include_str!("i18n/zh-CN.json")).expect("validated Chinese UI catalog"));

static SYSTEM_LANGUAGE: LazyLock<Language> = LazyLock::new(detect_system_language);
pub fn system_language() -> Language { *SYSTEM_LANGUAGE }

fn detect_system_language() -> Language {
    #[cfg(test)]
    return Language::English;
    #[cfg(not(test))]
    {
        #[cfg(target_os = "macos")]
        {
            // Read the OS language preference, not the Terminal process locale.
            if let Ok(output) = std::process::Command::new("/usr/bin/defaults")
                .args(["read", "-g", "AppleLanguages"]).output() {
                if output.status.success() {
                    let text = String::from_utf8_lossy(&output.stdout);
                    if let Some(locale) = text.split(['(', ')', ',', '"', '\n', ' ']).find(|s| !s.is_empty()) {
                        return Language::from_locale(locale);
                    }
                }
            }
        }
        #[cfg(target_os = "windows")]
        {
            #[link(name = "kernel32")]
            unsafe extern "system" { fn GetUserDefaultUILanguage() -> u16; }
            return if unsafe { GetUserDefaultUILanguage() } & 0x3ff == 0x04 { Language::Chinese } else { Language::English };
        }
        #[cfg(not(target_os = "windows"))]
        for key in ["LC_ALL", "LC_MESSAGES", "LANGUAGE", "LANG"] {
            if let Ok(value) = std::env::var(key) { if !value.is_empty() { return Language::from_locale(&value); } }
        }
        #[allow(unreachable_code)]
        Language::English
    }
}

pub fn language() -> Language {
    match LANGUAGE.load(Ordering::Acquire) {
        1 => Language::English,
        2 => Language::Chinese,
        _ => {
            let selected = crate::preferences::UserPreferences::load_or_default().ui.language.unwrap_or_else(system_language);
            set_current(selected);
            selected
        }
    }
}
fn set_current(language: Language) { LANGUAGE.store(match language { Language::English => 1, Language::Chinese => 2 }, Ordering::Release); }
pub fn select(language: Language) -> std::io::Result<()> {
    crate::preferences::UserPreferences::update(|prefs| prefs.ui.language = Some(language))?;
    set_current(language);
    Ok(())
}
/// Background tray process refreshes this only when constructing its menu.
pub fn reload() {
    set_current(crate::preferences::UserPreferences::load_or_default().ui.language.unwrap_or_else(system_language));
}
pub fn is_chinese() -> bool { language() == Language::Chinese }
pub fn translate(language: Language, source: &str) -> &str {
    if language == Language::Chinese { CATALOG.get(source).map(String::as_str).unwrap_or(source) } else { source }
}
pub fn tr(source: &str) -> &str { translate(language(), source) }

/// Render only application-owned messages. Device/app/file names must never be
/// passed here. Unknown OS/network diagnostic details remain byte-for-byte text.
pub fn message(source: &str) -> String {
    if !is_chinese() { return source.to_owned(); }
    if let Some(text) = CATALOG.get(source) { return text.clone(); }
    for (english, chinese) in CATALOG.iter() {
        if let Some(translated) = translate_message_template(english, chinese, source) { return translated; }
    }
    source.to_owned()
}

fn format_parts(template: &str) -> Option<(Vec<String>, Vec<String>)> {
    let mut literals = vec![String::new()]; let mut slots = Vec::new();
    let mut chars = template.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '{' {
            if chars.peek() == Some(&'{') { chars.next(); literals.last_mut()?.push('{'); continue; }
            let mut slot = String::new();
            loop { match chars.next()? { '}' => break, '{' => return None, c => slot.push(c) } }
            slots.push(slot); literals.push(String::new());
        } else if c == '}' {
            if chars.next() != Some('}') { return None; }
            literals.last_mut()?.push('}');
        } else { literals.last_mut()?.push(c); }
    }
    Some((literals, slots))
}
fn translate_message_template(english: &str, chinese: &str, source: &str) -> Option<String> {
    let (literals, slots) = format_parts(english)?;
    if slots.is_empty() || !literals[0].chars().any(|c| c.is_alphabetic()) { return None; }
    let mut rest = source.strip_prefix(&literals[0])?;
    let mut values = Vec::new();
    for (index, delimiter) in literals.iter().skip(1).enumerate() {
        if index + 1 == slots.len() {
            values.push(rest.strip_suffix(delimiter)?.to_owned()); rest = "";
        } else {
            if delimiter.is_empty() { return None; }
            let end = rest.find(delimiter)?;
            // Ambiguous user-provided values are diagnostics, not a reason to
            // guess a boundary and corrupt a name/path while translating.
            if rest[end + delimiter.len()..].contains(delimiter) { return None; }
            values.push(rest[..end].to_owned()); rest = &rest[end + delimiter.len()..];
        }
    }
    if !rest.is_empty() { return None; }
    let (translated_literals, translated_slots) = format_parts(chinese)?;
    let mut used = vec![false; slots.len()];
    let mut result = translated_literals[0].clone();
    for (index, slot) in translated_slots.iter().enumerate() {
        let original = slots.iter().enumerate().position(|(i, candidate)| !used[i] && candidate == slot)?;
        used[original] = true;
        result.push_str(&values[original]); result.push_str(&translated_literals[index + 1]);
    }
    used.into_iter().all(|v| v).then_some(result)
}

include!(concat!(env!("OUT_DIR"), "/i18n_formats.rs"));

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn locale_selection_accepts_os_locale_forms() {
        for locale in ["zh", "zh_CN.UTF-8", "zh-Hans-CN", "ZH-tw"] { assert_eq!(Language::from_locale(locale), Language::Chinese); }
        for locale in ["en-US", "en_GB", "C", "ja-JP"] { assert_eq!(Language::from_locale(locale), Language::English); }
    }
    #[test]
    fn message_templates_preserve_values_and_reject_ambiguous_boundaries() {
        assert_eq!(translate_message_template("Cannot open {path}: {error}", "无法打开 {path}：{error}", "Cannot open /tmp/中文.txt: Permission denied"), Some("无法打开 /tmp/中文.txt：Permission denied".into()));
        assert_eq!(translate_message_template("Cannot open {path}: {error}", "无法打开 {path}：{error}", "Cannot open a: b: error"), None);
        assert_eq!(translate_message_template("{device} ready", "{device} 已就绪", "Settings ready"), None);
        assert_eq!(translate_message_template("Connected to {name}", "已连接 {name}", "Connected to Settings"), Some("已连接 Settings".into()));
    }
    #[test]
    fn translated_templates_preserve_every_format_argument() {
        for (source, translated) in CATALOG.iter() {
            let (_, mut original) = format_parts(source).expect("valid source format");
            let (_, mut localized) = format_parts(translated).expect("valid translation format");
            original.sort(); localized.sort();
            assert_eq!(original, localized, "format arguments changed for {source}");
        }
    }
    #[test]
    fn catalogs_are_nonempty_and_leave_user_content_untouched() {
        assert!(!CATALOG.is_empty());
        for (source, translated) in CATALOG.iter() { assert!(!source.is_empty() && !translated.trim().is_empty()); }
        assert_eq!(translate(Language::English, "Settings"), "Settings");
        assert_eq!(translate(Language::Chinese, "Settings"), "设置");
        assert_eq!(translate(Language::Chinese, "My computer 你好 123"), "My computer 你好 123");
    }
}
