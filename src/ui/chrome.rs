//! Window chrome: the custom title bar with per-pane tab strips, window
//! buttons, resize edges, the split divider, toasts and the drag overlay.

use egui::{
    Color32, CornerRadius, CursorIcon, Id, LayerId, Order, Rect, ResizeDirection, Sense, Stroke, Ui, ViewportCommand,
    pos2, vec2,
};

use super::{elided, icon_button};
use crate::app::{Action, CONTROLS_W, DragPaths, FileFlier, plural};
use crate::icons;

const TAB_H: f32 = 32.0;

impl FileFlier {
    /// `strips` holds (pane index, left, right) for each visible pane column.
    pub(crate) fn title_bar(&mut self, ui: &mut Ui, rect: Rect, sidebar_w: f32, strips: &[(usize, f32, f32)]) {
        let pal = self.pal();
        let ctx = ui.ctx().clone();
        ui.painter().rect_filled(rect, 0.0, pal.titlebar);

        // Empty title-bar space drags the window; double-click toggles maximize.
        let drag = ui.interact(rect, Id::new("titlebar_drag"), Sense::click_and_drag());
        let maximized = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
        if drag.drag_started() {
            ctx.send_viewport_cmd(ViewportCommand::StartDrag);
        }
        if drag.double_clicked() {
            ctx.send_viewport_cmd(ViewportCommand::Maximized(!maximized));
        }

        // Logo + sidebar toggle.
        let logo = Rect::from_center_size(pos2(rect.left() + 22.0, rect.center().y), vec2(20.0, 20.0));
        icons::logo(ui.painter(), logo, pal);
        let logo_end = rect.left() + 44.0;
        if sidebar_w > 90.0 {
            let r = Rect::from_center_size(pos2(rect.left() + sidebar_w - 22.0, rect.center().y), vec2(28.0, 28.0));
            if icon_button(ui, r, Id::new("sb_toggle"), true, pal, "Hide sidebar (Ctrl+B)", icons::sidebar_toggle)
                .clicked()
            {
                self.actions.push(Action::Run(crate::commands::Command::ToggleSidebar));
            }
        }

        let controls_left = rect.right() - CONTROLS_W;
        for &(idx, left, right) in strips {
            let left = left.max(logo_end) + 6.0;
            let right = right.min(controls_left) - 4.0;
            if right > left + 40.0 {
                self.tab_strip(ui, idx, Rect::from_min_max(pos2(left, rect.top()), pos2(right, rect.bottom())));
            }
        }
        self.window_controls(ui, Rect::from_min_max(pos2(controls_left, rect.top()), rect.max), maximized);
    }

    fn tab_strip(&mut self, ui: &mut Ui, idx: usize, strip: Rect) {
        let pal = self.pal();
        let pane_active = idx == self.active;
        let pane = &self.panes[idx];
        let n = pane.tabs.len();
        let tab_w = ((strip.width() - 40.0) / n as f32).clamp(64.0, 220.0);
        let top = strip.bottom() - TAB_H;
        let painter = ui.painter().with_clip_rect(strip);
        let pointer = ui.input(|i| i.pointer.hover_pos());

        for (ti, tab) in pane.tabs.iter().enumerate() {
            let r = Rect::from_min_size(pos2(strip.left() + ti as f32 * tab_w, top), vec2(tab_w, TAB_H));
            if r.left() >= strip.right() - 30.0 {
                break;
            }
            let id = Id::new(("tab", idx, ti));
            let resp = ui.interact(r.intersect(strip), id, Sense::click_and_drag());
            let is_active = ti == pane.active;
            let hovered = resp.hovered();
            if is_active {
                let radius = CornerRadius { nw: 7, ne: 7, sw: 0, se: 0 };
                painter.rect_filled(r, radius, pal.bg);
                if pane_active {
                    // Thin accent on top of the active pane's current tab.
                    painter.line_segment(
                        [pos2(r.left() + 7.0, r.top() + 0.5), pos2(r.right() - 7.0, r.top() + 0.5)],
                        Stroke::new(1.0, pal.border),
                    );
                }
            } else if hovered {
                painter.rect_filled(r.shrink2(vec2(2.0, 3.0)), 6.0, pal.tab_hover);
            } else if ti + 1 < n && ti + 1 != pane.active {
                painter.line_segment(
                    [pos2(r.right(), r.top() + 9.0), pos2(r.right(), r.bottom() - 9.0)],
                    Stroke::new(1.0, pal.border),
                );
            }

            let icon = Rect::from_center_size(pos2(r.left() + 20.0, r.center().y), vec2(18.0, 18.0));
            icons::folder(&painter, icon, pal);
            if tab.locked {
                let lr = Rect::from_center_size(pos2(r.left() + 27.0, r.center().y + 5.0), vec2(10.0, 10.0));
                painter.circle_filled(lr.center(), 6.0, if is_active { pal.bg } else { pal.titlebar });
                icons::lock(&painter, lr, pal.text, true);
            }
            let show_close = n > 1 && (is_active || hovered);
            let text_right = if show_close { r.right() - 30.0 } else { r.right() - 10.0 };
            let color = if is_active { pal.text_strong } else { pal.text_dim };
            let g = elided(&painter, &tab.title(), 14.0, color, text_right - (r.left() + 36.0));
            painter.galley(pos2(r.left() + 36.0, r.center().y - g.size().y / 2.0), g, color);

            if show_close {
                let cr = Rect::from_center_size(pos2(r.right() - 17.0, r.center().y), vec2(20.0, 20.0));
                let close = ui.interact(cr, id.with("close"), Sense::click());
                if close.hovered() {
                    painter.rect_filled(cr, 4.0, pal.border);
                }
                icons::close(&painter, cr.shrink(3.0), if close.hovered() { pal.text_strong } else { pal.text_dim });
                if close.clicked() {
                    self.actions.push(Action::CloseTab(idx, ti));
                    continue;
                }
            }
            if resp.clicked() || resp.drag_started() {
                self.actions.push(Action::SelectTab(idx, ti));
            }
            if resp.middle_clicked() && n > 1 {
                self.actions.push(Action::CloseTab(idx, ti));
            }
            // Drag to reorder.
            if resp.dragged()
                && let Some(p) = pointer
            {
                let target = (((p.x - strip.left()) / tab_w).floor().max(0.0) as usize).min(n - 1);
                if target != ti {
                    self.actions.push(Action::MoveTab(idx, ti, target));
                }
            }
            resp.on_hover_text(tab.path.to_string_lossy());
        }

        // New-tab button.
        let plus_x = (strip.left() + n as f32 * tab_w + 18.0).min(strip.right() - 16.0);
        let pr = Rect::from_center_size(pos2(plus_x, top + TAB_H / 2.0), vec2(28.0, 28.0));
        if icon_button(ui, pr, Id::new(("newtab", idx)), true, pal, "New tab (Ctrl+T)", icons::plus).clicked() {
            self.actions.push(Action::NewTabIn(idx));
        }
    }

