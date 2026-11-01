//! Native camera catalog and ownership, sharing the current host/runtime identity.
#[cfg(target_os="macos")]
mod macos;
#[cfg(target_os="macos")]
pub use macos::{access, request_access};
#[cfg(target_os="macos")]
pub(crate) use macos::{list, CameraHandle};
#[cfg(target_os="linux")]
mod linux;
#[cfg(target_os="linux")]
pub use linux::{access, request_access};
#[cfg(target_os="linux")]
pub(crate) use linux::{list, start};
#[cfg(target_os="windows")]
mod windows;
#[cfg(target_os="windows")]
pub(crate) use windows::{list,CameraStream};
