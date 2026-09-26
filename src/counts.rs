//! Folder item counts for the details view, computed off the UI thread so a slow
//! or unreachable network folder can't freeze the window.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, channel};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Count {
    Pending,
    Items(usize),
    Unknown,
}

pub struct ItemCounter {
    map: HashMap<PathBuf, Count>,
    requests: Sender<PathBuf>,
    results: Receiver<(PathBuf, Option<usize>)>,
}

impl ItemCounter {
    pub fn start(ctx: egui::Context) -> Self {
        let (req_tx, req_rx) = channel::<PathBuf>();
        let (res_tx, res_rx) = channel();
        std::thread::Builder::new()
            .name("item-counter".into())
            .spawn(move || {
                for path in req_rx {
                    let n = std::fs::read_dir(&path).ok().map(|d| d.count());
                    if res_tx.send((path, n)).is_err() {
                        break;
                    }
                    ctx.request_repaint();
                }
            })
            .expect("spawn item counter");
        Self { map: HashMap::new(), requests: req_tx, results: res_rx }
    }

    /// Returns the cached count, queueing a background count on first sight.
    pub fn get(&mut self, path: &Path) -> Count {
        if let Some(c) = self.map.get(path) {
            return *c;
        }
        self.map.insert(path.to_path_buf(), Count::Pending);
        let _ = self.requests.send(path.to_path_buf());
        Count::Pending
    }

    pub fn poll(&mut self) {
        while let Ok((path, n)) = self.results.try_recv() {
            self.map.insert(path, n.map_or(Count::Unknown, Count::Items));
        }
    }

    pub fn clear(&mut self) {
        self.map.clear();
    }
}
