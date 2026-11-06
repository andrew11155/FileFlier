//! The preview panel (inspector), Finder style: a large zoomable preview (videos and
//! music play in it), the item's name, rating and tags, and an Information list.
//! It docks on any side of the file panes: a column on the left or right (header,
//! preview, details, and Open / Quick Look in a footer level with the panes' bottom
//! bars), or a band across the bottom or top (the preview beside the details).

use egui::{Align2, Color32, CursorIcon, Id, Rect, Sense, Stroke, StrokeKind, Ui, UiBuilder, pos2, vec2};

use super::preview_view::{self, Nav, Style};
use super::{elided, text};
use crate::app::{FileFlier, format_mode, format_time, human_size};
use crate::config::PanelSide;
use crate::fs_model::Entry;
use crate::preview::Content;
use crate::ui::pane_view::type_label;

/// Level with the panes' toolbars and bottom bars.
const HEAD_H: f32 = 42.0;
const FOOT_H: f32 = 40.0;

impl FileFlier {
    /// Asks the previewer for the item under the cursor (and the current page).
    pub(crate) fn request_preview(&mut self) -> Option<crate::fs_model::Entry> {
        let e = self.tab().cursor_entry().cloned()?;
        if self.preview_page.0 != e.path {
            self.preview_page = (e.path.clone(), 0);
        }
        let opts = crate::preview::Options {
            office_pages: self.cfg.office_previews,
            remote: crate::mounts::is_remote_path(&e.path, &self.mounts),
        };
        self.pv.want(&e, self.preview_page.1, opts);
        Some(e)
    }

    pub(crate) fn inspector_ui(&mut self, ui: &mut Ui, rect: Rect) {
        let pal = self.pal();
        let fx = self.fx();
        let side = self.cfg.inspector_side;
        super::surface(ui.painter(), rect, pal.sidebar, &fx, pal);
        self.inspector_resize(ui, rect, side);
        self.inspector_body(ui, rect, side);
        // The line between the panel and the panes, on top of everything in it.
        if !fx.glass {
            let s = Stroke::new(1.0, pal.border);
            let p = ui.painter();
            match side {
                PanelSide::Right => p.vline(rect.left() + 0.5, rect.y_range(), s),
                PanelSide::Left => p.vline(rect.right() - 0.5, rect.y_range(), s),
                PanelSide::Bottom => p.hline(rect.x_range(), rect.top() + 0.5, s),
                PanelSide::Top => p.hline(rect.x_range(), rect.bottom() - 0.5, s),
            };
        }
    }

    fn inspector_body(&mut self, ui: &mut Ui, rect: Rect, side: PanelSide) {
        let pal = self.pal();
        // One player at a time: Quick Look has its own.
        if self.quicklook {
            self.insp_player = None;
        }
        let e = self.request_preview();
        if e.as_ref().is_none_or(|e| self.insp_player.as_ref().is_some_and(|p| p.path != e.path)) {
            self.insp_player = None;
        }

        if side.is_column() {
            let head = Rect::from_min_max(rect.min, pos2(rect.right(), rect.top() + HEAD_H));
            let foot = Rect::from_min_max(pos2(rect.left(), rect.bottom() - FOOT_H), rect.max);
            let body = Rect::from_min_max(pos2(rect.left(), head.bottom()), pos2(rect.right(), foot.top()));
            self.inspector_head(ui, head, e.as_ref(), false);
            let Some(e) = e else {
                text(ui.painter(), body.center(), Align2::CENTER_CENTER, "Nothing selected", 14.0, pal.text_dim);
                return;
            };
            // Taller for things you read (text, documents, tables).
            let reading = self.reading(&e);
            let preview_h = if reading {
                (body.height() * 0.55).max(160.0)
            } else {
                body.width().min(body.height() * 0.5).max(140.0)
            };
            let pr = Rect::from_min_size(body.min, vec2(body.width(), preview_h.min(body.height())));
            self.inspector_preview(ui, pr, &e);
            let p = ui.painter();
            p.hline(pr.x_range(), pr.top() + 0.5, Stroke::new(1.0, pal.row_sep));
            p.hline(pr.x_range(), pr.bottom() - 0.5, Stroke::new(1.0, pal.row_sep));
            let details = Rect::from_min_max(pos2(body.left(), pr.bottom()), body.max);
            self.inspector_details(ui, details, &e, true);
            self.inspector_foot(ui, foot);
        } else {
            let Some(e) = e else {
                self.inspector_head(
                    ui,
                    Rect::from_min_max(rect.min, pos2(rect.right(), rect.top() + HEAD_H)),
                    None,
                    true,
                );
                text(ui.painter(), rect.center(), Align2::CENTER_CENTER, "Nothing selected", 14.0, pal.text_dim);
                return;
            };
            // The preview on the left, about as wide as a photo is for this height.
            let pw = (rect.height() * 1.6).min(rect.width() * 0.45).max(160.0);
            let pr = Rect::from_min_max(rect.min, pos2(rect.left() + pw, rect.bottom()));
            self.inspector_preview(ui, pr, &e);
            ui.painter().vline(pr.right() - 0.5, pr.y_range(), Stroke::new(1.0, pal.row_sep));
            let right = Rect::from_min_max(pos2(pr.right(), rect.top()), rect.max);
            let head = Rect::from_min_max(right.min, pos2(right.right(), right.top() + HEAD_H));
            self.inspector_head(ui, head, Some(&e), true);
            let details = Rect::from_min_max(pos2(right.left(), head.bottom()), right.max);
            self.inspector_details(ui, details, &e, false);
        }
    }

