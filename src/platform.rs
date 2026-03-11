use color::{AlphaColor, Srgb};

use crate::config::ColorsConfig;
use crate::state::NetStatusKind;

pub fn color_for_status(status: NetStatusKind, colors: &ColorsConfig) -> &AlphaColor<Srgb> {
    match status {
        NetStatusKind::Connected => &colors.connected,
        NetStatusKind::Disconnected => &colors.disconnected,
        NetStatusKind::Slow => &colors.slow,
    }
}

pub trait Overlay: Send {
    /// Show the colored bar across applicable screens.
    fn show(&mut self, color: AlphaColor<Srgb>, fade_after_seconds: f64);
    /// Immediately hide all bars.
    fn hide(&mut self);
    /// Re-enumerate displays and recreate bar windows.
    fn refresh_displays(&mut self);
    /// Run the platform event loop (blocking). Called from the main thread.
    fn run_loop(&mut self);
}

#[cfg(target_os = "linux")]
pub fn create_overlay() -> Box<dyn Overlay> {
    Box::new(super::platform_linux::LinuxOverlay::new())
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub fn create_overlay() -> Box<dyn Overlay> {
    Box::new(NoopOverlay)
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
struct NoopOverlay;

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
impl Overlay for NoopOverlay {
    fn show(&mut self, _color: Color, _fade_after_seconds: f64) {}
    fn hide(&mut self) {}
    fn refresh_displays(&mut self) {}
    fn run_loop(&mut self) {
        std::thread::park();
    }
}