    fn window_controls(&mut self, ui: &mut Ui, rect: Rect, maximized: bool) {
        let pal = self.pal();
        let ctx = ui.ctx().clone();
        let w = rect.width() / 3.0;
        for i in 0..3 {
            let r = Rect::from_min_size(pos2(rect.left() + i as f32 * w, rect.top()), vec2(w, rect.height()));
            let resp = ui.interact(r, Id::new(("winctl", i)), Sense::click());
            let is_close = i == 2;
            if resp.hovered() {
                ui.painter().rect_filled(r, 0.0, if is_close { pal.close_hover } else { pal.tab_hover });
            }
            let color = if resp.hovered() && is_close { Color32::WHITE } else { pal.text };
            let icon = Rect::from_center_size(r.center(), vec2(16.0, 16.0));
            match i {
                0 => icons::minimize(ui.painter(), icon, color),
                1 => icons::maximize(ui.painter(), icon, color, maximized),
                _ => icons::close(ui.painter(), icon, color),
            }
            if resp.clicked() {
                ctx.send_viewport_cmd(match i {
                    0 => ViewportCommand::Minimized(true),
                    1 => ViewportCommand::Maximized(!maximized),
                    _ => ViewportCommand::Close,
                });
            }
        }
    }

    /// Without OS decorations we provide our own resize handles.
    pub(crate) fn resize_edges(&mut self, ui: &mut Ui, full: Rect) {
        let ctx = ui.ctx().clone();
        if ctx.input(|i| i.viewport().maximized.unwrap_or(false)) {
            return;
        }
        let m = 5.0;
        let c = 12.0;
        let (l, r, t, b) = (full.left(), full.right(), full.top(), full.bottom());
        let zones = [
            (
                Rect::from_min_max(pos2(l, t), pos2(l + c, t + c)),
                ResizeDirection::NorthWest,
                CursorIcon::ResizeNorthWest,
            ),
            (
                Rect::from_min_max(pos2(r - c, t), pos2(r, t + c)),
                ResizeDirection::NorthEast,
                CursorIcon::ResizeNorthEast,
            ),
            (
                Rect::from_min_max(pos2(l, b - c), pos2(l + c, b)),
                ResizeDirection::SouthWest,
                CursorIcon::ResizeSouthWest,
            ),
            (
                Rect::from_min_max(pos2(r - c, b - c), pos2(r, b)),
                ResizeDirection::SouthEast,
                CursorIcon::ResizeSouthEast,
            ),
            (Rect::from_min_max(pos2(l, t), pos2(r, t + m)), ResizeDirection::North, CursorIcon::ResizeNorth),
            (Rect::from_min_max(pos2(l, b - m), pos2(r, b)), ResizeDirection::South, CursorIcon::ResizeSouth),
            (Rect::from_min_max(pos2(l, t), pos2(l + m, b)), ResizeDirection::West, CursorIcon::ResizeWest),
            (Rect::from_min_max(pos2(r - m, t), pos2(r, b)), ResizeDirection::East, CursorIcon::ResizeEast),
        ];
        for (i, (zone, dir, cursor)) in zones.into_iter().enumerate() {
            let resp = ui.interact(zone, Id::new(("resize", i)), Sense::drag());
            if resp.hovered() || resp.dragged() {
                ctx.set_cursor_icon(cursor);
            }
            if resp.drag_started() {
                ctx.send_viewport_cmd(ViewportCommand::BeginResize(dir));
            }
        }
        // A 1px frame so the borderless window reads as a window.
        ui.painter().rect_stroke(full, 0.0, Stroke::new(1.0, self.pal().border), egui::StrokeKind::Inside);
    }