    fn reading(&mut self, e: &Entry) -> bool {
        self.pv.shown_for(&e.path).is_some_and(|s| {
            matches!(
                s.loaded.content,
                Content::Text { .. }
                    | Content::Document { .. }
                    | Content::Table { .. }
                    | Content::Dir(_)
                    | Content::Listing(_)
                    | Content::Page { .. }
                    | Content::Model { .. }
            )
        })
    }

    /// Dragging the inner edge resizes the panel.
    fn inspector_resize(&mut self, ui: &mut Ui, rect: Rect, side: PanelSide) {
        let handle = match side {
            PanelSide::Right => Rect::from_x_y_ranges(rect.left() - 3.0..=rect.left() + 3.0, rect.y_range()),
            PanelSide::Left => Rect::from_x_y_ranges(rect.right() - 3.0..=rect.right() + 3.0, rect.y_range()),
            PanelSide::Bottom => Rect::from_x_y_ranges(rect.x_range(), rect.top() - 3.0..=rect.top() + 3.0),
            PanelSide::Top => Rect::from_x_y_ranges(rect.x_range(), rect.bottom() - 3.0..=rect.bottom() + 3.0),
        };
        let h = ui.interact(handle, Id::new("inspector_resize"), Sense::drag());
        if h.hovered() || h.dragged() {
            ui.ctx().set_cursor_icon(if side.is_column() {
                CursorIcon::ResizeHorizontal
            } else {
                CursorIcon::ResizeVertical
            });
        }
        if h.dragged() {
            let d = h.drag_delta();
            match side {
                PanelSide::Right => self.cfg.inspector_width -= d.x,
                PanelSide::Left => self.cfg.inspector_width += d.x,
                PanelSide::Bottom => self.cfg.inspector_height -= d.y,
                PanelSide::Top => self.cfg.inspector_height += d.y,
            }
            self.cfg.inspector_width = self.cfg.inspector_width.clamp(240.0, 900.0);
            self.cfg.inspector_height = self.cfg.inspector_height.clamp(150.0, 800.0);
        }
        if h.drag_stopped() {
            self.cfg.save();
        }
    }

