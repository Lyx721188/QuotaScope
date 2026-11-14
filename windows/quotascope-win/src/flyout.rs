//! The detail flyout: the hover card as its own topmost window, the way a
//! WinUI flyout is its own surface rather than a layer of the window it
//! points at.
//!
//! It exists as a separate window because the dock bar is real Mica, and a
//! DWM backdrop fills the whole window rectangle — the only way to have
//! Mica on the bar and more Mica on the card without a slab of it between
//! is to make the card a window of its own. The card draws no surface of
//! its own; its backdrop is the flyout's.
//!
//! The panel owns the flyout's lifetime; the flyout only reports pointer
//! traffic back through `WM_APP_CARD`, so the panel can decide when the
//! card ought to go away.

use windows::core::w;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::UI::Input::KeyboardAndMouse::{TrackMouseEvent, TME_LEAVE, TRACKMOUSEEVENT};
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::card::{draw_card, CardData};
use crate::d2d::{global_engine, scale_matrix, SwapchainCanvas};
use crate::geometry::{Edge, Metrics};
use crate::theme::panel;
use crate::winutil::{self, WM_APP_CARD};

pub struct Flyout {
    pub hwnd: HWND,
    canvas: Option<SwapchainCanvas>,
    glass_capture: crate::liquid_glass::DesktopGlass,
    glass_capture_enabled: bool,
    /// Whether the pointer is over the card right now — the panel checks
    /// it before hiding the card on a bar mouse-leave.
    pub pointer_inside: bool,
    bar: HWND,
    dpi: f64,
    units: (f64, f64),
    tracking_mouse: bool,
    hidden_at: Option<std::time::Instant>,
    scroll: f64,
    scroll_max: f64,
    pulse: bool,
    edge: Edge,
    pointer: f64,
    content_id: Option<String>,
    current_data: Option<CardData>,
    previous_data: Option<CardData>,
    transition_started: Option<std::time::Instant>,
}

