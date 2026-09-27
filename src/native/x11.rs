//! X11: owning the CLIPBOARD selection with file formats, and dragging files out
//! of the window with the XDND protocol (the drag is tracked by polling the
//! pointer, so it works while winit holds the implicit pointer grab).

use std::sync::mpsc::{Receiver, Sender, TryRecvError, channel};
use std::time::{Duration, Instant};

use x11rb::connection::Connection;
use x11rb::protocol::Event;
use x11rb::protocol::xproto::{
    self, AtomEnum, ClientMessageEvent, ConnectionExt, CreateWindowAux, EventMask, KeyButMask, PropMode,
    SelectionNotifyEvent, SelectionRequestEvent, Window, WindowClass,
};
use x11rb::rust_connection::RustConnection;
use x11rb::wrapper::ConnectionExt as _;

use super::{Cmd, Payload};

x11rb::atom_manager! {
    Atoms: AtomsCookie {
        CLIPBOARD,
        TARGETS,
        UTF8_STRING,
        TEXT_PLAIN_UTF8: b"text/plain;charset=utf-8",
        TEXT_PLAIN: b"text/plain",
        URI_LIST: b"text/uri-list",
        GNOME_FILES: b"x-special/gnome-copied-files",
        KDE_CUT: b"application/x-kde-cutselection",
        XdndAware,
        XdndSelection,
        XdndEnter,
        XdndPosition,
        XdndStatus,
        XdndLeave,
        XdndDrop,
        XdndFinished,
        XdndActionCopy,
    }
}

pub struct Handle {
    tx: Sender<Cmd>,
}

impl Handle {
    pub fn send(&self, cmd: Cmd) -> bool {
        self.tx.send(cmd).is_ok()
    }
}

pub fn start(own_window: Window) -> Option<Handle> {
    let (conn, screen) = x11rb::connect(None).ok()?;
    let root = conn.setup().roots[screen].root;
    let atoms = Atoms::new(&conn).ok()?.reply().ok()?;
    let win = conn.generate_id().ok()?;
    conn.create_window(
        x11rb::COPY_DEPTH_FROM_PARENT,
        win,
        root,
        -10,
        -10,
        1,
        1,
        0,
        WindowClass::INPUT_OUTPUT,
        x11rb::COPY_FROM_PARENT,
        &CreateWindowAux::new().override_redirect(1).event_mask(EventMask::PROPERTY_CHANGE),
    )
    .ok()?;
    conn.flush().ok()?;
    let (tx, rx) = channel();
    std::thread::Builder::new()
        .name("x11-dnd".into())
        .spawn(move || {
            let mut s = State { conn, root, atoms, win, own_window, clipboard: None, xdnd: None, drag: None };
            s.run(rx);
        })
        .ok()?;
    Some(Handle { tx })
}

struct Drag {
    target: Option<(Window, u32)>,
    accepted: bool,
    waiting: bool,
    last: (i16, i16),
    started: Instant,
}

struct State {
    conn: RustConnection,
    root: Window,
    atoms: Atoms,
    win: Window,
    /// Our app's window: drags over it stay in-app.
    own_window: Window,
    clipboard: Option<Payload>,
    xdnd: Option<Payload>,
    drag: Option<Drag>,
}

impl State {
    fn run(&mut self, rx: Receiver<Cmd>) {
        loop {
            loop {
                match rx.try_recv() {
                    Ok(Cmd::Clipboard(p)) => {
                        self.clipboard = Some(p);
                        let _ = self.conn.set_selection_owner(self.win, self.atoms.CLIPBOARD, x11rb::CURRENT_TIME);
                    }
                    Ok(Cmd::Drag(p)) => {
                        if self.drag.is_none() {
                            self.xdnd = Some(p);
                            let _ =
                                self.conn.set_selection_owner(self.win, self.atoms.XdndSelection, x11rb::CURRENT_TIME);
                            self.drag = Some(Drag {
                                target: None,
                                accepted: false,
                                waiting: false,
                                last: (i16::MIN, i16::MIN),
                                started: Instant::now(),
                            });
                        }
                    }
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => return,
                }
            }
            while let Ok(Some(ev)) = self.conn.poll_for_event() {
                self.on_event(ev);
            }
            if self.drag.is_some() {
                self.track_drag();
            }
            let _ = self.conn.flush();
            std::thread::sleep(Duration::from_millis(if self.drag.is_some() { 12 } else { 30 }));
        }
    }

    fn on_event(&mut self, ev: Event) {
        match ev {
            Event::SelectionRequest(req) => self.serve(req),
            Event::SelectionClear(e) if e.selection == self.atoms.CLIPBOARD => self.clipboard = None,
            Event::ClientMessage(m) if m.type_ == self.atoms.XdndStatus => {
                if let Some(d) = &mut self.drag {
                    d.waiting = false;
                    d.accepted = m.data.as_data32()[1] & 1 == 1;
                }
            }
            Event::ClientMessage(m) if m.type_ == self.atoms.XdndFinished => {}
            _ => {}
        }
    }

