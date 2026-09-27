//! Wayland: file clipboard, dragging files out to other apps, and receiving file drops.
//!
//! winit has no drag-and-drop or rich clipboard support on Wayland, so this runs
//! its own event queue on the window's Wayland connection (the same approach
//! smithay-clipboard uses). Our own wl_pointer/wl_keyboard give us the input
//! serials that `set_selection` and `start_drag` require.

use std::io::{Read, Write};
use std::os::fd::{AsFd, AsRawFd, OwnedFd};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};

use wayland_client::protocol::wl_data_device::{self, WlDataDevice};
use wayland_client::protocol::wl_data_device_manager::{DndAction, WlDataDeviceManager};
use wayland_client::protocol::wl_data_offer::{self, WlDataOffer};
use wayland_client::protocol::wl_data_source::{self, WlDataSource};
use wayland_client::protocol::wl_keyboard::{self, WlKeyboard};
use wayland_client::protocol::wl_pointer::{self, WlPointer};
use wayland_client::protocol::wl_registry::{self, WlRegistry};
use wayland_client::protocol::wl_seat::{self, WlSeat};
use wayland_client::protocol::wl_surface::WlSurface;
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle, WEnum, event_created_child};

use super::{Cmd, Dropped, Payload};

type Offers = Mutex<Vec<String>>;
/// A drop whose data is still being read: the offer and the reader's result.
type PendingDrop = (WlDataOffer, Receiver<Vec<u8>>);

struct State {
    ddm: Option<WlDataDeviceManager>,
    seat: Option<WlSeat>,
    device: Option<WlDataDevice>,
    pointer: Option<WlPointer>,
    keyboard: Option<WlKeyboard>,
    surface: WlSurface,
    /// Serial of the mouse button currently held on our window (for start_drag).
    button_serial: Option<u32>,
    /// Latest input serial (for set_selection).
    input_serial: u32,
    selection: Option<WlDataSource>,
    drag: Option<WlDataSource>,
    selection_offer: Option<WlDataOffer>,
    dnd_offer: Option<WlDataOffer>,
    dnd_pos: (f64, f64),
    pending_drop: Option<PendingDrop>,
    drops: Arc<Mutex<Vec<Dropped>>>,
    dragging: Arc<AtomicBool>,
    ctx: egui::Context,
}

pub struct Handle {
    tx: Sender<Cmd>,
    pub drops: Arc<Mutex<Vec<Dropped>>>,
}

impl Handle {
    pub fn send(&self, cmd: Cmd) -> bool {
        self.tx.send(cmd).is_ok()
    }
}

/// Starts the Wayland helper thread.
///
/// # Safety
/// `display` and `surface` must be the live `wl_display` / `wl_surface` of our window,
/// valid for as long as the window exists.
pub unsafe fn start(
    display: *mut std::ffi::c_void,
    surface: *mut std::ffi::c_void,
    ctx: egui::Context,
) -> Option<Handle> {
    // SAFETY: the caller guarantees the display outlives us (it's the app's window).
    let backend = unsafe { wayland_client::backend::Backend::from_foreign_display(display.cast()) };
    let conn = Connection::from_backend(backend);
    // SAFETY: `surface` is our window's wl_surface.
    let id = unsafe { wayland_client::backend::ObjectId::from_ptr(WlSurface::interface(), surface.cast()) }.ok()?;
    let surface = WlSurface::from_id(&conn, id).ok()?;
    let (tx, rx) = channel();
    let drops = Arc::new(Mutex::new(Vec::new()));
    let dragging = Arc::new(AtomicBool::new(false));
    let (d2, g2) = (drops.clone(), dragging.clone());
    std::thread::Builder::new().name("wayland-dnd".into()).spawn(move || run(conn, surface, rx, d2, g2, ctx)).ok()?;
    Some(Handle { tx, drops })
}

fn run(
    conn: Connection,
    surface: WlSurface,
    rx: Receiver<Cmd>,
    drops: Arc<Mutex<Vec<Dropped>>>,
    dragging: Arc<AtomicBool>,
    ctx: egui::Context,
) {
    let mut queue = conn.new_event_queue::<State>();
    let qh = queue.handle();
    conn.display().get_registry(&qh, ());
    let mut state = State {
        ddm: None,
        seat: None,
        device: None,
        pointer: None,
        keyboard: None,
        surface,
        button_serial: None,
        input_serial: 0,
        selection: None,
        drag: None,
        selection_offer: None,
        dnd_offer: None,
        dnd_pos: (0.0, 0.0),
        pending_drop: None,
        drops,
        dragging,
        ctx,
    };
    if queue.roundtrip(&mut state).is_err() {
        return;
    }
    loop {
        if queue.dispatch_pending(&mut state).is_err() {
            return;
        }
        loop {
            match rx.try_recv() {
                Ok(cmd) => state.handle(cmd, &qh),
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => return,
            }
        }
        state.finish_drop();
        let _ = conn.flush();
        // Wait for events (winit may read them for us; the timeout keeps us polling).
        if let Some(guard) = queue.prepare_read() {
            let fd = guard.connection_fd().as_raw_fd();
            let mut pfd = libc::pollfd { fd, events: libc::POLLIN, revents: 0 };
            // SAFETY: one valid pollfd.
            let n = unsafe { libc::poll(&mut pfd, 1, 16) };
            if n > 0 {
                let _ = guard.read();
            }
        }
    }
}

