//! Read-only permission checks run in the process which actually hosts capture.
//! Opening settings never substitutes for a system grant or a captured frame.
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) enum Access {
    Allowed,
    NeedsReview,
    SelectSource,
    SystemManaged,
    OnUse,
}
impl Access {
    pub fn label(self) -> &'static str {
        match self {
            Self::Allowed => "Allowed",
            Self::NeedsReview => "Needs permission",
            Self::SelectSource => "Choose a source",
            Self::SystemManaged => "Managed by the system",
            Self::OnUse => "Checked when used",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct Status {
    pub screen: Access,
    pub control: Access,
    pub microphone: Access,
}

pub(crate) fn inspect() -> Status {
    #[cfg(target_os = "macos")]
    {
        #[link(name = "CoreGraphics", kind = "framework")]
        unsafe extern "C" {
            fn CGPreflightScreenCaptureAccess() -> bool;
            fn CGPreflightPostEventAccess() -> bool;
        }
        #[link(name = "ApplicationServices", kind = "framework")]
        unsafe extern "C" {
            fn AXIsProcessTrusted() -> bool;
        }
        // A missing preflight is an access result, not a diagnosis of why it
        // failed. Existing signature/source/error diagnostics remain intact.
        let checked = |allowed| {
            if allowed {
                Access::Allowed
            } else {
                Access::NeedsReview
            }
        };
        Status {
            screen: checked(unsafe { CGPreflightScreenCaptureAccess() }),
            control: checked(unsafe { AXIsProcessTrusted() && CGPreflightPostEventAccess() }),
            microphone: microphone_access(),
        }
    }
    #[cfg(target_os = "linux")]
    {
        Status {
            screen: Access::SelectSource,
            control: Access::SelectSource,
            microphone: Access::OnUse,
        }
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        Status {
            screen: Access::SystemManaged,
            control: Access::SystemManaged,
            microphone: Access::SystemManaged,
        }
    }
}

#[cfg(target_os = "macos")]
fn microphone_access() -> Access {
    use cocoa::{base::nil, foundation::NSString};
    use objc::{msg_send, sel, sel_impl};
    #[link(name = "AVFoundation", kind = "framework")]
    unsafe extern "C" {}
    let Some(class) = objc::runtime::Class::get("AVCaptureDevice") else {
        return Access::OnUse;
    };
    // authorizationStatus is read-only; only the user can grant microphone use.
    let status: i64 = unsafe {
        let audio = NSString::alloc(nil).init_str("soun");
        let status = msg_send![class, authorizationStatusForMediaType:audio];
        let _: () = msg_send![audio, release];
        status
    };
    match status {
        3 => Access::Allowed,
        0..=2 => Access::NeedsReview,
        _ => Access::OnUse,
    }
}

#[derive(Clone, Copy)]
pub(crate) enum Setting {
    Screen,
    Control,
    Microphone,
}

pub(crate) fn settings_uri(setting: Setting) -> Option<&'static str> {
    #[cfg(target_os = "macos")]
    {
        Some(match setting {
            Setting::Screen => {
                "x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture"
            }
            Setting::Control => {
                "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility"
            }
            Setting::Microphone => {
                "x-apple.systempreferences:com.apple.preference.security?Privacy_Microphone"
            }
        })
    }
    #[cfg(target_os = "windows")]
    {
        match setting {
            Setting::Microphone => Some("ms-settings:privacy-microphone"),
            // The existing Win32 desktop capture/SendInput adapters do not use
            // GraphicsCapture consent or an Accessibility permission switch.
            Setting::Screen | Setting::Control => None,
        }
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        let _ = setting;
        None
    }
}
