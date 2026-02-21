#[allow(dead_code)]
pub enum MouseEvent {
    SelectionDone { down_x: i32, down_y: i32, up_x: i32, up_y: i32 },
    Click,
    Quit,
}

#[cfg(target_os = "windows")]
mod windows;
#[cfg(target_os = "windows")]
pub use self::windows::*;

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
pub use self::macos::*;
