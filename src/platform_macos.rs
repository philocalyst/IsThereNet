use crate::platform::Overlay;
use color::{AlphaColor, Srgb};
use objc2::rc::Retained;
use objc2::{MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{
    NSApplication, NSApplicationActivationPolicy, NSBackingStoreType, NSColor, NSScreen, NSView,
    NSWindow, NSWindowCollectionBehavior, NSWindowSharingType, NSWindowStyleMask,
};
use objc2_foundation::{NSPoint, NSRect, NSSize};
use std::sync::mpsc;
use std::time::Instant;

pub enum OverlayCommand {
    Show(AlphaColor<Srgb>, f64),
    Hide,
    RefreshDisplays,
    Stop,
}

pub struct MacOverlay {
    sender: mpsc::Sender<OverlayCommand>,
}

unsafe impl Send for MacOverlay {}

impl MacOverlay {
    pub fn new(sender: mpsc::Sender<OverlayCommand>) -> Self {
        Self { sender }
    }
}

impl Overlay for MacOverlay {
    fn show(&mut self, color: AlphaColor<Srgb>, fade_after_seconds: f64) {
        let _ = self
            .sender
            .send(OverlayCommand::Show(color, fade_after_seconds));
    }

    fn hide(&mut self) {
        let _ = self.sender.send(OverlayCommand::Hide);
    }

    fn refresh_displays(&mut self) {
        let _ = self.sender.send(OverlayCommand::RefreshDisplays);
    }

    fn run_loop(&mut self) {}
}

struct BarWindow {
    window: Retained<NSWindow>,
}

struct FadeState {
    fade_start: Instant,
    fade_after: f64,
    fading: bool,
}

/// Runs on the main thread. Receives commands from the async runtime and
/// manipulates AppKit overlay windows.
pub fn run_main_thread_loop(receiver: mpsc::Receiver<OverlayCommand>) {
    let mtm = MainThreadMarker::new().expect("must be called from the main thread");

    let app = NSApplication::sharedApplication(mtm);
    app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);

    let mut bars = create_bar_windows(mtm);
    let mut fade_state: Option<FadeState> = None;

    loop {
        // Process pending AppKit events
        while let Some(event) = unsafe {
            app.nextEventMatchingMask_untilDate_inMode_dequeue(
                objc2_app_kit::NSEventMask::Any,
                None,
                objc2_foundation::NSDefaultRunLoopMode,
                true,
            )
        } {
            app.sendEvent(&event);
        }

        // Process pending commands
        loop {
            match receiver.try_recv() {
                Ok(OverlayCommand::Show(color, fade_after)) => {
                    let color = color.to_rgba8();
                    let ns_color = NSColor::colorWithSRGBRed_green_blue_alpha(
                        color.r as f64 / 255.0,
                        color.g as f64 / 255.0,
                        color.b as f64 / 255.0,
                        color.a as f64 / 255.0,
                    );
                    show_bars(&mut bars, &ns_color, mtm);

                    if fade_after > 0.0 {
                        fade_state = Some(FadeState {
                            fade_start: Instant::now(),
                            fade_after,
                            fading: false,
                        });
                    } else {
                        fade_state = None;
                    }
                }
                Ok(OverlayCommand::Hide) => {
                    hide_bars(&mut bars);
                    fade_state = None;
                }
                Ok(OverlayCommand::RefreshDisplays) => {
                    hide_bars(&mut bars);
                    bars = create_bar_windows(mtm);
                }
                Ok(OverlayCommand::Stop) => return,
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => return,
            }
        }

        // Handle fade timing
        if let Some(ref mut state) = fade_state {
            let elapsed = state.fade_start.elapsed().as_secs_f64();
            if !state.fading && elapsed >= state.fade_after {
                state.fading = true;
            }
            if state.fading {
                let fade_elapsed = elapsed - state.fade_after;
                let fade_duration = 2.0;
                if fade_elapsed >= fade_duration {
                    hide_bars(&mut bars);
                    fade_state = None;
                } else {
                    let alpha = 1.0 - (fade_elapsed / fade_duration);
                    for bar in &bars {
                        bar.window.setAlphaValue(alpha.max(0.01));
                    }
                }
            }
        }

        // Brief sleep via run loop to avoid busy-waiting while keeping events flowing
        unsafe {
            let run_loop = objc2_foundation::NSRunLoop::currentRunLoop();
            let until = objc2_foundation::NSDate::dateWithTimeIntervalSinceNow(0.016);
            run_loop.runMode_beforeDate(objc2_foundation::NSDefaultRunLoopMode, &until);
        }
    }
}

fn create_bar_windows(mtm: MainThreadMarker) -> Vec<BarWindow> {
    let screens = NSScreen::screens(mtm);
    let mut bars = Vec::new();

    for screen in screens.iter() {
        let frame = screen.frame();
        let bar_rect = NSRect::new(
            NSPoint::new(
                frame.origin.x - 5.0,
                frame.origin.y + frame.size.height - 12.0,
            ),
            NSSize::new(frame.size.width + 10.0, 20.0),
        );

        let window = unsafe {
            NSWindow::initWithContentRect_styleMask_backing_defer_screen(
                NSWindow::alloc(mtm),
                bar_rect,
                NSWindowStyleMask::Borderless | NSWindowStyleMask::FullSizeContentView,
                NSBackingStoreType::Buffered,
                false,
                Some(&*screen),
            )
        };

        unsafe { window.setReleasedWhenClosed(false) };
        window.setBackgroundColor(Some(&NSColor::clearColor()));
        window.setLevel(unsafe { core_graphics::display::CGShieldingWindowLevel() as isize });
        window.setOpaque(false);
        window.setHasShadow(false);
        window.setIgnoresMouseEvents(true);
        window.setMovableByWindowBackground(false);
        window.setSharingType(NSWindowSharingType::None);
        window.setCollectionBehavior(
            NSWindowCollectionBehavior::CanJoinAllSpaces
                | NSWindowCollectionBehavior::Stationary
                | NSWindowCollectionBehavior::IgnoresCycle
                | NSWindowCollectionBehavior::FullScreenDisallowsTiling,
        );
        window.setAlphaValue(0.0);

        bars.push(BarWindow { window });
    }

    bars
}

fn show_bars(bars: &mut [BarWindow], color: &NSColor, mtm: MainThreadMarker) {
    for bar in bars.iter_mut() {
        let frame = bar.window.frame();

        let box_view = {
            let view = NSView::initWithFrame(
                NSView::alloc(mtm),
                NSRect::new(NSPoint::new(0.0, 10.0), NSSize::new(frame.size.width, 3.0)),
            );
            view.setWantsLayer(true);
            if let Some(layer) = view.layer() {
                let cg_color = color.CGColor();
                unsafe {
                    use objc2::msg_send;
                    let _: () = msg_send![&layer, setBackgroundColor: &*cg_color];
                }
            }
            view
        };

        let container = NSView::initWithFrame(
            NSView::alloc(mtm),
            NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(frame.size.width, 20.0)),
        );
        container.addSubview(&box_view);
        bar.window.setContentView(Some(&container));
        bar.window.setAlphaValue(1.0);
        bar.window.orderFront(None);
    }
}

fn hide_bars(bars: &mut [BarWindow]) {
    for bar in bars.iter_mut() {
        bar.window.setAlphaValue(0.0);
        bar.window.orderOut(None);
    }
}
