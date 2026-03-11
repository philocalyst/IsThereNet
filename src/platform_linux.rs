use crate::platform::{Color, Overlay};
use std::time::Instant;
use x11rb::connection::Connection;
use x11rb::protocol::randr::ConnectionExt as RandrConnectionExt;
use x11rb::protocol::xproto::{
    self, ColormapAlloc, ConnectionExt, CreateWindowAux, EventMask, WindowClass,
};
use x11rb::rust_connection::RustConnection;

struct BarWindow {
    window_id: u32,
    colormap: u32,
    width: u16,
}

struct FadeState {
    fade_start: Instant,
    fade_after: f64,
    fading: bool,
}

pub struct LinuxOverlay {
    connection: Option<RustConnection>,
    screen_num: usize,
    bars: Vec<BarWindow>,
    fade_state: Option<FadeState>,
    current_color: Option<Color>,
}

impl LinuxOverlay {
    pub fn new() -> Self {
        let (connection, screen_num) = match RustConnection::connect(None) {
            Ok(result) => result,
            Err(error) => {
                tracing::error!("Failed to connect to X11 display: {error}");
                return Self {
                    connection: None,
                    screen_num: 0,
                    bars: Vec::new(),
                    fade_state: None,
                    current_color: None,
                };
            }
        };

        let mut overlay = Self {
            connection: Some(connection),
            screen_num,
            bars: Vec::new(),
            fade_state: None,
            current_color: None,
        };
        overlay.refresh_displays();
        overlay
    }

    fn create_bars(&mut self) {
        let connection = match &self.connection {
            Some(connection) => connection,
            None => return,
        };

        let screen = &connection.setup().roots[self.screen_num];
        let root = screen.root;
        let depth = screen.root_depth;
        let bar_height: u16 = 3;

        // Try to get monitor info via RandR
        let monitors = match connection.randr_get_monitors(root, true) {
            Ok(cookie) => match cookie.reply() {
                Ok(reply) => reply
                    .monitors
                    .iter()
                    .map(|monitor| (monitor.x, monitor.y, monitor.width, monitor.height))
                    .collect::<Vec<_>>(),
                Err(_) => vec![(0, 0, screen.width_in_pixels, screen.height_in_pixels)],
            },
            Err(_) => vec![(0, 0, screen.width_in_pixels, screen.height_in_pixels)],
        };

        for (x, y, width, _height) in monitors {
            let colormap = match connection.generate_id() {
                Ok(id) => id,
                Err(error) => {
                    tracing::error!("Failed to generate X11 colormap id: {error}");
                    continue;
                }
            };

            if let Err(error) = connection.create_colormap(
                ColormapAlloc::NONE,
                colormap,
                root,
                screen.root_visual,
            ) {
                tracing::error!("Failed to create X11 colormap: {error}");
                continue;
            }

            let window_id = match connection.generate_id() {
                Ok(id) => id,
                Err(error) => {
                    tracing::error!("Failed to generate X11 window id: {error}");
                    continue;
                }
            };

            let values = CreateWindowAux::new()
                .background_pixel(0)
                .border_pixel(0)
                .override_redirect(1)
                .event_mask(EventMask::EXPOSURE)
                .colormap(colormap);

            if let Err(error) = connection.create_window(
                depth,
                window_id,
                root,
                x,
                y,
                width,
                bar_height,
                0,
                WindowClass::INPUT_OUTPUT,
                screen.root_visual,
                &values,
            ) {
                tracing::error!("Failed to create X11 window: {error}");
                continue;
            }

            // Set window to stay above others
            let net_wm_state = connection.intern_atom(false, b"_NET_WM_STATE");
            let net_wm_state_above = connection.intern_atom(false, b"_NET_WM_STATE_ABOVE");
            if let (Ok(state_cookie), Ok(above_cookie)) = (net_wm_state, net_wm_state_above) {
                if let (Ok(state_atom), Ok(above_atom)) =
                    (state_cookie.reply(), above_cookie.reply())
                {
                    let _ = connection.change_property32(
                        xproto::PropMode::REPLACE,
                        window_id,
                        state_atom.atom,
                        xproto::AtomEnum::ATOM,
                        &[above_atom.atom],
                    );
                }
            }

            self.bars.push(BarWindow {
                window_id,
                colormap,
                width,
            });
        }

        if let Err(error) = connection.flush() {
            tracing::error!("Failed to flush X11 connection: {error}");
        }
    }
}

