use iced::window::Window;

#[derive(Default)]
pub struct MouseCapture {
    active: bool,
    #[cfg(target_os = "linux")]
    wayland: Option<wayland::Capture>,
}

impl MouseCapture {
    pub const RECLIP_ON_MOVE: bool = cfg!(target_os = "windows");

    pub fn set(&mut self, window: &dyn Window, capture: bool) -> anyhow::Result<()> {
        if self.active == capture && !(capture && Self::RECLIP_ON_MOVE) {
            return Ok(());
        }

        self.confine(window, capture)?;
        self.active = capture;

        Ok(())
    }

    pub fn poll(&mut self) {
        #[cfg(target_os = "linux")]
        if let Some(capture) = &mut self.wayland {
            capture.poll();
        }
    }
}

#[cfg(target_os = "linux")]
impl MouseCapture {
    fn confine(&mut self, window: &dyn Window, capture: bool) -> anyhow::Result<()> {
        use iced::window::raw_window_handle::RawWindowHandle;

        match window.window_handle()?.as_raw() {
            RawWindowHandle::Wayland(surface) => {
                if self.wayland.is_none() && capture {
                    self.wayland = Some(wayland::Capture::new(window, surface)?);
                }

                if let Some(wayland) = &mut self.wayland {
                    wayland.set(capture)?;
                }

                Ok(())
            }
            RawWindowHandle::Xlib(handle) => self::confine_x11(window, handle.window, capture),
            _ => anyhow::bail!("mouse confinement is unsupported on this window system"),
        }
    }
}

#[cfg(target_os = "linux")]
fn confine_x11(window: &dyn Window, target: u64, capture: bool) -> anyhow::Result<()> {
    use iced::window::raw_window_handle::RawDisplayHandle;
    use std::sync::OnceLock;
    use x11_dl::xlib;

    static XLIB: OnceLock<Result<xlib::Xlib, String>> = OnceLock::new();

    let RawDisplayHandle::Xlib(display) = window.display_handle()?.as_raw() else {
        anyhow::bail!("missing X11 display");
    };

    let display = display.display.ok_or_else(|| anyhow::anyhow!("missing X11 display"))?;
    let xlib = XLIB
        .get_or_init(|| xlib::Xlib::open().map_err(|error| error.to_string()))
        .as_ref()
        .map_err(|error| anyhow::anyhow!("{error}"))?;

    // SAFETY: iced lends live Xlib handles on the window's event thread.
    unsafe {
        (xlib.XUngrabPointer)(display.as_ptr().cast(), xlib::CurrentTime);

        if capture {
            let result = (xlib.XGrabPointer)(
                display.as_ptr().cast(),
                target,
                xlib::True,
                (xlib::ButtonPressMask
                    | xlib::ButtonReleaseMask
                    | xlib::PointerMotionMask
                    | xlib::EnterWindowMask
                    | xlib::LeaveWindowMask) as u32,
                xlib::GrabModeAsync,
                xlib::GrabModeAsync,
                target,
                0,
                xlib::CurrentTime,
            );
            anyhow::ensure!(result == xlib::GrabSuccess, "X11 mouse capture failed: {result}");
        }

        (xlib.XFlush)(display.as_ptr().cast());
    }

    Ok(())
}

#[cfg(target_os = "windows")]
impl MouseCapture {
    fn confine(&mut self, window: &dyn Window, capture: bool) -> anyhow::Result<()> {
        use iced::window::raw_window_handle::RawWindowHandle;
        use windows_sys::Win32::Foundation::{POINT, RECT};
        use windows_sys::Win32::Graphics::Gdi::ClientToScreen;
        use windows_sys::Win32::UI::WindowsAndMessaging::{ClipCursor, GetClientRect};

        let RawWindowHandle::Win32(handle) = window.window_handle()?.as_raw() else {
            anyhow::bail!("mouse confinement is unsupported on this window system");
        };

        // SAFETY: iced lends a live HWND; all output pointers are valid stack storage.
        unsafe {
            let mut rect = RECT::default();

            if capture {
                let hwnd = handle.hwnd.get() as _;
                anyhow::ensure!(GetClientRect(hwnd, &mut rect) != 0, "GetClientRect failed");

                let mut origin = POINT::default();
                anyhow::ensure!(ClientToScreen(hwnd, &mut origin) != 0, "ClientToScreen failed");

                rect.left += origin.x;
                rect.right += origin.x;
                rect.top += origin.y;
                rect.bottom += origin.y;
            }

            anyhow::ensure!(
                ClipCursor(if capture { &rect } else { std::ptr::null() }) != 0,
                "ClipCursor failed"
            );
        }

        Ok(())
    }
}

