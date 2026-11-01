//! Native camera catalog and ownership, sharing the current host/runtime identity.
#[cfg(target_os="macos")]
mod macos;
#[cfg(target_os="macos")]
pub use macos::{access, request_access};
#[cfg(target_os="macos")]
pub(crate) use macos::{list, CameraHandle};