fn new_source(state: &State, qh: &QueueHandle<State>, payload: Payload, mimes: &[&str]) -> Option<WlDataSource> {
    let src = state.ddm.as_ref()?.create_data_source(qh, Arc::new(payload));
    for m in mimes {
        src.offer(m.to_string());
    }
    Some(src)
}

impl State {
    fn handle(&mut self, cmd: Cmd, qh: &QueueHandle<State>) {
        match cmd {
            Cmd::Clipboard(payload) => {
                let mut mimes = vec![
                    "text/uri-list",
                    "x-special/gnome-copied-files",
                    "text/plain;charset=utf-8",
                    "text/plain",
                    "UTF8_STRING",
                ];
                if payload.cut {
                    mimes.push("application/x-kde-cutselection");
                }
                let (Some(src), Some(dev)) = (new_source(self, qh, payload, &mimes), self.device.clone()) else {
                    return;
                };
                dev.set_selection(Some(&src), self.input_serial);
                if let Some(old) = self.selection.replace(src) {
                    old.destroy();
                }
            }
            Cmd::Drag(payload) => {
                let Some(serial) = self.button_serial else { return };
                let mimes = ["text/uri-list", "text/plain;charset=utf-8", "text/plain", "UTF8_STRING"];
                let (Some(src), Some(dev)) = (new_source(self, qh, payload, &mimes), self.device.clone()) else {
                    return;
                };
                if src.version() >= 3 {
                    src.set_actions(DndAction::Copy);
                }
                dev.start_drag(Some(&src), &self.surface, None, serial);
                self.dragging.store(true, Ordering::SeqCst);
                if let Some(old) = self.drag.replace(src) {
                    old.destroy();
                }
            }
        }
    }

    /// Completes a drop once its data has been read.
    fn finish_drop(&mut self) {
        let Some((offer, rx)) = &self.pending_drop else { return };
        let Ok(bytes) = rx.try_recv() else { return };
        let paths = super::parse_uri_list(&String::from_utf8_lossy(&bytes));
        if offer.version() >= 3 {
            offer.finish();
        }
        offer.destroy();
        self.pending_drop = None;
        if !paths.is_empty() {
            self.drops.lock().unwrap().push(Dropped { paths });
            self.ctx.request_repaint();
        }
    }
}

impl Dispatch<WlRegistry, ()> for State {
    fn event(
        state: &mut Self,
        reg: &WlRegistry,
        ev: wl_registry::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_registry::Event::Global { name, interface, version } = ev {
            match interface.as_str() {
                "wl_seat" if state.seat.is_none() => {
                    let seat: WlSeat = reg.bind(name, version.min(5), qh, ());
                    state.seat = Some(seat);
                }
                "wl_data_device_manager" if state.ddm.is_none() => {
                    let ddm: WlDataDeviceManager = reg.bind(name, version.min(3), qh, ());
                    state.ddm = Some(ddm);
                }
                _ => {}
            }
            if state.device.is_none()
                && let (Some(ddm), Some(seat)) = (&state.ddm, &state.seat)
            {
                state.device = Some(ddm.get_data_device(seat, qh, ()));
            }
        }
    }
}

impl Dispatch<WlSeat, ()> for State {
    fn event(state: &mut Self, seat: &WlSeat, ev: wl_seat::Event, _: &(), _: &Connection, qh: &QueueHandle<Self>) {
        if let wl_seat::Event::Capabilities { capabilities: WEnum::Value(caps) } = ev {
            if caps.contains(wl_seat::Capability::Pointer) && state.pointer.is_none() {
                state.pointer = Some(seat.get_pointer(qh, ()));
            }
            if caps.contains(wl_seat::Capability::Keyboard) && state.keyboard.is_none() {
                state.keyboard = Some(seat.get_keyboard(qh, ()));
            }
        }
    }
}

impl Dispatch<WlPointer, ()> for State {
    fn event(state: &mut Self, _: &WlPointer, ev: wl_pointer::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        match ev {
            wl_pointer::Event::Button { serial, state: WEnum::Value(s), .. } => {
                state.input_serial = serial;
                state.button_serial = (s == wl_pointer::ButtonState::Pressed).then_some(serial);
            }
            wl_pointer::Event::Enter { serial, .. } => state.input_serial = serial,
            wl_pointer::Event::Leave { .. } => {}
            _ => {}
        }
    }
}