    pub(crate) fn split_divider(&mut self, ui: &mut Ui, area: Rect, x: f32) {
        let pal = self.pal();
        let line = Rect::from_min_max(pos2(x, area.top()), pos2(x + 1.0, area.bottom()));
        ui.painter().rect_filled(line, 0.0, pal.border);
        let handle = line.expand2(vec2(3.0, 0.0));
        let resp = ui.interact(handle, Id::new("split_divider"), Sense::drag());
        if resp.hovered() || resp.dragged() {
            ui.ctx().set_cursor_icon(CursorIcon::ResizeHorizontal);
        }
        if resp.dragged()
            && let Some(p) = resp.interact_pointer_pos()
        {
            self.cfg.split_ratio = ((p.x - area.left()) / area.width()).clamp(0.2, 0.8);
        }
        if resp.drag_stopped() {
            self.cfg.save();
        }
    }

    /// Transient notifications and background-job progress.
    pub(crate) fn toasts(&mut self, ui: &mut Ui, full: Rect) {
        let pal = self.pal();
        let (msg, err, spinner) = if let Some(job) = &self.job {
            let p = job.progress.lock().map(|p| p.clone()).unwrap_or_default();
            (format!("{} · {p}", job.label), false, true)
        } else if let Some((msg, at, err)) = &self.status {
            if at.elapsed().as_secs_f32() > 4.0 {
                return;
            }
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(250));
            (msg.clone(), *err, false)
        } else {
            return;
        };
        let painter = ui.ctx().layer_painter(LayerId::new(Order::Foreground, Id::new("toasts")));
        let color = if err { Color32::from_rgb(255, 140, 130) } else { pal.text };
        let g = elided(&painter, &msg, 13.5, color, full.width() * 0.6);
        let extra = if spinner { 26.0 } else { 0.0 };
        let size = g.size() + vec2(28.0 + extra, 16.0);
        let r = Rect::from_center_size(pos2(full.center().x, full.bottom() - 62.0), size);
        painter.rect_filled(r, 6.0, pal.popup);
        painter.rect_stroke(
            r,
            6.0,
            Stroke::new(1.0, if err { Color32::from_rgb(170, 60, 50) } else { pal.border }),
            egui::StrokeKind::Inside,
        );
        if spinner {
            let t = ui.input(|i| i.time) as f32;
            let c = pos2(r.left() + 20.0, r.center().y);
            let pts: Vec<_> = (0..=16)
                .map(|k| {
                    let a = t * 6.0 + k as f32 / 16.0 * 4.5;
                    c + vec2(a.cos(), a.sin()) * 6.0
                })
                .collect();
            painter.add(egui::Shape::line(pts, Stroke::new(2.0, pal.accent)));
            ui.ctx().request_repaint();
        }
        painter.galley(pos2(r.left() + 14.0 + extra, r.center().y - g.size().y / 2.0), g, color);
    }

    /// A small label following the pointer while files are being dragged.
    pub(crate) fn drag_overlay(&mut self, ctx: &egui::Context) {
        let Some(payload) = egui::DragAndDrop::payload::<DragPaths>(ctx) else { return };
        let Some(pos) = ctx.pointer_hover_pos() else { return };
        let pal = self.pal();
        let painter = ctx.layer_painter(LayerId::new(Order::Tooltip, Id::new("dnd")));
        let copy = ctx.input(|i| i.modifiers.command);
        let n = payload.0.len();
        let label = format!("{} {n} item{}", if copy { "Copy" } else { "Move" }, plural(n));
        let g = painter.layout_no_wrap(label, super::font(13.0), pal.on_accent);
        let at = pos + vec2(18.0, 14.0);
        painter.rect_filled(Rect::from_min_size(at, g.size()).expand2(vec2(8.0, 4.0)), 4.0, pal.accent);
        painter.galley(at, g, pal.on_accent);
    }
}
