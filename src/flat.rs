//! Flat view: every file in a folder and all of its subfolders, as one list.
//! The tree is walked on a background thread; files arrive in batches.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};

use crate::fs_model::Entry;

/// Stop after this many files (a whole disk would take minutes and gigabytes).
pub const MAX_FILES: usize = 200_000;

enum Msg {
    Files(Vec<Entry>),
    Done { truncated: bool },
}

pub struct Scan {
    cancel: Arc<AtomicBool>,
    rx: Receiver<Msg>,
    pub done: bool,
    /// The scan stopped at `MAX_FILES`.
    pub truncated: bool,
}

impl Scan {
    pub fn start(root: PathBuf, include_hidden: bool, ctx: Option<egui::Context>) -> Self {
        let cancel = Arc::new(AtomicBool::new(false));
        let (tx, rx) = channel();
        let c = cancel.clone();
        std::thread::spawn(move || {
            let mut w = Walk { include_hidden, cancel: c, tx, batch: Vec::new(), sent: Instant::now(), found: 0, ctx };
            let truncated = w.walk(&root);
            w.flush();
            let _ = w.tx.send(Msg::Done { truncated });
            if let Some(ctx) = &w.ctx {
                ctx.request_repaint();
            }
        });
        Self { cancel, rx, done: false, truncated: false }
    }

    /// Files found since the last call (empty if none).
    pub fn poll(&mut self) -> Vec<Entry> {
        let mut out = Vec::new();
        while let Ok(msg) = self.rx.try_recv() {
            match msg {
                Msg::Files(f) => out.extend(f),
                Msg::Done { truncated } => {
                    self.done = true;
                    self.truncated = truncated;
                }
            }
        }
        out
    }
}

impl Drop for Scan {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

struct Walk {
    include_hidden: bool,
    cancel: Arc<AtomicBool>,
    tx: Sender<Msg>,
    batch: Vec<Entry>,
    sent: Instant,
    found: usize,
    ctx: Option<egui::Context>,
}

impl Walk {
    /// Returns true if it stopped at the file limit.
    fn walk(&mut self, root: &Path) -> bool {
        // Iterative so deep trees can't overflow the stack.
        let mut stack = vec![root.to_path_buf()];
        while let Some(dir) = stack.pop() {
            let Ok(rd) = std::fs::read_dir(&dir) else { continue };
            let mut subdirs = Vec::new();
            for item in rd.flatten() {
                if self.cancel.load(Ordering::Relaxed) {
                    return false;
                }
                if self.found >= MAX_FILES {
                    return true;
                }
                if !self.include_hidden && item.file_name().to_string_lossy().starts_with('.') {
                    continue;
                }
                let Ok(ft) = item.file_type() else { continue };
                if ft.is_dir() {
                    subdirs.push(item.path()); // symlinked folders aren't followed: no loops
                    continue;
                }
                // Files, and symlinks that point at files (not at folders).
                if let Some(e) = Entry::from_path(&item.path()).filter(|e| !e.is_dir) {
                    self.found += 1;
                    self.batch.push(e);
                }
            }
            // Pop folders in name order so results arrive roughly top to bottom.
            subdirs.sort_unstable_by(|a, b| b.cmp(a));
            stack.extend(subdirs);
            if self.batch.len() >= 4096 || (!self.batch.is_empty() && self.sent.elapsed() > Duration::from_millis(120))
            {
                self.flush();
            }
        }
        false
    }

    fn flush(&mut self) {
        if !self.batch.is_empty() {
            let _ = self.tx.send(Msg::Files(std::mem::take(&mut self.batch)));
            if let Some(ctx) = &self.ctx {
                ctx.request_repaint();
            }
        }
        self.sent = Instant::now();
    }
}
