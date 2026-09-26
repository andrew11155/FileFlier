//! Right-hand preview panel, Finder style: a large zoomable preview, the item's
//! name, and an Information section with everything we know about it.

use egui::{Align2, CursorIcon, Id, Rect, Sense, Stroke, StrokeKind, Ui, UiBuilder, pos2, vec2};

use super::preview_view::{self, Nav, Style};
use super::{elided, text, wrapped};
use crate::app::{FileFlier, format_mode, format_time, human_size};
use crate::preview::Content;
use crate::ui::pane_view::type_label;

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
        super::surface(ui.painter(), rect, pal.sidebar, &fx, pal);
        if !fx.glass {
            ui.painter().vline(rect.left() + 0.5, rect.y_range(), Stroke::new(1.0, pal.border));
        }

        // Resize handle on the left edge.
        let handle = Rect::from_min_max(pos2(rect.left() - 3.0, rect.top()), pos2(rect.left() + 3.0, rect.bottom()));
        let h = ui.interact(handle, Id::new("inspector_resize"), Sense::drag());
        if h.hovered() || h.dragged() {
            ui.ctx().set_cursor_icon(CursorIcon::ResizeHorizontal);
        }
        if h.dragged() {
            self.cfg.inspector_width = (self.cfg.inspector_width - h.drag_delta().x).clamp(240.0, 900.0);
        }
        if h.drag_stopped() {
            self.cfg.save();
        }

        let Some(e) = self.request_preview() else {
            text(ui.painter(), rect.center(), Align2::CENTER_CENTER, "Nothing selected", 14.0, pal.text_dim);
            return;
        };
        let loading = self.pv.is_loading();
        let date_style = self.cfg.date_style;
        let inner = rect.shrink2(vec2(14.0, 12.0));

        // Preview: taller for things you read (text, documents, tables).
        let reading = self.pv.shown_for(&e.path).is_some_and(|s| {
            matches!(
                s.loaded.content,
                Content::Text { .. }
                    | Content::Document { .. }
                    | Content::Table { .. }
                    | Content::Dir(_)
                    | Content::Listing(_)
                    | Content::Page { .. }
            )
        });
        let preview_h = if reading {
            (inner.height() * 0.58).max(160.0)
        } else {
            inner.width().min(inner.height() * 0.5).max(140.0)
        };
        let pr = Rect::from_min_size(inner.min, vec2(inner.width(), preview_h));
        let well = if fx.glass { super::glass::with_alpha(pal.bg, 0.5) } else { pal.bg.gamma_multiply(0.7) };
        ui.painter().rect_filled(pr, 8.0, well);
        let mut view = std::mem::take(&mut self.insp_view);
        let nav = {
            let shown = self.pv.shown_for(&e.path).map(|s| &*s);
            preview_view::show(ui, pr.shrink(1.0), &e, shown, loading, &mut view, pal, Style { big: false })
        };
        self.insp_view = view;
        if let Some(Nav::Page(n)) = nav {
            self.preview_page.1 = n;
        }
        ui.painter().rect_stroke(pr, 8.0, Stroke::new(1.0, pal.border), StrokeKind::Inside);

        let shown = self.pv.shown_for(&e.path);
        let (kind, mut info, times) = match shown {
            Some(s) => (s.loaded.kind.clone(), s.loaded.info.clone(), s.loaded.times),
            None => (None, Vec::new(), Default::default()),
        };
        info.retain(|(k, _)| !k.starts_with('_'));
        let kind = kind.unwrap_or_else(|| crate::preview::friendly_kind(&e).unwrap_or_else(|| type_label(&e)));

        // Name and kind.
        let mut y = pr.bottom() + 12.0;
        let p = ui.painter();
        let g = wrapped(p, &e.name, 16.0, pal.text_strong, inner.width(), 3);
        let name_h = g.size().y;
        p.galley(pos2(inner.center().x, y), g, pal.text_strong);
        y += name_h + 3.0;
        let sub = if e.is_dir { kind.clone() } else { format!("{kind} — {}", human_size(e.size)) };
        let g = elided(p, &sub, 12.5, pal.text_dim, inner.width());
        p.galley(pos2(inner.center().x - g.size().x / 2.0, y), g, pal.text_dim);
        y += 24.0;

        // Actions.
        let bw = ((inner.width() - 8.0) / 2.0).min(120.0);
        let open_r = Rect::from_min_size(pos2(inner.center().x - bw - 4.0, y), vec2(bw, 28.0));
        let ql_r = Rect::from_min_size(pos2(inner.center().x + 4.0, y), vec2(bw, 28.0));
        let open = ui.interact(open_r, Id::new("insp_open"), Sense::click());
        let ql = ui.interact(ql_r, Id::new("insp_ql"), Sense::click());
        let p = ui.painter();
        p.rect_filled(open_r, 6.0, if open.hovered() { pal.accent.gamma_multiply(1.15) } else { pal.accent });
        text(p, open_r.center(), Align2::CENTER_CENTER, "Open", 13.0, pal.on_accent);
        p.rect_filled(ql_r, 6.0, if ql.hovered() { pal.hover } else { pal.tab_hover });
        p.rect_stroke(ql_r, 6.0, Stroke::new(1.0, pal.border), StrokeKind::Inside);
        text(p, ql_r.center(), Align2::CENTER_CENTER, "Quick Look", 13.0, pal.text);
        if open.on_hover_text("Open (Enter)").clicked() {
            self.open_entries();
        }
        if ql.on_hover_text("Large preview (Space)").clicked() {
            self.quicklook = true;
        }
        y += 40.0;

        // Information.
        let mut rows: Vec<(String, String)> = info;
        let push_time = |rows: &mut Vec<(String, String)>, k: &str, t: Option<std::time::SystemTime>| {
            if let Some(t) = t {
                rows.push((k.to_string(), format_time(t, date_style)));
            }
        };
        push_time(&mut rows, "Created", times.created);
        push_time(&mut rows, "Modified", e.modified);
        push_time(&mut rows, "Last opened", times.accessed);
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

        let p = ui.painter();
        p.hline(inner.x_range(), y, Stroke::new(1.0, pal.row_sep));
        text(p, pos2(inner.left(), y + 14.0), Align2::LEFT_CENTER, "Information", 12.5, pal.text_strong);
        y += 28.0;
        let area = Rect::from_min_max(pos2(inner.left(), y), inner.max);
        if area.height() < 20.0 {
            return;
        }
        let label_w = 96.0f32.min(area.width() * 0.4);
        ui.scope_builder(UiBuilder::new().max_rect(area), |ui| {
            ui.set_clip_rect(area.intersect(ui.clip_rect()));
            egui::ScrollArea::vertical().id_salt("insp_info").auto_shrink(false).show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                let w = ui.available_width() - 4.0;
                for (k, v) in &rows {
                    let p = ui.painter();
                    let g = right_aligned(p, v, 12.5, pal.text, w - label_w - 8.0);
                    let h = (g.size().y + 8.0).max(22.0);
                    let (r, resp) = ui.allocate_exact_size(vec2(w, h), Sense::hover());
                    let p = ui.painter();
                    text(p, pos2(r.left(), r.top() + 11.0), Align2::LEFT_CENTER, k, 12.5, pal.text_dim);
                    // Values right-aligned, like Finder.
                    p.galley(pos2(r.right() - 2.0, r.top() + 4.0), g, pal.text);
                    if v.len() > 30 {
                        resp.on_hover_text(v);
                    }
                }
            });
        });
    }
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
