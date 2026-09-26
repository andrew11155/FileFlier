//! Background recursive search.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};

use crate::fuzzy;

pub const MAX_RESULTS: usize = 2000;

pub struct Hit {
    pub path: PathBuf,
    pub is_dir: bool,
    pub score: i32,
}

pub enum Msg {
    Hits(Vec<Hit>),
    Done { scanned: usize },
}

pub struct Search {
    cancel: Arc<AtomicBool>,
    rx: Receiver<Msg>,
    pub results: Vec<Hit>,
    pub scanned: usize,
    pub done: bool,
}

impl Search {
    pub fn start(root: PathBuf, query: String, include_hidden: bool, ctx: egui::Context) -> Self {
        let cancel = Arc::new(AtomicBool::new(false));
        let (tx, rx) = channel();
        let c = cancel.clone();
        std::thread::spawn(move || {
            let mut state = Walk { query, include_hidden, cancel: c, tx, batch: Vec::new(), found: 0, scanned: 0, ctx };
            state.walk(&root);
            state.flush();
            let _ = state.tx.send(Msg::Done { scanned: state.scanned });
            state.ctx.request_repaint();
        });
        Self { cancel, rx, results: Vec::new(), scanned: 0, done: false }
    }

    /// Pulls pending results from the worker. Returns true if anything changed.
    pub fn poll(&mut self) -> bool {
        let mut changed = false;
        while let Ok(msg) = self.rx.try_recv() {
            changed = true;
            match msg {
                Msg::Hits(h) => self.results.extend(h),
                Msg::Done { scanned } => {
                    self.scanned = scanned;
                    self.done = true;
                }
            }
        }
        if changed {
            self.results.sort_by_key(|h| std::cmp::Reverse(h.score));
        }
        changed
    }
}

impl Drop for Search {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

struct Walk {
    query: String,
    include_hidden: bool,
    cancel: Arc<AtomicBool>,
    tx: Sender<Msg>,
    batch: Vec<Hit>,
    found: usize,
    scanned: usize,
    ctx: egui::Context,
}

impl Walk {
    fn walk(&mut self, dir: &Path) {
        // Iterative DFS so deep trees can't blow the stack.
        let mut stack = vec![dir.to_path_buf()];
        while let Some(dir) = stack.pop() {
            let Ok(rd) = std::fs::read_dir(&dir) else {
                continue;
            };
            for item in rd.flatten() {
                if self.cancel.load(Ordering::Relaxed) || self.found >= MAX_RESULTS {
                    return;
                }
                self.scanned += 1;
                let name = item.file_name().to_string_lossy().into_owned();
                if !self.include_hidden && name.starts_with('.') {
                    continue;
                }
                let Ok(ft) = item.file_type() else { continue };
                let is_dir = ft.is_dir(); // don't follow symlinked dirs: avoids cycles
                if let Some(score) = fuzzy::score(&self.query, &name) {
                    self.found += 1;
                    self.batch.push(Hit { path: item.path(), is_dir, score });
                    if self.batch.len() >= 64 {
                        self.flush();
                    }
                }
                if is_dir {
                    stack.push(item.path());
                }
            }
        }
    }

    fn flush(&mut self) {
        if !self.batch.is_empty() {
            let _ = self.tx.send(Msg::Hits(std::mem::take(&mut self.batch)));
            self.ctx.request_repaint();
        }
    }
}