impl Flyout {
    /// Builds the flyout. Returns a box because the window procedure holds
    /// a pointer into it from the first message onwards.
    pub fn new(bar: HWND) -> Box<Flyout> {
        let class_name = w!("QuotaScopeFlyout");
        unsafe {
            let wc = WNDCLASSW {
                lpfnWndProc: Some(flyout_wndproc),
                lpszClassName: class_name,
                hInstance: winutil::hinstance(),
                hCursor: winutil::arrow_cursor(),
                ..Default::default()
            };
            RegisterClassW(&wc);
        }

        let mut flyout = Box::new(Flyout {
            hwnd: HWND::default(),
            canvas: None,
            glass_capture: Default::default(),
            glass_capture_enabled: false,
            pointer_inside: false,
            bar,
            dpi: 1.0,
            units: (0.0, 0.0),
            tracking_mouse: false,
            hidden_at: None,
            scroll: 0.0,
            scroll_max: 0.0,
            pulse: false,
            edge: Edge::Right,
            pointer: 0.0,
            content_id: None,
            current_data: None,
            previous_data: None,
            transition_started: None,
        });
        unsafe {
            let hwnd = CreateWindowExW(
                WS_EX_NOREDIRECTIONBITMAP | WS_EX_TOOLWINDOW | WS_EX_TOPMOST | WS_EX_NOACTIVATE,
                class_name,
                w!("QuotaScope"),
                // Framed enough for DWM to paint its backdrop on; the
                // frame itself is swallowed by WM_NCCALCSIZE below.
                WS_CAPTION | WS_THICKFRAME,
                0,
                0,
                10,
                10,
                None,
                None,
                Some(winutil::hinstance()),
                None,
            )
            .expect("CreateWindowExW for flyout");
            flyout.hwnd = hwnd;
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, &*flyout as *const Flyout as isize);
            winutil::apply_system_backdrop(hwnd, panel::is_dark());
        }
        flyout
    }

    pub fn set_glass_capture(&mut self, enabled: bool) {
        self.glass_capture_enabled = enabled;
        if !enabled {
            self.glass_capture.clear();
        }
    }

    pub fn configure(&mut self, pulse: bool, edge: Edge, pointer: f64) {
        if self.pulse != pulse {
            self.canvas = None;
            self.pulse = pulse;
            if pulse {
                winutil::apply_pulse_backdrop(self.hwnd);
            } else {
                winutil::restore_panel_backdrop(self.hwnd, panel::is_dark());
            }
        }
        self.edge = edge;
        self.pointer = pointer;
    }

    /// Moves, sizes, shows. `units` is the card's design size; the canvas
    /// is kept in step with it and the monitor's DPI.
    pub fn show_at(&mut self, x: i32, y: i32, units: (f64, f64), dpi: f64) {
        self.hidden_at = None;
        let px_w = (units.0 * dpi).ceil() as i32;
        let px_h = (units.1 * dpi).ceil() as i32;
        let resized = self.units != units || self.dpi != dpi;
        if self.canvas.is_none() {
            self.canvas = Some(
                SwapchainCanvas::new(self.hwnd, global_engine(), px_w.max(1), px_h.max(1))
                    .expect("flyout canvas"),
            );
        } else if resized {
            self.canvas
                .as_mut()
                .expect("flyout canvas")
                .resize(px_w.max(1), px_h.max(1))
                .expect("flyout canvas resize");
        }
        self.units = units;
        self.dpi = dpi;
        unsafe {
            let _ = SetWindowPos(
                self.hwnd,
                Some(HWND_TOPMOST),
                x,
                y,
                px_w.max(1),
                px_h.max(1),
                SWP_NOACTIVATE,
            );
            let _ = ShowWindow(self.hwnd, SW_SHOWNOACTIVATE);
        }
    }

    pub fn hide(&mut self) {
        if self.is_shown() {
            self.pointer_inside = false;
            self.hidden_at = Some(std::time::Instant::now());
            unsafe {
                let _ = ShowWindow(self.hwnd, SW_HIDE);
            }
        }
        self.content_id = None;
        self.current_data = None;
        self.previous_data = None;
        self.transition_started = None;
    }

    pub fn is_transitioning(&self) -> bool {
        self.transition_started
            .is_some_and(|start| start.elapsed().as_secs_f64() < 0.20)
    }

    pub fn release_idle_canvas(&mut self) {
        if self
            .hidden_at
            .is_some_and(|at| at.elapsed() >= std::time::Duration::from_secs(60))
        {
            self.canvas = None;
            self.hidden_at = None;
        }
    }

    /// Slides the window without touching its size — the panel's spring
    /// calls this while the card glides between rings.
    pub fn move_to(&mut self, x: i32, y: i32) {
        unsafe {
            let _ = SetWindowPos(
                self.hwnd,
                Some(HWND_TOPMOST),
                x,
                y,
                0,
                0,
                SWP_NOACTIVATE | SWP_NOSIZE,
            );
        }
    }

    pub fn is_shown(&self) -> bool {
        unsafe { IsWindowVisible(self.hwnd).as_bool() }
    }

    /// Draws the card's content over its Mica. `alpha` is the entrance
    /// fade, driven by the panel's spring.
    pub fn draw(&mut self, data: &CardData, m: &Metrics, alpha: f32) {
        if self.pulse {
            let id = format!("{}|{}", data.usage.account.id(), data.title);
            if self.content_id.as_ref().is_some_and(|old| old != &id) {
                self.previous_data = self.current_data.take();
                self.transition_started = Some(std::time::Instant::now());
                self.scroll = 0.0;
            }
            self.content_id = Some(id);
            self.current_data = Some(data.clone());
            if self
                .transition_started
                .is_some_and(|start| start.elapsed().as_secs_f64() >= 0.20)
            {
                self.previous_data = None;
                self.transition_started = None;
            }
        }
        let offset = if self.pulse {
            match self.edge {
                Edge::Left => (m.s(20.0), 0.0),
                Edge::Top => (0.0, m.s(20.0)),
                _ => (0.0, 0.0),
            }
        } else {
            (0.0, 0.0)
        };
        let body_height = self.units.1
            - if self.pulse && !self.edge.is_vertical() {
                m.s(20.0)
            } else {
                0.0
            };
        self.scroll_max = (crate::card::body_size(m, data).1 - body_height).max(0.0);
        self.scroll = self.scroll.clamp(0.0, self.scroll_max);
        let Some(canvas) = self.canvas.as_ref() else {
            return;
        };
        let backdrop = if self.glass_capture_enabled {
            self.glass_capture.frame(self.hwnd, &canvas.rt)
        } else {
            None
        };
        canvas.begin();
        unsafe {
            let transform = if self.pulse {
                let p = f64::from(alpha.clamp(0.0, 1.0));
                let s = 0.88 + 0.12 * p;
                let direction = if matches!(self.edge, Edge::Left | Edge::Top) {
                    1.0
                } else {
                    -1.0
                };
                let anchor_x = if self.edge.is_vertical() && direction < 0.0 {
                    self.units.0
                } else if !self.edge.is_vertical() {
                    self.units.0 / 2.0
                } else {
                    0.0
                };
                let anchor_y = if !self.edge.is_vertical() && direction < 0.0 {
                    self.units.1
                } else if self.edge.is_vertical() {
                    self.units.1 / 2.0
                } else {
                    0.0
                };
                let slide = (1.0 - p) * 10.0 * m.scale * direction;
                windows_numerics::Matrix3x2 {
                    M11: (s * self.dpi) as f32,
                    M12: 0.0,
                    M21: 0.0,
                    M22: (s * self.dpi) as f32,
                    M31: ((anchor_x * (1.0 - s)
                        + if self.edge.is_vertical() { slide } else { 0.0 })
                        * self.dpi) as f32,
                    M32: ((anchor_y * (1.0 - s)
                        + if self.edge.is_vertical() { 0.0 } else { slide })
                        * self.dpi) as f32,
                }
            } else {
                scale_matrix(self.dpi as f32, self.dpi as f32)
            };
            let _ = canvas.rt.SetTransform(&transform);
        }
        let painter = crate::d2d::Painter {
            rt: &canvas.rt,
            engine: global_engine(),
        };
        if self.pulse {
            let _ = crate::pulse_rail::bubble(
                &painter,
                self.units.0,
                self.units.1,
                m.scale,
                self.edge,
                self.pointer,
                alpha,
                m.glass,
                backdrop.as_ref(),
            );
        }
        let header = crate::card::header_height(m);
        let body_width = self.units.0
            - if self.pulse && self.edge.is_vertical() {
                m.s(20.0)
            } else {
                0.0
            };
        // Content never spills into the pointer, even for long provider names.
        unsafe {
            canvas.rt.PushAxisAlignedClip(
                &crate::d2d::rect(
                    offset.0 as f32,
                    offset.1 as f32,
                    body_width.max(0.0) as f32,
                    body_height.max(0.0) as f32,
                ),
                windows::Win32::Graphics::Direct2D::D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
            );
        }
        let paint_content = |item: &CardData, row_alpha: f32, header_alpha: f32, shift: f64| {
            unsafe {
                canvas.rt.PushAxisAlignedClip(
                    &crate::d2d::rect(
                        offset.0 as f32,
                        (offset.1 + header) as f32,
                        body_width.max(0.0) as f32,
                        (body_height - header).max(0.0) as f32,
                    ),
                    windows::Win32::Graphics::Direct2D::D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
                );
            }
            let _ = draw_card(
                &painter,
                m,
                (offset.0, offset.1 - self.scroll + shift),
                item,
                row_alpha,
            );
            unsafe {
                canvas.rt.PopAxisAlignedClip();
            }
            let _ = crate::card::draw_card_header(&painter, m, offset, item, header_alpha);
        };
        if let (Some(previous), Some(start)) = (&self.previous_data, self.transition_started) {
            let elapsed = start.elapsed().as_secs_f64();
            let ease = |value: f64| {
                let t = value.clamp(0.0, 1.0);
                1.0 - (1.0 - t) * (1.0 - t)
            };
            let old = (1.0 - ease(elapsed / 0.06)) as f32 * alpha;
            let head = ease(elapsed / 0.10) as f32;
            let rows = ease((elapsed - 0.06) / 0.14) as f32;
            paint_content(previous, old, old, 0.0);
            paint_content(
                data,
                rows * alpha,
                head * alpha,
                -m.s(6.0) * (1.0 - f64::from(rows)),
            );
        } else {
            paint_content(data, alpha, alpha, 0.0);
        }
        unsafe {
            canvas.rt.PopAxisAlignedClip();
        }
        canvas.present();
    }
}

