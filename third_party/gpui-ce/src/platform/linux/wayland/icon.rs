pub(super) mod protocol {
    #![allow(dead_code,non_camel_case_types,non_upper_case_globals,non_snake_case,unused_imports,unused_unsafe)]
    use wayland_client;
    use wayland_client::protocol::*;
    use wayland_protocols::xdg::shell::client::*;
    pub mod __interfaces {
        use wayland_client::protocol::__interfaces::*;
        use wayland_protocols::xdg::shell::client::__interfaces::*;
        wayland_scanner::generate_interfaces!("protocols/xdg-toplevel-icon-v1.xml");
    }
    use self::__interfaces::*;
    wayland_scanner::generate_client_code!("protocols/xdg-toplevel-icon-v1.xml");
}

#[cfg(target_os="linux")]
pub(super) fn set(globals:&super::Globals,toplevel:&wayland_protocols::xdg::shell::client::xdg_toplevel::XdgToplevel,
    image:&crate::WindowIcon)->anyhow::Result<bool> {
    use std::{io::Write,os::fd::{AsFd,FromRawFd,OwnedFd}};
    let Some(manager)=globals.icon_manager.as_ref() else {return Ok(false);};
    let raw=unsafe {libc::memfd_create(c"gpui-window-icon".as_ptr(),libc::MFD_CLOEXEC)};
    if raw<0 {return Err(std::io::Error::last_os_error().into());}
    let owned=unsafe {OwnedFd::from_raw_fd(raw)};
    let mut file=std::fs::File::from(owned);
    let pixels=image.bgra();file.write_all(&pixels)?;
    let pool=globals.shm.create_pool(file.as_fd(),pixels.len() as i32,&globals.qh,());
    let buffer=pool.create_buffer(0,image.size as i32,image.size as i32,image.size as i32*4,
        wayland_client::protocol::wl_shm::Format::Argb8888,&globals.qh,());
    let icon=manager.create_icon(&globals.qh,());
    icon.add_buffer(&buffer,1);
    manager.set_icon(toplevel,Some(&icon));
    // set_icon copies the immutable icon state. The protocol explicitly permits
    // destroying the icon afterwards; only then may its wl_buffer be destroyed.
    icon.destroy();buffer.destroy();pool.destroy();
    Ok(true)
}

#[cfg(not(target_os="linux"))]
pub(super) fn set(_globals:&super::Globals,_toplevel:&wayland_protocols::xdg::shell::client::xdg_toplevel::XdgToplevel,
    _image:&crate::WindowIcon)->anyhow::Result<bool> {Ok(false)}