#[cfg(not(any(target_os = "linux", target_os = "windows")))]
impl MouseCapture {
    fn confine(&mut self, _window: &dyn Window, _capture: bool) -> anyhow::Result<()> {
        anyhow::bail!("mouse confinement is unsupported on this window system")
    }
}

#[cfg(target_os = "windows")]
impl Drop for MouseCapture {
    fn drop(&mut self) {
        if self.active {
            // SAFETY: a null rectangle releases this process's cursor clip on window close.
            unsafe {
                windows_sys::Win32::UI::WindowsAndMessaging::ClipCursor(std::ptr::null());
            }
        }
    }
}

#[cfg(target_os = "linux")]
mod wayland {
    use iced::window::Window;
    use iced::window::raw_window_handle::{RawDisplayHandle, WaylandWindowHandle};
    use wayland_client::backend::{Backend, ObjectId};
    use wayland_client::globals::{GlobalListContents, registry_queue_init};
    use wayland_client::protocol::{wl_pointer, wl_registry, wl_seat, wl_surface};
    use wayland_client::{Connection, Dispatch, EventQueue, Proxy, QueueHandle, delegate_noop};
    use wayland_protocols::wp::pointer_constraints::zv1::client::zwp_confined_pointer_v1::ZwpConfinedPointerV1;
    use wayland_protocols::wp::pointer_constraints::zv1::client::zwp_pointer_constraints_v1::{
        Lifetime, ZwpPointerConstraintsV1,
    };

    pub struct Capture {
        connection: Connection,
        queue: EventQueue<State>,
        constraints: ZwpPointerConstraintsV1,
        seat: wl_seat::WlSeat,
        surface: wl_surface::WlSurface,
        confined: Option<(ZwpConfinedPointerV1, wl_pointer::WlPointer)>,
    }

    impl Capture {
        pub fn new(window: &dyn Window, surface: WaylandWindowHandle) -> anyhow::Result<Self> {
            let RawDisplayHandle::Wayland(display) = window.display_handle()?.as_raw() else {
                anyhow::bail!("missing Wayland display");
            };

            // SAFETY: the handles belong to iced's live window/display. This backend borrows
            // the display, and never destroys the foreign surface.
            let backend = unsafe { Backend::from_foreign_display(display.display.as_ptr().cast()) };
            let connection = Connection::from_backend(backend);

            let surface_id =
                unsafe { ObjectId::from_ptr(wl_surface::WlSurface::interface(), surface.surface.as_ptr().cast())? };
            let surface = wl_surface::WlSurface::from_id(&connection, surface_id)?;

            let (globals, queue) = registry_queue_init::<State>(&connection)?;
            let qh = queue.handle();

            let constraints = globals
                .bind(&qh, 1..=1, ())
                .map_err(|_| anyhow::anyhow!("compositor does not support pointer confinement"))?;

            let seat = globals
                .bind(&qh, 5..=5, ())
                .map_err(|_| anyhow::anyhow!("missing Wayland seat"))?;

            Ok(Self {
                connection,
                queue,
                constraints,
                seat,
                surface,
                confined: None,
            })
        }

        pub fn set(&mut self, capture: bool) -> anyhow::Result<()> {
            if !capture {
                self::release(&mut self.confined);
            } else if self.confined.is_none() {
                let qh = self.queue.handle();
                let pointer = self.seat.get_pointer(&qh, ());
                let confined =
                    self.constraints
                        .confine_pointer(&self.surface, &pointer, None, Lifetime::Persistent, &qh, ());

                self.confined = Some((confined, pointer));
            }

            self.connection.flush()?;

            Ok(())
        }

        pub fn poll(&mut self) {
            let _ = self.queue.dispatch_pending(&mut State);
        }
    }

    impl Drop for Capture {
        fn drop(&mut self) {
            self::release(&mut self.confined);

            self.constraints.destroy();
            self.seat.release();

            let _ = self.connection.flush();
        }
    }

    fn release(confined: &mut Option<(ZwpConfinedPointerV1, wl_pointer::WlPointer)>) {
        if let Some((confined, pointer)) = confined.take() {
            confined.destroy();
            pointer.release();
        }
    }

    struct State;

    impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for State {
        fn event(
            _: &mut Self,
            _: &wl_registry::WlRegistry,
            _: wl_registry::Event,
            _: &GlobalListContents,
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
        }
    }

    delegate_noop!(State: ignore wl_seat::WlSeat);
    delegate_noop!(State: ignore wl_pointer::WlPointer);
    delegate_noop!(State: ignore ZwpPointerConstraintsV1);
    delegate_noop!(State: ignore ZwpConfinedPointerV1);
}