impl Drop for Flyout {
    fn drop(&mut self) {
        self.canvas = None;
        unsafe {
            SetWindowLongPtrW(self.hwnd, GWLP_USERDATA, 0);
            let _ = DestroyWindow(self.hwnd);
        }
    }
}

unsafe extern "system" fn flyout_wndproc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    let state = GetWindowLongPtrW(hwnd, GWLP_USERDATA);
    if state == 0 {
        return DefWindowProcW(hwnd, msg, wparam, lparam);
    }
    let flyout = &mut *(state as *mut Flyout);

    match msg {
        WM_MOUSEWHEEL => {
            let delta = ((wparam.0 >> 16) as u16) as i16;
            flyout.scroll =
                (flyout.scroll - f64::from(delta) / 120.0 * 48.0).clamp(0.0, flyout.scroll_max);
            let _ = SendMessageW(flyout.bar, WM_APP_CARD, Some(WPARAM(1)), Some(LPARAM(0)));
            LRESULT(0)
        }
        WM_MOUSEMOVE => {
            flyout.pointer_inside = true;
            flyout.track_mouse();
            // The panel owns the show/hide decision.
            let _ = SendMessageW(flyout.bar, WM_APP_CARD, Some(WPARAM(1)), Some(LPARAM(0)));
            LRESULT(0)
        }
        // WM_MOUSELEAVE lives in UI::Controls in the windows metadata.
        windows::Win32::UI::Controls::WM_MOUSELEAVE => {
            flyout.pointer_inside = false;
            flyout.tracking_mouse = false;
            let _ = SendMessageW(flyout.bar, WM_APP_CARD, Some(WPARAM(0)), Some(LPARAM(0)));
            LRESULT(0)
        }
        // The card is read-only, but it still takes the click — a press on
        // the card must not fall through to whatever sits beneath it.
        WM_LBUTTONDOWN | WM_LBUTTONUP | WM_LBUTTONDBLCLK => LRESULT(0),
        WM_NCHITTEST => LRESULT(HTCLIENT as isize),
        // All frame, all client — see the panel's proc.
        WM_NCCALCSIZE if wparam.0 != 0 => LRESULT(0),
        WM_DESTROY => LRESULT(0),
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

impl Flyout {
    fn track_mouse(&mut self) {
        if self.tracking_mouse {
            return;
        }
        self.tracking_mouse = true;
        unsafe {
            let mut tme = TRACKMOUSEEVENT {
                cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                dwFlags: TME_LEAVE,
                hwndTrack: self.hwnd,
                dwHoverTime: 0,
            };
            let _ = TrackMouseEvent(&mut tme);
        }
    }
}