    /// The header: the kind (or, in a band, the name with Open and Quick Look), then
    /// buttons to move and close the panel.
    fn inspector_head(&mut self, ui: &mut Ui, r: Rect, e: Option<&Entry>, band: bool) {
        let pal = self.pal();
        let cy = r.center().y;
        let close_r = Rect::from_center_size(pos2(r.right() - 22.0, cy), vec2(28.0, 28.0));
        let side_r = Rect::from_center_size(pos2(close_r.left() - 16.0, cy), vec2(28.0, 28.0));
        if super::icon_button(
            ui,
            close_r,
            Id::new("insp_close"),
            true,
            pal,
            "Hide the preview panel (F3)",
            |p, ir, c| crate::icons::close(p, ir.shrink(3.0), c),
        )
        .clicked()
        {
            self.cfg.show_preview = false;
            self.cfg.save();
        }
        let side = self.cfg.inspector_side;
        if super::icon_button(ui, side_r, Id::new("insp_side"), true, pal, "Move the preview panel", |p, ir, c| {
            panel_glyph(p, ir, c, side)
        })
        .clicked()
        {
            use crate::app::MenuItem;
            use crate::commands::Command;
            let items = vec![
                MenuItem::Choice(Command::PreviewRight, side == PanelSide::Right),
                MenuItem::Choice(Command::PreviewLeft, side == PanelSide::Left),
                MenuItem::Choice(Command::PreviewBottom, side == PanelSide::Bottom),
                MenuItem::Choice(Command::PreviewTop, side == PanelSide::Top),
            ];
            let h = 52.0 + 30.0 * items.len() as f32;
            let below = side != PanelSide::Bottom;
            let pos = if below {
                pos2(side_r.right() - 300.0, side_r.bottom() + 4.0)
            } else {
                pos2(side_r.right() - 300.0, side_r.top() - h - 4.0)
            };
            self.menu = Some(crate::app::Menu {
                pos,
                query: String::new(),
                cursor: 0,
                items,
                opened: std::time::Instant::now(),
            });
        }
        let Some(e) = e else { return };
        let mut right = side_r.left() - 8.0;
        if band {
            // Open and Quick Look here (there's no footer in a band).
            right = self.open_buttons(ui, Rect::from_min_max(pos2(right - 210.0, cy - 14.0), pos2(right, cy + 14.0)));
            right -= 10.0;
            let p = ui.painter();
            let g =
                super::elided_in(p, &e.name, super::bold(14.5), pal.text_strong, (right - r.left() - 16.0).max(20.0));
            p.galley(pos2(r.left() + 16.0, cy - g.size().y / 2.0), g, pal.text_strong);
        } else {
            let p = ui.painter();
            let kind = self.kind_of(e);
            let g = super::elided_in(p, &kind, super::bold(13.0), pal.text_dim, (right - r.left() - 16.0).max(20.0));
            p.galley(pos2(r.left() + 16.0, cy - g.size().y / 2.0), g, pal.text_dim);
        }
    }

    /// Open (the accent button) and Quick Look, side by side in `r`. Returns their left edge.
    fn open_buttons(&mut self, ui: &mut Ui, r: Rect) -> f32 {
        let pal = self.pal();
        let gap = 8.0;
        let bw = ((r.width() - gap) / 2.0).min(120.0);
        let ql_r = Rect::from_min_max(pos2(r.right() - bw, r.top()), r.max);
        let open_r = Rect::from_min_max(pos2(ql_r.left() - gap - bw, r.top()), pos2(ql_r.left() - gap, r.bottom()));
        let open = ui.interact(open_r, Id::new("insp_open"), Sense::click());
        let ql = ui.interact(ql_r, Id::new("insp_ql"), Sense::click());
        let p = ui.painter();
        p.rect_filled(
            open_r,
            pal.round(6.0),
            if open.hovered() { pal.accent.gamma_multiply(1.15) } else { pal.accent },
        );
        super::text_bold(p, open_r.center(), Align2::CENTER_CENTER, "Open", 13.0, pal.on_accent);
        p.rect_filled(ql_r, pal.round(6.0), if ql.hovered() { pal.hover } else { pal.tab_hover });
        p.rect_stroke(ql_r, pal.round(6.0), Stroke::new(1.0, pal.border), StrokeKind::Inside);
        text(p, ql_r.center(), Align2::CENTER_CENTER, "Quick Look", 13.0, pal.text);
        if open.on_hover_text("Open (Enter)").clicked() {
            self.open_entries();
        }
        if ql.on_hover_text("Large preview (Space)").clicked() {
            self.quicklook = true;
        }
        open_r.left()
    }

    /// The footer of a column: Open and Quick Look, level with the bottom bars.
    fn inspector_foot(&mut self, ui: &mut Ui, r: Rect) {
        let pal = self.pal();
        ui.painter().hline(r.x_range(), r.top() + 0.5, Stroke::new(1.0, pal.row_sep));
        let inner = Rect::from_min_max(pos2(r.left() + 12.0, r.top() + 6.0), pos2(r.right() - 12.0, r.bottom() - 6.0));
        let w = inner.width().min(260.0);
        self.open_buttons(ui, Rect::from_center_size(inner.center(), vec2(w, inner.height())));
    }