impl Dispatch<WlKeyboard, ()> for State {
    fn event(state: &mut Self, _: &WlKeyboard, ev: wl_keyboard::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        match ev {
            wl_keyboard::Event::Enter { serial, .. } | wl_keyboard::Event::Key { serial, .. } => {
                state.input_serial = serial;
            }
            // The keymap fd is closed when dropped; we don't need it.
            _ => {}
        }
    }
}

impl Dispatch<WlDataDeviceManager, ()> for State {
    fn event(
        _: &mut Self,
        _: &WlDataDeviceManager,
        _: <WlDataDeviceManager as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<WlDataDevice, ()> for State {
    fn event(
        state: &mut Self,
        _: &WlDataDevice,
        ev: wl_data_device::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match ev {
            wl_data_device::Event::Enter { serial, surface, x, y, id } => {
                if let Some(old) = state.dnd_offer.take() {
                    old.destroy();
                }
                state.dnd_pos = (x, y);
                if let Some(offer) = id {
                    let has_uris =
                        offer.data::<Offers>().is_some_and(|m| m.lock().unwrap().iter().any(|m| m == "text/uri-list"));
                    // Accept only file drops onto our own window, and only as a copy.
                    if has_uris && surface == state.surface {
                        offer.accept(serial, Some("text/uri-list".into()));
                        if offer.version() >= 3 {
                            offer.set_actions(DndAction::Copy, DndAction::Copy);
                        }
                    } else {
                        offer.accept(serial, None);
                    }
                    state.dnd_offer = Some(offer);
                }
            }
            wl_data_device::Event::Motion { x, y, .. } => state.dnd_pos = (x, y),
            wl_data_device::Event::Leave => {
                if let Some(o) = state.dnd_offer.take() {
                    o.destroy();
                }
            }
            wl_data_device::Event::Drop => {
                let Some(offer) = state.dnd_offer.take() else { return };
                let has_uris =
                    offer.data::<Offers>().is_some_and(|m| m.lock().unwrap().iter().any(|m| m == "text/uri-list"));
                if !has_uris {
                    offer.destroy();
                    return;
                }
                let Some((read, write)) = pipe() else {
                    offer.destroy();
                    return;
                };
                offer.receive("text/uri-list".into(), write.as_fd());
                drop(write);
                let (tx, rx) = channel();
                std::thread::spawn(move || {
                    let mut buf = Vec::new();
                    let _ = std::fs::File::from(read).take(16 << 20).read_to_end(&mut buf);
                    let _ = tx.send(buf);
                });
                state.pending_drop = Some((offer, rx));
            }
            wl_data_device::Event::Selection { id } => {
                if let Some(old) = std::mem::replace(&mut state.selection_offer, id) {
                    old.destroy();
                }
            }
            _ => {}
        }
    }

    event_created_child!(State, WlDataDevice, [
        wl_data_device::EVT_DATA_OFFER_OPCODE => (WlDataOffer, Mutex::new(Vec::new())),
    ]);
}

impl Dispatch<WlDataOffer, Offers> for State {
    fn event(
        _: &mut Self,
        _: &WlDataOffer,
        ev: wl_data_offer::Event,
        data: &Offers,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_data_offer::Event::Offer { mime_type } = ev {
            data.lock().unwrap().push(mime_type);
        }
    }
}

impl Dispatch<WlDataSource, Arc<Payload>> for State {
    fn event(
        state: &mut Self,
        src: &WlDataSource,
        ev: wl_data_source::Event,
        payload: &Arc<Payload>,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match ev {
            wl_data_source::Event::Send { mime_type, fd } => {
                let bytes = payload.bytes_for(&mime_type);
                // Write off-thread: the reader may be slow (or be us).
                std::thread::spawn(move || {
                    let _ = std::fs::File::from(fd).write_all(&bytes);
                });
            }
            wl_data_source::Event::Cancelled => {
                src.destroy();
                if state.drag.as_ref() == Some(src) {
                    state.drag = None;
                    state.dragging.store(false, Ordering::SeqCst);
                }
                if state.selection.as_ref() == Some(src) {
                    state.selection = None;
                }
            }
            wl_data_source::Event::DndFinished | wl_data_source::Event::DndDropPerformed
                if state.drag.as_ref() == Some(src) =>
            {
                state.dragging.store(false, Ordering::SeqCst);
                if matches!(ev, wl_data_source::Event::DndFinished) {
                    src.destroy();
                    state.drag = None;
                }
            }
            _ => {}
        }
    }
}

fn pipe() -> Option<(OwnedFd, OwnedFd)> {
    let mut fds = [0; 2];
    // SAFETY: `fds` has room for two descriptors; on success both are ours to own.
    if unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) } != 0 {
        return None;
    }
    use std::os::fd::FromRawFd;
    // SAFETY: freshly created, owned by nobody else.
    Some(unsafe { (OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])) })
}