    /// Answers another app asking for our clipboard / drag data.
    fn serve(&self, req: SelectionRequestEvent) {
        let a = &self.atoms;
        let payload = if req.selection == a.CLIPBOARD {
            self.clipboard.as_ref()
        } else if req.selection == a.XdndSelection {
            self.xdnd.as_ref()
        } else {
            None
        };
        let property = if req.property == x11rb::NONE { req.target } else { req.property };
        let mut ok = false;
        if let Some(p) = payload {
            let formats = [
                (a.URI_LIST, "text/uri-list"),
                (a.GNOME_FILES, "x-special/gnome-copied-files"),
                (a.KDE_CUT, "application/x-kde-cutselection"),
                (a.UTF8_STRING, "UTF8_STRING"),
                (a.TEXT_PLAIN_UTF8, "text/plain;charset=utf-8"),
                (a.TEXT_PLAIN, "text/plain"),
                (AtomEnum::STRING.into(), "STRING"),
            ];
            if req.target == a.TARGETS {
                let mut list: Vec<u32> = vec![a.TARGETS];
                list.extend(formats.iter().map(|f| f.0));
                ok = self
                    .conn
                    .change_property32(PropMode::REPLACE, req.requestor, property, AtomEnum::ATOM, &list)
                    .is_ok();
            } else if let Some((atom, mime)) = formats.iter().find(|f| f.0 == req.target) {
                let bytes = p.bytes_for(mime);
                ok = self.conn.change_property8(PropMode::REPLACE, req.requestor, property, *atom, &bytes).is_ok();
            }
        }
        let notify = SelectionNotifyEvent {
            response_type: xproto::SELECTION_NOTIFY_EVENT,
            sequence: 0,
            time: req.time,
            requestor: req.requestor,
            selection: req.selection,
            target: req.target,
            property: if ok { property } else { x11rb::NONE },
        };
        let _ = self.conn.send_event(false, req.requestor, EventMask::NO_EVENT, notify);
    }

    /// The XDND-aware window under the pointer and its protocol version.
    fn target_at(&self) -> Option<(Window, u32)> {
        let mut w = self.root;
        for _ in 0..16 {
            let child = self.conn.query_pointer(w).ok()?.reply().ok()?.child;
            if child == x11rb::NONE {
                return None;
            }
            if child == self.own_window {
                return None;
            }
            let prop =
                self.conn.get_property(false, child, self.atoms.XdndAware, AtomEnum::ATOM, 0, 1).ok()?.reply().ok()?;
            if let Some(v) = prop.value32().and_then(|mut i| i.next()) {
                return Some((child, v.min(5)));
            }
            w = child;
        }
        None
    }

    fn track_drag(&mut self) {
        let Ok(Ok(ptr)) = self.conn.query_pointer(self.root).map(|c| c.reply()) else {
            self.drag = None;
            return;
        };
        let held = ptr.mask.contains(KeyButMask::BUTTON1);
        let target = self.target_at();
        let a_enter = self.atoms.XdndEnter;
        let a_leave = self.atoms.XdndLeave;
        let a_pos = self.atoms.XdndPosition;
        let a_drop = self.atoms.XdndDrop;
        let types = [self.atoms.URI_LIST, self.atoms.UTF8_STRING, self.atoms.TEXT_PLAIN];
        let copy = self.atoms.XdndActionCopy;
        let Some(d) = self.drag.as_mut() else { return };
        let old = d.target;
        if old.map(|t| t.0) != target.map(|t| t.0) {
            if let Some((w, _)) = old {
                let ev = ClientMessageEvent::new(32, w, a_leave, [self.win, 0, 0, 0, 0]);
                let _ = self.conn.send_event(false, w, EventMask::NO_EVENT, ev);
            }
            if let Some((w, v)) = target {
                let ev = ClientMessageEvent::new(32, w, a_enter, [self.win, v << 24, types[0], types[1], types[2]]);
                let _ = self.conn.send_event(false, w, EventMask::NO_EVENT, ev);
            }
            d.target = target;
            d.accepted = false;
            d.waiting = false;
            d.last = (i16::MIN, i16::MIN);
        }
        if let Some((w, _)) = d.target
            && !d.waiting
            && (ptr.root_x, ptr.root_y) != d.last
            && held
        {
            let xy = ((ptr.root_x as u16 as u32) << 16) | ptr.root_y as u16 as u32;
            let ev = ClientMessageEvent::new(32, w, a_pos, [self.win, 0, xy, x11rb::CURRENT_TIME, copy]);
            let _ = self.conn.send_event(false, w, EventMask::NO_EVENT, ev);
            d.waiting = true;
            d.last = (ptr.root_x, ptr.root_y);
        }
        if !held || d.started.elapsed() > Duration::from_secs(120) {
            if let Some((w, _)) = d.target {
                if d.accepted {
                    let ev = ClientMessageEvent::new(32, w, a_drop, [self.win, 0, x11rb::CURRENT_TIME, 0, 0]);
                    let _ = self.conn.send_event(false, w, EventMask::NO_EVENT, ev);
                } else {
                    let ev = ClientMessageEvent::new(32, w, a_leave, [self.win, 0, 0, 0, 0]);
                    let _ = self.conn.send_event(false, w, EventMask::NO_EVENT, ev);
                }
            }
            // Keep `xdnd` so the target can still fetch the data after the drop.
            self.drag = None;
        }
    }
}