    fn kind_of(&mut self, e: &Entry) -> String {
        self.pv
            .shown_for(&e.path)
            .and_then(|s| s.loaded.kind.clone())
            .unwrap_or_else(|| crate::preview::friendly_kind(e).unwrap_or_else(|| type_label(e)))
    }

    /// The preview, edge to edge. Videos and music get a play button, then play here
    /// with the same controls as Quick Look.
    fn inspector_preview(&mut self, ui: &mut Ui, pr: Rect, e: &Entry) {
        let pal = self.pal();
        let fx = self.fx();
        let well = if fx.glass { super::glass::with_alpha(pal.bg, 0.45) } else { pal.bg };
        ui.painter().rect_filled(pr, 0.0, well);
        let kind = crate::preview::classify(&e.path);
        let media = e.is_file
            && matches!(kind, crate::preview::Kind::Video | crate::preview::Kind::Audio)
            && crate::preview::player::available();
        if !media {
            self.insp_player = None;
        }
        let mut player = self.insp_player.take();
        if let Some(pl) = player.as_mut() {
            pl.update(ui.ctx());
        }
        let bar_h = 40.0;
        let (screen_r, bar) = if player.is_some() {
            let bar = Rect::from_min_max(pos2(pr.left(), pr.bottom() - bar_h), pr.max);
            (Rect::from_min_max(pr.min, pos2(pr.right(), bar.top())), Some(bar))
        } else {
            (pr, None)
        };
        let loading = self.pv.is_loading();
        let mut view = std::mem::take(&mut self.insp_view);
        let mut nav = None;
        match player.as_ref().and_then(|p| p.tex.as_ref()) {
            Some(tex) => {
                let size = tex.size_vec2();
                let s = (screen_r.width() / size.x).min(screen_r.height() / size.y);
                let img = Rect::from_center_size(screen_r.center(), size * s);
                let p = ui.painter();
                p.rect_filled(screen_r, 0.0, Color32::BLACK);
                p.image(tex.id(), img, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
            }
            None => {
                let shown = self.pv.shown_for(&e.path).map(|s| &*s);
                nav = preview_view::show(
                    ui,
                    screen_r.shrink(10.0),
                    e,
                    shown,
                    loading,
                    &mut view,
                    pal,
                    Style { big: false },
                );
            }
        }
        self.insp_view = view;
        if let Some(Nav::Page(n)) = nav {
            self.preview_page.1 = n;
        }
        match (player.as_mut(), bar) {
            (Some(pl), Some(bar)) => {
                // Clicking the picture pauses or plays.
                if ui.interact(screen_r, Id::new("insp_video"), Sense::click()).clicked() {
                    pl.toggle_pause();
                }
                if let Some(err) = &pl.error {
                    text(
                        ui.painter(),
                        screen_r.center_bottom() - vec2(0.0, 18.0),
                        Align2::CENTER_CENTER,
                        err,
                        12.5,
                        pal.text_dim,
                    );
                }
                let p = ui.painter();
                p.rect_filled(bar, 0.0, pal.sidebar);
                p.hline(bar.x_range(), bar.top() + 0.5, Stroke::new(1.0, pal.row_sep));
                super::quicklook::video_controls(ui, bar.shrink2(vec2(6.0, 2.0)), pl, pal);
            }
            _ if media => {
                let c = screen_r.center();
                let r = Rect::from_center_size(c, vec2(54.0, 54.0));
                let resp = ui.interact(r, Id::new("insp_play"), Sense::click());
                let p = ui.painter();
                p.circle_filled(c, 27.0, Color32::from_black_alpha(if resp.hovered() { 200 } else { 150 }));
                p.circle_stroke(c, 27.0, Stroke::new(1.0, Color32::from_white_alpha(60)));
                let (x, y) = (c.x + 3.0, c.y);
                p.add(egui::Shape::convex_polygon(
                    vec![pos2(x - 9.0, y - 12.0), pos2(x + 12.0, y), pos2(x - 9.0, y + 12.0)],
                    Color32::WHITE,
                    Stroke::NONE,
                ));
                let tip = if kind == crate::preview::Kind::Audio { "Play" } else { "Play the video" };
                if resp.on_hover_text(tip).clicked() {
                    player = Some(crate::preview::player::Player::open(&e.path, false));
                }
            }
            _ => {}
        }
        self.insp_player = player;
    }

    /// Name, size, rating, tags and the Information list, scrolling if they don't fit.
    fn inspector_details(&mut self, ui: &mut Ui, area: Rect, e: &Entry, column: bool) {
        let pal = self.pal();
        let date_style = self.cfg.date_style;
        let shown = self.pv.shown_for(&e.path);
        let (mut info, times) = match shown {
            Some(s) => (s.loaded.info.clone(), s.loaded.times),
            None => (Vec::new(), Default::default()),
        };
        info.retain(|(k, _)| !k.starts_with('_'));
        let kind = self.kind_of(e);
        let rows = info_rows(e, info, times, date_style);
        let ratings = self.cfg.ext_on(crate::ext::Ext::Ratings);
        let tags_on = self.cfg.ext_on(crate::ext::Ext::Tags);
        if ratings || tags_on {
            self.meta.observe(&e.path);
        }
        let rating = self.meta.rating(&e.path);
        let tags: Vec<crate::meta::TagDef> =
            self.meta.tags_of(&e.path).iter().filter_map(|t| self.meta.tag(t).cloned()).collect();

        let mut rate: Option<u8> = None;
        let mut show_tag: Option<String> = None;
        let mut edit_tags = false;
        let pad = 14.0;
        let inner = Rect::from_min_max(pos2(area.left() + pad, area.top()), pos2(area.right() - pad, area.bottom()));
        ui.scope_builder(UiBuilder::new().max_rect(inner), |ui| {
            ui.set_clip_rect(area.intersect(ui.clip_rect()));
            egui::ScrollArea::vertical().id_salt("insp_details").auto_shrink(false).show(ui, |ui| {
                ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
                let full_w = ui.available_width();
                ui.add_space(12.0);
                // A wide band puts the Information beside the rest instead of below it.
                let two = !column && full_w >= 560.0;
                let meta_w = if two { (full_w * 0.4).clamp(220.0, 320.0) } else { full_w };
                let info_w = if two { (full_w - meta_w - 28.0).min(560.0) } else { full_w };
                let layout = if two {
                    egui::Layout::left_to_right(egui::Align::Min)
                } else {
                    egui::Layout::top_down(egui::Align::Min)
                };
                ui.with_layout(layout, |ui| {
                    ui.vertical(|ui| {
                        ui.set_width(meta_w);
                        let w = meta_w;
                        if column {
                            // The name, in full, and what it is.
                            let g = super::wrapped_in(ui.painter(), &e.name, super::bold(16.0), pal.text_strong, w, 4);
                            let (r, _) = ui.allocate_exact_size(vec2(w, g.size().y), Sense::hover());
                            ui.painter().galley(pos2(r.center().x, r.top()), g, pal.text_strong);
                            ui.add_space(3.0);
                        }
                        let sub = if e.is_dir {
                            if column { String::new() } else { kind.clone() }
                        } else if column {
                            human_size(e.size)
                        } else {
                            format!("{kind} — {}", human_size(e.size))
                        };
                        if !sub.is_empty() {
                            let (r, _) = ui.allocate_exact_size(vec2(w, 18.0), Sense::hover());
                            let g = elided(ui.painter(), &sub, 12.5, pal.text_dim, w);
                            let x = if column { r.center().x - g.size().x / 2.0 } else { r.left() };
                            ui.painter().galley(pos2(x, r.center().y - g.size().y / 2.0), g, pal.text_dim);
                        }
                        ui.add_space(8.0);
                        if ratings {
                            let size = 18.0;
                            let (r, _) = ui.allocate_exact_size(vec2(w, 26.0), Sense::hover());
                            let left = if column { r.center().x - size * 2.5 } else { r.left() };
                            rate =
                                super::tags::star_row(ui, Id::new("insp_stars"), left, r.center().y, size, rating, pal);
                            ui.add_space(6.0);
                        }
                        if tags_on {
                            // Chips, wrapping; click one to see everything with it, + to edit.
                            let chip_h = 22.0;
                            let labels: Vec<(Option<&crate::meta::TagDef>, std::sync::Arc<egui::Galley>)> = tags
                                .iter()
                                .map(Some)
                                .chain(std::iter::once(None))
                                .map(|t| {
                                    let label = t.map_or("+ Tag".to_string(), |t| t.name.clone());
                                    (t, ui.painter().layout_no_wrap(label, super::font(12.5), pal.text))
                                })
                                .collect();
                            let mut spots = Vec::new();
                            let (mut x, mut y) = (0.0f32, 0.0f32);
                            for (t, g) in &labels {
                                let cw = g.size().x + if t.is_some() { 28.0 } else { 16.0 };
                                if x + cw > w && x > 0.0 {
                                    x = 0.0;
                                    y += chip_h + 6.0;
                                }
                                spots.push(Rect::from_min_size(pos2(x, y), vec2(cw, chip_h)));
                                x += cw + 6.0;
                            }
                            let (block, _) = ui.allocate_exact_size(vec2(w, y + chip_h), Sense::hover());
                            // A column centers its rows of chips.
                            for (i, ((t, g), spot)) in labels.into_iter().zip(&spots).enumerate() {
                                let row_w = spots
                                    .iter()
                                    .filter(|s| s.top() == spot.top())
                                    .map(|s| s.right())
                                    .fold(0.0, f32::max);
                                let dx = if column { (w - row_w) / 2.0 } else { 0.0 };
                                let r = spot.translate(block.min.to_vec2() + vec2(dx, 0.0));
                                let resp = ui.interact(r, Id::new(("insp_tag", i)), Sense::click());
                                let p = ui.painter();
                                match t {
                                    Some(t) => {
                                        let c = t.color32();
                                        p.rect_filled(
                                            r,
                                            chip_h / 2.0,
                                            c.gamma_multiply(if resp.hovered() { 0.35 } else { 0.22 }),
                                        );
                                        p.circle_filled(pos2(r.left() + 11.0, r.center().y), 4.5, c);
                                        p.galley(pos2(r.left() + 20.0, r.center().y - g.size().y / 2.0), g, pal.text);
                                        if resp.on_hover_text(format!("Show everything tagged “{}”", t.name)).clicked()
                                        {
                                            show_tag = Some(t.name.clone());
                                        }
                                    }
                                    None => {
                                        p.rect_filled(
                                            r,
                                            chip_h / 2.0,
                                            if resp.hovered() { pal.hover } else { pal.tab_hover },
                                        );
                                        p.galley(
                                            pos2(r.left() + 8.0, r.center().y - g.size().y / 2.0),
                                            g,
                                            pal.text_dim,
                                        );
                                        edit_tags = resp.on_hover_text("Add or remove tags").clicked();
                                    }
                                }
                            }
                            ui.add_space(10.0);
                        }
                    });
                    if two {
                        ui.add_space(28.0);
                    }
                    ui.vertical(|ui| {
                        ui.set_width(info_w);
                        let w = info_w;
                        // Information: a rounded list, a line between rows.
                        if !two {
                            ui.add_space(4.0);
                        }
                        let (r, _) = ui.allocate_exact_size(vec2(w, 24.0), Sense::hover());
                        super::text_bold(
                            ui.painter(),
                            pos2(r.left() + 2.0, r.center().y),
                            Align2::LEFT_CENTER,
                            "Information",
                            12.5,
                            pal.text_strong,
                        );
                        ui.add_space(4.0);
                        let label_w = 100.0f32.min(w * 0.4);
                        let lay: Vec<(&str, std::sync::Arc<egui::Galley>, f32)> = rows
                            .iter()
                            .map(|(k, v)| {
                                let g = right_aligned(ui.painter(), v, 12.5, pal.text, w - label_w - 28.0);
                                let h = (g.size().y + 14.0).max(32.0);
                                (k.as_str(), g, h)
                            })
                            .collect();
                        let total: f32 = lay.iter().map(|l| l.2).sum();
                        let (boxr, _) = ui.allocate_exact_size(vec2(w, total), Sense::hover());
                        let p = ui.painter();
                        p.rect_filled(boxr, pal.round(8.0), pal.bg);
                        p.rect_stroke(boxr, pal.round(8.0), Stroke::new(1.0, pal.border), StrokeKind::Inside);
                        let mut y = boxr.top();
                        for (i, (k, g, h)) in lay.into_iter().enumerate() {
                            let row = Rect::from_min_size(pos2(boxr.left(), y), vec2(w, h));
                            if i > 0 {
                                ui.painter().hline(
                                    row.x_range().shrink(10.0),
                                    row.top(),
                                    Stroke::new(1.0, pal.row_sep),
                                );
                            }
                            let resp = ui.interact(row, Id::new(("insp_row", i)), Sense::hover());
                            let p = ui.painter();
                            text(
                                p,
                                pos2(row.left() + 12.0, row.center().y),
                                Align2::LEFT_CENTER,
                                k,
                                12.5,
                                pal.text_dim,
                            );
                            // Values right-aligned, like Finder.
                            let value = rows[i].1.clone();
                            p.galley(pos2(row.right() - 12.0, row.center().y - g.size().y / 2.0), g, pal.text);
                            if value.len() > 30 {
                                resp.on_hover_text(value);
                            }
                            y += h;
                        }
                    });
                });
                ui.add_space(14.0);
            });
        });
        if let Some(n) = rate {
            self.rate(std::slice::from_ref(&e.path), n);
        }
        if let Some(t) = show_tag {
            self.show_tag(&t);
        }
        if edit_tags {
            self.dialog =
                Some(crate::app::Dialog::TagFiles { paths: vec![e.path.clone()], new_tag: String::new(), error: None });
        }
    }
}

/// The Information list's rows: the previewer's details, then dates, permissions,
/// link target, exact size and where it is.
fn info_rows(
    e: &Entry,
    info: Vec<(String, String)>,
    times: crate::preview::Times,
    date_style: crate::config::DateStyle,
) -> Vec<(String, String)> {
    let mut rows = info;
    let mut push_time = |k: &str, t: Option<std::time::SystemTime>| {
        if let Some(t) = t {
            rows.push((k.to_string(), format_time(t, date_style)));
        }
    };
    push_time("Created", times.created);
    push_time("Modified", e.modified);
    push_time("Last opened", times.accessed);
    if let Some(m) = times.mode {
        rows.push(("Permissions".into(), format_mode(m)));
    }
    if e.is_symlink
        && let Ok(t) = std::fs::read_link(&e.path)
    {
        rows.push(("Links to".into(), t.to_string_lossy().into_owned()));
    }
    if !e.is_dir {
        rows.push(("Size".into(), format!("{} bytes", group_digits(e.size))));
    }
    rows.push(("Where".into(), e.path.parent().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default()));
    rows
}

/// The window with the panel's side filled in (the "move the panel" button).
fn panel_glyph(p: &egui::Painter, r: Rect, c: Color32, side: PanelSide) {
    let r = r.shrink2(vec2(1.0, 3.0));
    p.rect_stroke(r, 2.0, Stroke::new(1.3, c), StrokeKind::Inside);
    let band = match side {
        PanelSide::Right => Rect::from_min_max(pos2(r.right() - r.width() * 0.38, r.top()), r.max),
        PanelSide::Left => Rect::from_min_max(r.min, pos2(r.left() + r.width() * 0.38, r.bottom())),
        PanelSide::Bottom => Rect::from_min_max(pos2(r.left(), r.bottom() - r.height() * 0.42), r.max),
        PanelSide::Top => Rect::from_min_max(r.min, pos2(r.right(), r.top() + r.height() * 0.42)),
    };
    p.rect_filled(band, 2.0, c.gamma_multiply(0.7));
}

/// Wrapped text (up to 3 lines) laid out to the left of its anchor x.
fn right_aligned(
    p: &egui::Painter,
    s: &str,
    size: f32,
    color: egui::Color32,
    max_w: f32,
) -> std::sync::Arc<egui::Galley> {
    let mut job =
        egui::text::LayoutJob::single_section(s.to_string(), egui::TextFormat::simple(super::font(size), color));
    job.wrap =
        egui::text::TextWrapping { max_width: max_w.max(8.0), max_rows: 3, break_anywhere: true, ..Default::default() };
    job.halign = egui::Align::RIGHT;
    p.layout_job(job)
}

fn group_digits(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}
