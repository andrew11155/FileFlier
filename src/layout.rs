//! How panes are tiled in the window: a tree of horizontal and vertical splits,
//! with a pane index at each leaf. Dragging a tab onto the edge of a pane
//! splits that pane; closing a pane's last tab removes it from the tree.

use egui::{Rect, pos2};

/// Which edge of a pane a dragged tab lands on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Left,
    Right,
    Top,
    Bottom,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum Layout {
    Pane(usize),
    /// `row`: side by side (a left of b); otherwise stacked (a above b).
    /// `ratio` is the share of the space given to `a`.
    Split {
        row: bool,
        ratio: f32,
        a: Box<Layout>,
        b: Box<Layout>,
    },
}

/// A draggable divider between the two halves of a split.
pub struct Divider {
    /// Path of the split in the tree (false = a, true = b at each level).
    pub path: Vec<bool>,
    pub row: bool,
    /// The area the split divides (for turning the pointer into a ratio).
    pub area: Rect,
    /// Where the divider line is (x for rows, y for columns).
    pub at: f32,
}

pub const MIN_RATIO: f32 = 0.1;

impl Default for Layout {
    fn default() -> Self {
        Layout::Pane(0)
    }
}

impl Layout {
    pub fn two(ratio: f32) -> Self {
        Layout::Split { row: true, ratio, a: Box::new(Layout::Pane(0)), b: Box::new(Layout::Pane(1)) }
    }

    /// Pane indices in reading order (left to right, top to bottom).
    pub fn panes(&self) -> Vec<usize> {
        let mut out = Vec::new();
        self.collect(&mut out);
        out
    }

    fn collect(&self, out: &mut Vec<usize>) {
        match self {
            Layout::Pane(i) => out.push(*i),
            Layout::Split { a, b, .. } => {
                a.collect(out);
                b.collect(out);
            }
        }
    }

    pub fn count(&self) -> usize {
        match self {
            Layout::Pane(_) => 1,
            Layout::Split { a, b, .. } => a.count() + b.count(),
        }
    }

    /// Check that the leaves are exactly 0..n, each once (for a restored layout).
    pub fn is_valid(&self, n: usize) -> bool {
        let mut p = self.panes();
        p.sort_unstable();
        p == (0..n).collect::<Vec<_>>()
    }

    /// Split the leaf holding `target`, putting `new` on `side` of it.
    pub fn split(&mut self, target: usize, new: usize, side: Side) -> bool {
        match self {
            Layout::Pane(i) if *i == target => {
                let old = Box::new(Layout::Pane(target));
                let add = Box::new(Layout::Pane(new));
                let row = matches!(side, Side::Left | Side::Right);
                let (a, b) = if matches!(side, Side::Left | Side::Top) { (add, old) } else { (old, add) };
                *self = Layout::Split { row, ratio: 0.5, a, b };
                true
            }
            Layout::Pane(_) => false,
            Layout::Split { a, b, .. } => a.split(target, new, side) || b.split(target, new, side),
        }
    }

    /// Remove the leaf holding `pane` (its sibling takes the space) and shift
    /// higher pane indices down by one, matching `Vec::remove` on the panes.
    pub fn remove(&mut self, pane: usize) {
        self.remove_leaf(pane);
        self.renumber(pane);
    }

    fn remove_leaf(&mut self, pane: usize) -> bool {
        let Layout::Split { a, b, .. } = self else { return false };
        if **a == Layout::Pane(pane) {
            *self = std::mem::take(&mut **b);
            return true;
        }
        if **b == Layout::Pane(pane) {
            *self = std::mem::take(&mut **a);
            return true;
        }
        a.remove_leaf(pane) || b.remove_leaf(pane)
    }

    fn renumber(&mut self, removed: usize) {
        match self {
            Layout::Pane(i) => {
                if *i > removed {
                    *i -= 1;
                }
            }
            Layout::Split { a, b, .. } => {
                a.renumber(removed);
                b.renumber(removed);
            }
        }
    }

    /// Lay the tree out in `area`, leaving `gap` between neighbours. Returns the
    /// rectangle of every pane (indexed by pane) and the dividers.
    pub fn rects(&self, area: Rect, gap: f32, n: usize) -> (Vec<Rect>, Vec<Divider>) {
        let mut rects = vec![Rect::NOTHING; n];
        let mut dividers = Vec::new();
        self.place(area, gap, &mut Vec::new(), &mut rects, &mut dividers);
        (rects, dividers)
    }

