//! Quick Look: a large preview of the item under the cursor (Space to toggle).

use egui::emath::TSTransform;
use egui::{Align2, Area, Color32, Id, Order, Rect, Sense, Stroke, StrokeKind, pos2, vec2};

use super::preview_view::{self, Nav, Style};
use super::{appear, elided, glass, text};
use crate::app::{FileFlier, format_time, human_size};
use crate::icons;
use crate::ui::pane_view::{file_kind, type_label};

impl FileFlier {
    pub(crate) fn quicklook_ui(&mut self, ctx: &egui::Context) {
        if !self.quicklook {
            self.ql_player = None; // stops the video and its sound
            return;
        }
        let Some(e) = self.request_preview() else {
            self.quicklook = false;
            self.ql_player = None;
            return;
        };
        // Videos play: keep one player for the item shown.
        let is_video = e.is_file
            && crate::preview::classify(&e.path) == crate::preview::Kind::Video
            && crate::preview::player::available();
        if !is_video {
            self.ql_player = None;
        } else if self.ql_player.as_ref().is_none_or(|p| p.path != e.path) {
            let muted = self.ql_player.as_ref().is_some_and(|p| p.muted);
            self.ql_player = Some(crate::preview::player::Player::open(&e.path, muted));
        }
        if let Some(p) = self.ql_player.as_mut() {
            p.update(ctx);
        }
        let mut player = self.ql_player.take();
        let tint = super::tint::color_of(&self.cfg.folder_colors, self.cfg.folder_scheme, self.pal().accent, &e.path);
        let loading = self.pv.is_loading();
        let mut view = std::mem::take(&mut self.ql_view);
        let mut nav = None;
        let pal = self.pal();
        let fx = self.fx();
        let date_style = self.cfg.date_style;
        let (pos, total) = (self.tab().cursor + 1, self.tab().visible.len());
        let screen = ctx.content_rect();
        let t = appear(ctx, Id::new("ql_appear"), fx.dur(0.18));
        let mut close = false;
        let mut open = false;

        // Dimmed backdrop; clicking it closes.
        Area::new(Id::new("ql_backdrop")).order(Order::Foreground).fixed_pos(screen.min).show(ctx, |ui| {
            let (r, resp) = ui.allocate_exact_size(screen.size(), Sense::click());
            ui.painter().rect_filled(r, 0.0, Color32::from_black_alpha((120.0 * t) as u8));
            close |= resp.clicked();
        });

        let size = vec2((screen.width() * 0.82).min(1040.0), (screen.height() * 0.84).min(740.0));
        let card = Rect::from_center_size(screen.center(), size);
        let shown = self.pv.shown_for(&e.path).map(|s| &*s);
        // The card stays above the dimmed backdrop (clicking the backdrop would otherwise
        // bring it to the front, dimming the card too).
        ctx.move_to_top(egui::LayerId::new(Order::Foreground, Id::new("ql_card")));
        Area::new(Id::new("ql_card")).order(Order::Foreground).fixed_pos(card.min).show(ctx, |ui| {
            // Grow from 94% to full size while fading in.
            let s = 0.94 + 0.06 * t;
            let c = card.center().to_vec2();
            ui.ctx().set_transform_layer(ui.layer_id(), TSTransform::new(c * (1.0 - s), s));
            ui.multiply_opacity(t);

            let (r, _) = ui.allocate_exact_size(size, Sense::click()); // swallow clicks on the card
            let p = ui.painter().clone();
            p.add(
                egui::Shadow { offset: [0, 16], blur: 48, spread: 0, color: Color32::from_black_alpha(140) }
                    .as_shape(r, 14),
            );
            // Solid-ish even in glass mode: the app can't blur its own content behind it.
            let fill = if fx.glass { glass::with_alpha(pal.popup, 0.97) } else { pal.popup };
            if fx.glass {
                glass::panel(&p, r, fill, 14, pal);
            } else {
                p.rect_filled(r, pal.round(14.0), fill);
                p.rect_stroke(r, pal.round(14.0), Stroke::new(1.0, pal.border), StrokeKind::Inside);
            }

            // Header: icon, name, details, actions.
            let header = Rect::from_min_size(r.min, vec2(r.width(), 64.0));
            let icon = Rect::from_center_size(pos2(header.left() + 34.0, header.center().y), vec2(30.0, 30.0));
            if e.is_dir {
                icons::folder_icon(&p, icon, pal, tint, crate::system_icons::special_folder(&e.path));
            } else {
                icons::file(&p, icon, file_kind(&e), pal);
            }
            let kind = shown
                .and_then(|s| s.loaded.kind.clone())
                .or_else(|| crate::preview::friendly_kind(&e))
                .unwrap_or_else(|| type_label(&e));
            let mut meta = vec![kind];
            if !e.is_dir {
                meta.push(human_size(e.size));
            }
            if let Some(s) = shown {
                for key in ["Dimensions", "Duration", "Pages", "Contains"] {
                    if let Some((_, v)) = s.loaded.info.iter().find(|(k, _)| k == key) {
                        meta.push(if key == "Pages" { format!("{v} pages") } else { v.clone() });
                    }
                }
            }
            if let Some(m) = e.modified {
                meta.push(format_time(m, date_style));
            }
            let name_w = header.width() - 64.0 - 170.0;
            let g = elided(&p, &e.name, 17.0, pal.text_strong, name_w);
            p.galley(pos2(header.left() + 62.0, header.top() + 13.0), g, pal.text_strong);
            let g = elided(&p, &meta.join("  ·  "), 12.5, pal.text_dim, name_w);
            p.galley(pos2(header.left() + 62.0, header.top() + 37.0), g, pal.text_dim);

            let close_r = Rect::from_center_size(pos2(header.right() - 30.0, header.center().y), vec2(30.0, 30.0));
            close |= super::icon_button(ui, close_r, Id::new("ql_close"), true, pal, "Close (Space)", icons::close)
                .clicked();
            let open_r = Rect::from_min_size(pos2(close_r.left() - 96.0, header.center().y - 15.0), vec2(84.0, 30.0));
            let open_resp = ui.interact(open_r, Id::new("ql_open"), Sense::click());
            let fill = if open_resp.hovered() { pal.accent.gamma_multiply(1.15) } else { pal.accent };
            p.rect_filled(open_r, pal.round(6.0), fill);
            text(&p, open_r.center(), Align2::CENTER_CENTER, "Open", 13.5, pal.on_accent);
            open |= open_resp.on_hover_text("Open (Enter)").clicked();
            p.hline(r.x_range().shrink(1.0), header.bottom(), Stroke::new(1.0, pal.row_sep));

            // Footer: position and key hints.
            let footer = Rect::from_min_max(pos2(r.left(), r.bottom() - 34.0), r.max);
            p.hline(r.x_range().shrink(1.0), footer.top(), Stroke::new(1.0, pal.row_sep));
            text(
                &p,
                pos2(footer.left() + 18.0, footer.center().y),
                Align2::LEFT_CENTER,
                format!("{pos} of {total}"),
                12.0,
                pal.text_dim,
            );
            text(
                &p,
                pos2(footer.right() - 18.0, footer.center().y),
                Align2::RIGHT_CENTER,
                if player.is_some() {
                    "K  play/pause   ·   J L  −5 s / +5 s   ·   E F  next / previous frame   ·   M  mute   ·   ← →  browse   ·   Space  close"
                } else {
                    "← →  browse   ·   scroll  zoom   ·   PgUp PgDn  pages   ·   Enter  open   ·   Space  close"
                },
                12.0,
                pal.text_dim,
            );

            // Body.
            let body = Rect::from_min_max(
                pos2(r.left() + 16.0, header.bottom() + 12.0),
                pos2(r.right() - 16.0, footer.top() - 12.0),
            );
            match player.as_mut() {
                Some(pl) => {
                    let bar = Rect::from_min_max(pos2(body.left(), body.bottom() - 36.0), body.max);
                    let screen_r = Rect::from_min_max(body.min, pos2(body.right(), bar.top() - 8.0));
                    match &pl.tex {
                        Some(tex) => {
                            let size = tex.size_vec2();
                            let s = (screen_r.width() / size.x).min(screen_r.height() / size.y);
                            let img = Rect::from_center_size(screen_r.center(), size * s);
                            p.rect_filled(screen_r, pal.round(8.0), Color32::BLACK);
                            p.image(tex.id(), img, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
                        }
                        // Until the first frame: the still the preview made.
                        None => {
                            nav = preview_view::show(ui, screen_r, &e, shown, loading, &mut view, pal, Style { big: true });
                        }
                    }
                    if let Some(err) = &pl.error {
                        text(&p, screen_r.center_bottom() - vec2(0.0, 20.0), Align2::CENTER_CENTER, err, 13.0, pal.text_dim);
                    }
                    // Clicking the picture pauses or plays.
                    if ui.interact(screen_r, Id::new("ql_video"), Sense::click()).clicked() {
                        pl.toggle_pause();
                    }
                    video_controls(ui, bar, pl, pal);
                }
                None => {
                    nav = preview_view::show(ui, body, &e, shown, loading, &mut view, pal, Style { big: true });
                }
            }
        });
        self.ql_player = player;
        self.ql_view = view;
        if let Some(Nav::Page(n)) = nav {
            self.preview_page.1 = n;
        }

        if open {
            self.quicklook = false;
            self.open_entries();
        } else if close {
            self.quicklook = false;
        }
    }
}

/// Play/pause, time, a seek bar and mute, under a playing video.
pub(crate) fn video_controls(
    ui: &mut egui::Ui,
    bar: Rect,
    pl: &mut crate::preview::player::Player,
    pal: &crate::theme::Palette,
) {
    use crate::preview::format_duration;
    let p = ui.painter().clone();
    let cy = bar.center().y;
    // Play / pause.
    let pr = Rect::from_center_size(pos2(bar.left() + 18.0, cy), vec2(32.0, 32.0));
    let presp = ui.interact(pr, Id::new("ql_play"), Sense::click());
    if presp.hovered() {
        p.rect_filled(pr, pal.round(6.0), pal.tab_hover);
    }
    let c = pal.text_strong;
    if pl.paused() || pl.ended {
        let (x, y) = (pr.center().x - 4.0, cy);
        p.add(egui::Shape::convex_polygon(
            vec![pos2(x - 2.0, y - 8.0), pos2(x + 10.0, y), pos2(x - 2.0, y + 8.0)],
            c,
            Stroke::NONE,
        ));
    } else {
        for dx in [-4.5, 2.0] {
            p.rect_filled(Rect::from_min_size(pos2(pr.center().x + dx, cy - 7.5), vec2(3.5, 15.0)), 1.0, c);
        }
    }
    let tip = if pl.ended {
        "Play again (K)"
    } else if pl.paused() {
        "Play (K)"
    } else {
        "Pause (K)"
    };
    if presp.on_hover_text(tip).clicked() {
        pl.toggle_pause();
    }
    let duration = pl.info.map_or(0.0, |i| i.duration);
    let has_audio = pl.info.is_some_and(|i| i.has_audio);
    // Mute, at the right end.
    let mut right = bar.right();
    if has_audio {
        let mr = Rect::from_center_size(pos2(bar.right() - 18.0, cy), vec2(32.0, 32.0));
        right = mr.left() - 6.0;
        let mresp = ui.interact(mr, Id::new("ql_mute"), Sense::click());
        if mresp.hovered() {
            p.rect_filled(mr, pal.round(6.0), pal.tab_hover);
        }
        let (x, y) = (mr.center().x - 5.0, cy);
        p.rect_filled(Rect::from_min_size(pos2(x - 4.0, y - 3.0), vec2(4.0, 6.0)), 0.0, c);
        p.add(egui::Shape::convex_polygon(
            vec![pos2(x, y - 3.0), pos2(x + 5.0, y - 7.0), pos2(x + 5.0, y + 7.0), pos2(x, y + 3.0)],
            c,
            Stroke::NONE,
        ));
        if pl.muted {
            p.line_segment([pos2(x + 8.0, y - 4.0), pos2(x + 14.0, y + 4.0)], Stroke::new(1.6, c));
            p.line_segment([pos2(x + 14.0, y - 4.0), pos2(x + 8.0, y + 4.0)], Stroke::new(1.6, c));
        } else {
            p.line_segment([pos2(x + 8.0, y - 3.0), pos2(x + 8.0, y + 3.0)], Stroke::new(1.6, c));
            p.line_segment([pos2(x + 11.5, y - 6.0), pos2(x + 11.5, y + 6.0)], Stroke::new(1.6, c));
        }
        if mresp.on_hover_text(if pl.muted { "Unmute (M)" } else { "Mute (M)" }).clicked() {
            pl.toggle_mute();
        }
    }
    // Time and seek bar. Dragging shows where it would go; letting go seeks.
    let label = format!("{} / {}", format_duration(pl.position()), format_duration(duration));
    let tw = p.layout_no_wrap(label.clone(), super::font(12.5), pal.text).size().x;
    let time_r = Rect::from_min_max(pos2(right - tw - 8.0, bar.top()), pos2(right, bar.bottom()));
    let track = Rect::from_min_max(pos2(pr.right() + 12.0, cy - 3.0), pos2(time_r.left() - 12.0, cy + 3.0));
    let hit = track.expand2(vec2(0.0, 10.0));
    let sresp = ui.interact(hit, Id::new("ql_seek"), Sense::click_and_drag());
    let frac_at = |x: f32| ((x - track.left()) / track.width()).clamp(0.0, 1.0);
    let scrub = sresp.interact_pointer_pos().filter(|_| sresp.dragged() || sresp.drag_stopped()).map(|p| frac_at(p.x));
    let frac = scrub.unwrap_or(if duration > 0.0 { (pl.position() / duration) as f32 } else { 0.0 });
    p.rect_filled(track, pal.round(3.0), pal.input);
    p.rect_filled(
        Rect::from_min_max(track.min, pos2(track.left() + track.width() * frac, track.bottom())),
        3.0,
        pal.accent,
    );
    let knob = pos2(track.left() + track.width() * frac, cy);
    p.circle_filled(knob, if sresp.hovered() || sresp.dragged() { 7.0 } else { 5.5 }, pal.accent);
    if duration > 0.0 {
        if (sresp.drag_stopped() || sresp.clicked())
            && let Some(pos) = sresp.interact_pointer_pos()
        {
            pl.seek(frac_at(pos.x) as f64 * duration);
        }
        if sresp.hovered() || sresp.dragged() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
    }
    let shown_time =
        scrub.map_or(label, |f| format!("{} / {}", format_duration(f as f64 * duration), format_duration(duration)));
    text(&p, pos2(time_r.right(), cy), Align2::RIGHT_CENTER, shown_time, 12.5, pal.text);
}
