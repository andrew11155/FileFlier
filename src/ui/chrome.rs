//! Window chrome: the custom title bar with per-pane tab strips, window
//! buttons, resize edges, the split divider, toasts and the drag overlay.

use egui::{
    Color32, CornerRadius, CursorIcon, Id, LayerId, Order, Rect, ResizeDirection, Sense, Stroke, Ui, ViewportCommand,
    pos2, vec2,
};

use super::{elided, icon_button};
use crate::app::{Action, CONTROLS_W, DragPaths, FileFlier, TRAFFIC_LIGHTS_W, plural};
use crate::icons;

const TAB_H: f32 = 32.0;

impl FileFlier {
    /// `strips` holds (pane index, left, right) for each visible pane column.
    pub(crate) fn title_bar(&mut self, ui: &mut Ui, rect: Rect, sidebar_w: f32, strips: &[(usize, f32, f32)]) {
        let pal = self.pal();
        let ctx = ui.ctx().clone();
        if !self.fx().glass {
            ui.painter().rect_filled(rect, 0.0, pal.titlebar); // glass: tabs float on the backdrop
        }

        // Empty title-bar space drags the window; double-click toggles maximize.
        let drag = ui.interact(rect, Id::new("titlebar_drag"), Sense::click_and_drag());
        let maximized = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
        if drag.drag_started() {
            ctx.send_viewport_cmd(ViewportCommand::StartDrag);
            self.release_after_grab = true;
        }
        if drag.double_clicked() {
            ctx.send_viewport_cmd(ViewportCommand::Maximized(!maximized));
        }

        // Logo + sidebar toggle. On macOS the native window buttons sit where the logo would.
        let logo_end = if cfg!(target_os = "macos") {
            rect.left() + TRAFFIC_LIGHTS_W
        } else {
            let logo = Rect::from_center_size(pos2(rect.left() + 22.0, rect.center().y), vec2(20.0, 20.0));
            icons::logo(ui.painter(), logo, pal);
            rect.left() + 44.0
        };
        if sidebar_w > 90.0 {
            let r = Rect::from_center_size(pos2(rect.left() + sidebar_w - 22.0, rect.center().y), vec2(28.0, 28.0));
            if icon_button(ui, r, Id::new("sb_toggle"), true, pal, "Hide sidebar (Ctrl+B)", icons::sidebar_toggle)
                .clicked()
            {
                self.actions.push(Action::Run(crate::commands::Command::ToggleSidebar));
            }
        }

        let controls_left = rect.right() - CONTROLS_W;
        let tabs_right = self.update_pill(ui, rect, controls_left).unwrap_or(controls_left);
        for &(idx, left, right) in strips {
            let left = left.max(logo_end) + 6.0;
            let right = right.min(tabs_right) - 4.0;
            if right > left + 40.0 {
                self.tab_strip(ui, idx, Rect::from_min_max(pos2(left, rect.top()), pos2(right, rect.bottom())));
            }
        }
        if CONTROLS_W > 0.0 {
            self.window_controls(ui, Rect::from_min_max(pos2(controls_left, rect.top()), rect.max), maximized);
        }
    }

