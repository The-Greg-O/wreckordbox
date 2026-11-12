// Prevents an extra console window on Windows in release.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    rbxport_lib::appimage_wayland::reexec_with_host_wayland_if_needed();
    rbxport_lib::run();
}