impl Overlay for LinuxOverlay {
    fn show(&mut self, color: Color, fade_after_seconds: f64) {
        let connection = match &self.connection {
            Some(connection) => connection,
            None => return,
        };

        self.current_color = Some(color);

        let screen = &connection.setup().roots[self.screen_num];
        let pixel = ((255.0 * color.r) as u32) << 16
            | ((255.0 * color.g) as u32) << 8
            | (255.0 * color.b) as u32;

        for bar in &self.bars {
            let gc_id = match connection.generate_id() {
                Ok(id) => id,
                Err(_) => continue,
            };
            let gc_values = xproto::CreateGCAux::new().foreground(pixel);
            if connection
                .create_gc(gc_id, bar.window_id, &gc_values)
                .is_err()
            {
                continue;
            }

            let _ = connection.map_window(bar.window_id);
            let _ = connection.poly_fill_rectangle(
                bar.window_id,
                gc_id,
                &[xproto::Rectangle {
                    x: 0,
                    y: 0,
                    width: bar.width,
                    height: 3,
                }],
            );
            let _ = connection.free_gc(gc_id);

            // Raise to top
            let values = xproto::ConfigureWindowAux::new()
                .stack_mode(xproto::StackMode::ABOVE);
            let _ = connection.configure_window(bar.window_id, &values);
        }

        if let Err(error) = connection.flush() {
            tracing::error!("Failed to flush X11 connection: {error}");
        }

        if fade_after_seconds > 0.0 {
            self.fade_state = Some(FadeState {
                fade_start: Instant::now(),
                fade_after: fade_after_seconds,
                fading: false,
            });
        } else {
            self.fade_state = None;
        }
    }

    fn hide(&mut self) {
        let connection = match &self.connection {
            Some(connection) => connection,
            None => return,
        };

        for bar in &self.bars {
            let _ = connection.unmap_window(bar.window_id);
        }
        let _ = connection.flush();
        self.fade_state = None;
    }

    fn refresh_displays(&mut self) {
        self.hide();

        let connection = match &self.connection {
            Some(connection) => connection,
            None => return,
        };

        for bar in &self.bars {
            let _ = connection.destroy_window(bar.window_id);
            let _ = connection.free_colormap(bar.colormap);
        }
        let _ = connection.flush();
        self.bars.clear();

        self.create_bars();
    }

    fn run_loop(&mut self) {
        let connection = match &self.connection {
            Some(connection) => connection,
            None => {
                std::thread::park();
                return;
            }
        };

        loop {
            // Process X11 events
            while let Ok(Some(_event)) = connection.poll_for_event() {
                // Handle expose events by redrawing if we have a color
                // (simplified - just keeps windows mapped)
            }

            // Handle fade timing
            if let Some(ref mut state) = self.fade_state {
                let elapsed = state.fade_start.elapsed().as_secs_f64();
                if !state.fading && elapsed >= state.fade_after {
                    state.fading = true;
                }
                if state.fading {
                    let fade_elapsed = elapsed - state.fade_after;
                    let fade_duration = 2.0;
                    if fade_elapsed >= fade_duration {
                        // X11 doesn't have native window alpha for override-redirect windows,
                        // so we just unmap
                        for bar in &self.bars {
                            let _ = connection.unmap_window(bar.window_id);
                        }
                        let _ = connection.flush();
                        self.fade_state = None;
                    }
                }
            }

            std::thread::sleep(std::time::Duration::from_millis(50));
        }
    }
}