    /// The accent "Update available" pill left of the window controls. Returns its left
    /// edge so the tab strips can stop short of it.
    fn update_pill(&mut self, ui: &mut Ui, rect: Rect, controls_left: f32) -> Option<f32> {
        if !crate::updater::ENABLED {
            return None;
        }
        let label = if self.updater.restart_pending() {
            "Restart to update"
        } else if self.updater.offered(&self.cfg).is_some() {
            "Update available"
        } else {
            return None;
        };
        let pal = self.pal();
        let w = ui.painter().layout_no_wrap(label.to_string(), super::bold(12.5), pal.on_accent).size().x + 24.0;
        let r = Rect::from_min_size(pos2(controls_left - 8.0 - w, rect.center().y - 12.0), vec2(w, 24.0));
        let resp = ui.interact(r, Id::new("update_pill"), Sense::click());
        let fill = if resp.hovered() { pal.accent.gamma_multiply(1.2) } else { pal.accent };
        ui.painter().rect_filled(r, 12.0, fill);
        super::text_bold(ui.painter(), r.center(), egui::Align2::CENTER_CENTER, label, 12.5, pal.on_accent);
        if resp.on_hover_cursor(CursorIcon::PointingHand).clicked() {
            self.dialog = Some(crate::app::Dialog::Update);
        }
        Some(r.left())
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
                let fx = self.fx();
                let fill = if fx.glass { crate::ui::glass::with_alpha(pal.bg, fx.opacity) } else { pal.bg };
                painter.rect_filled(r, radius, fill);
                if pane_active {
                    // Thin accent on top of the active pane's current tab.
                    painter.rect_filled(
                        Rect::from_min_max(pos2(r.left() + 10.0, r.top()), pos2(r.right() - 10.0, r.top() + 2.0)),
                        1.0,
                        pal.accent,
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
                self.release_after_grab = true;
            }
        }
        // A 1px frame so the borderless window reads as a window.
        if !self.fx().see_through {
            ui.painter().rect_stroke(full, 0.0, Stroke::new(1.0, self.pal().border), egui::StrokeKind::Inside);
        }
    }

    pub(crate) fn split_divider(&mut self, ui: &mut Ui, area: Rect, x: f32, draw_line: bool) {
        let pal = self.pal();
        let line = Rect::from_min_max(pos2(x, area.top()), pos2(x + 1.0, area.bottom()));
        if draw_line {
            ui.painter().rect_filled(line, 0.0, pal.border);
        }
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
        let fx = self.fx();
        let ctx = ui.ctx().clone();
        // Undoable toasts stay up longer so there's time to click Undo.
        let life = if self.toast_undo { 7.0 } else { 4.0 };
        let (msg, err, spinner, key, fade_out) = if let Some(job) = &self.job {
            let p = job.progress.lock().map(|p| p.clone()).unwrap_or_default();
            (format!("{} · {p}", job.label), false, true, format!("job{}", job.label), 1.0)
        } else if let Some((msg, at, err)) = &self.status {
            let age = at.elapsed().as_secs_f32();
            if age > life {
                return;
            }
            ctx.request_repaint_after(std::time::Duration::from_millis(50));
            let out = if fx.animations { ((life - age) / 0.3).clamp(0.0, 1.0) } else { 1.0 };
            (msg.clone(), *err, false, format!("{at:?}"), out)
        } else {
            return;
        };
        let show_undo = self.toast_undo && self.last_undo.is_some() && self.job.is_none();
        let t_in = super::appear(&ctx, Id::new(("toast", key)), fx.dur(0.22));
        let mut undo_clicked = false;
        egui::Area::new(Id::new("toast_area"))
            .order(Order::Foreground)
            .pivot(egui::Align2::CENTER_BOTTOM)
            .fixed_pos(pos2(full.center().x, full.bottom() - 48.0 + (1.0 - t_in) * 18.0))
            .show(&ctx, |ui| {
                ui.multiply_opacity(t_in * fade_out);
                let color = if err { Color32::from_rgb(255, 140, 130) } else { pal.text };
                let border = if err { Color32::from_rgb(170, 60, 50) } else { pal.border };
                super::popup_frame(pal, &fx)
                    .stroke(Stroke::new(1.0, border))
                    .inner_margin(egui::Margin::symmetric(12, 8))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 10.0;
                            if spinner {
                                ui.add(egui::Spinner::new().size(14.0).color(pal.accent));
                            }
                            let g = elided(ui.painter(), &msg, 13.5, color, full.width() * 0.55);
                            let (r, _) = ui.allocate_exact_size(g.size(), egui::Sense::hover());
                            ui.painter().galley(r.min, g, color);
                            if show_undo {
                                let label = egui::RichText::new("Undo").size(13.5).color(pal.accent).strong();
                                let b =
                                    ui.add(egui::Button::new(label).frame(false)).on_hover_text(super::keys("Ctrl+Z"));
                                undo_clicked = b.clicked();
                            }
                        });
                    });
            });
        if undo_clicked {
            self.undo_last();
        }
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