    fn place(&self, area: Rect, gap: f32, path: &mut Vec<bool>, rects: &mut [Rect], dividers: &mut Vec<Divider>) {
        match self {
            Layout::Pane(i) => {
                if let Some(r) = rects.get_mut(*i) {
                    *r = area;
                }
            }
            Layout::Split { row, ratio, a, b } => {
                let ratio = ratio.clamp(MIN_RATIO, 1.0 - MIN_RATIO);
                let (ra, rb, at) = if *row {
                    let x = area.left() + area.width() * ratio;
                    (
                        Rect::from_min_max(area.min, pos2(x - gap / 2.0, area.bottom())),
                        Rect::from_min_max(pos2(x + gap / 2.0, area.top()), area.max),
                        x,
                    )
                } else {
                    let y = area.top() + area.height() * ratio;
                    (
                        Rect::from_min_max(area.min, pos2(area.right(), y - gap / 2.0)),
                        Rect::from_min_max(pos2(area.left(), y + gap / 2.0), area.max),
                        y,
                    )
                };
                dividers.push(Divider { path: path.clone(), row: *row, area, at });
                path.push(false);
                a.place(ra, gap, path, rects, dividers);
                path.pop();
                path.push(true);
                b.place(rb, gap, path, rects, dividers);
                path.pop();
            }
        }
    }

    /// The ratio of the split at `path`, for dragging its divider.
    pub fn ratio_mut(&mut self, path: &[bool]) -> Option<&mut f32> {
        match (self, path.split_first()) {
            (Layout::Split { ratio, .. }, None) => Some(ratio),
            (Layout::Split { a, b, .. }, Some((&side, rest))) => {
                if side {
                    b.ratio_mut(rest)
                } else {
                    a.ratio_mut(rest)
                }
            }
            (Layout::Pane(_), _) => None,
        }
    }
}

/// Which drop zone of a pane `p` is in when a tab is dragged over it: one of
/// the four edges (split there), or `None` for the middle (move the tab in).
pub fn zone(rect: Rect, p: egui::Pos2) -> Option<Side> {
    let fx = (p.x - rect.left()) / rect.width();
    let fy = (p.y - rect.top()) / rect.height();
    // Distance to each edge as a share of the pane; the nearest edge wins if
    // it's within the outer quarter.
    let edges = [(fx, Side::Left), (1.0 - fx, Side::Right), (fy, Side::Top), (1.0 - fy, Side::Bottom)];
    let (d, side) = edges.into_iter().min_by(|a, b| a.0.total_cmp(&b.0))?;
    (d < 0.25).then_some(side)
}

/// The area a dragged tab would take up if dropped on `side` (or the whole pane).
pub fn preview(rect: Rect, side: Option<Side>) -> Rect {
    let (l, r, t, b) = (rect.left(), rect.right(), rect.top(), rect.bottom());
    let (cx, cy) = (rect.center().x, rect.center().y);
    match side {
        None => rect,
        Some(Side::Left) => Rect::from_min_max(pos2(l, t), pos2(cx, b)),
        Some(Side::Right) => Rect::from_min_max(pos2(cx, t), pos2(r, b)),
        Some(Side::Top) => Rect::from_min_max(pos2(l, t), pos2(r, cy)),
        Some(Side::Bottom) => Rect::from_min_max(pos2(l, cy), pos2(r, b)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_and_remove() {
        let mut l = Layout::Pane(0);
        assert!(l.split(0, 1, Side::Right));
        assert!(l.split(1, 2, Side::Bottom));
        assert!(l.split(0, 3, Side::Top));
        assert_eq!(l.panes(), vec![3, 0, 1, 2]);
        assert!(l.is_valid(4));
        l.remove(1);
        // 2 took pane 1's place and was renumbered to 1; 3 became 2.
        assert_eq!(l.panes(), vec![2, 0, 1]);
        assert!(l.is_valid(3));
        l.remove(2);
        l.remove(0);
        assert_eq!(l, Layout::Pane(0));
    }

    #[test]
    fn rects_cover_area() {
        let mut l = Layout::two(0.5);
        l.split(1, 2, Side::Bottom);
        let area = Rect::from_min_max(pos2(0.0, 0.0), pos2(100.0, 100.0));
        let (r, d) = l.rects(area, 0.0, 3);
        assert_eq!(r[0], Rect::from_min_max(pos2(0.0, 0.0), pos2(50.0, 100.0)));
        assert_eq!(r[1], Rect::from_min_max(pos2(50.0, 0.0), pos2(100.0, 50.0)));
        assert_eq!(r[2], Rect::from_min_max(pos2(50.0, 50.0), pos2(100.0, 100.0)));
        assert_eq!(d.len(), 2);
        *l.ratio_mut(&d[1].path).unwrap() = 0.25;
        let (r, _) = l.rects(area, 0.0, 3);
        assert_eq!(r[1].bottom(), 25.0);
    }

    #[test]
    fn drop_zones() {
        let r = Rect::from_min_max(pos2(0.0, 0.0), pos2(100.0, 100.0));
        assert_eq!(zone(r, pos2(5.0, 50.0)), Some(Side::Left));
        assert_eq!(zone(r, pos2(50.0, 95.0)), Some(Side::Bottom));
        assert_eq!(zone(r, pos2(50.0, 50.0)), None);
        assert!(!Layout::two(0.5).is_valid(3));
    }
}
