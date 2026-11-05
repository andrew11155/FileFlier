//! The shortcuts overlay (F1): every command with its keys, a search box, a
//! button to run each command, and keys you can change. Click a key to record a
//! new one, right-click it to remove it, `+` to add another.

use egui::{Align2, Color32, Id, Key, Rect, Sense, Stroke, StrokeKind, Ui, pos2, vec2};

use super::{elided, font, text};
use crate::app::FileFlier;
use crate::commands::{self, Command};
use crate::fuzzy;
use crate::theme::Palette;

/// Keys that work everywhere but aren't commands (shown, not editable).
const BASICS: &[(&str, &str)] = &[
    ("Move cursor (Shift selects)", "↑ ↓ PgUp PgDn Home End"),
    ("Open folder / file", "Enter  →"),
    ("Parent folder", "Backspace  ←"),
    ("Filter this folder", "Type anything"),
    ("Multi-select", "Ctrl+Click  Shift+Click"),
    ("Open folder in a new tab", "Ctrl+Double-click  Middle-click"),
    ("Drag files (hold Ctrl to copy)", "Drag"),
    ("Switch views", "Ctrl+Wheel"),
    ("Quick Look video: play, back, ahead, mute", "K  J  L  M"),
];

const ROW_H: f32 = 34.0;

/// What a click in the overlay asked for, applied after drawing.
enum Op {
    Record(Command, Option<usize>),
    Remove(Command, usize),
    Reset(Command),
    ResetAll,
    Run(Command),
}

pub(crate) struct Outcome {
    pub run: Option<Command>,
    pub close: bool,
}

impl FileFlier {
    /// Draw the overlay's contents. Returns a command to run (which closes it).
    pub(crate) fn shortcuts_ui(
        &mut self,
        ui: &mut Ui,
        query: &mut String,
        cursor: &mut usize,
        record: &mut Option<(Command, Option<usize>)>,
    ) -> Outcome {
        let pal = self.pal();
        let ctx = ui.ctx().clone();
        let mut out = Outcome { run: None, close: false };

        // Recording: the next key press (with its modifiers) becomes the shortcut.
        if let Some((cmd, slot)) = *record {
            let press = ctx.input_mut(|i| {
                let press = i.events.iter().find_map(|e| match e {
                    egui::Event::Key { key, pressed: true, repeat: false, modifiers, .. } => Some((*modifiers, *key)),
                    _ => None,
                });
                // Nothing else (the search box, list keys) sees these keys.
                i.events.retain(|e| !matches!(e, egui::Event::Key { .. } | egui::Event::Text(_)));
                press
            });
            if let Some((m, key)) = press {
                self.assign_shortcut(cmd, slot, commands::from_press(m, key), &ctx);
                *record = None;
            }
        }
        let recording = *record;

        // Header: title, search box.
        super::popups::label(ui, pal, "Keyboard shortcuts", 16.0, Some(pal.text_strong));
        let (sr, _) = ui.allocate_exact_size(vec2(ui.available_width(), 34.0), Sense::hover());
        let (search, _) =
            super::search_box(ui, sr, pal, query, Id::new("shortcuts_search"), "Search commands or keys…", false);
        if recording.is_none() {
            search.request_focus();
        } else {
            search.surrender_focus();
        }
        if search.changed() {
            *cursor = 0;
        }
        ui.add_space(6.0);

        // Commands that match the search, by name or by key ("ctrl+shift").
        let q = query.trim();
        let mut items: Vec<(i32, Command)> = commands::ALL
            .iter()
            .filter(|c| c.ext().is_none_or(|e| self.cfg.ext_on(e)))
            .filter_map(|&c| {
                let keys = c.shortcut_texts(&ctx).join(" ");
                let s = fuzzy::score(q, c.label()).or_else(|| fuzzy::score(q, &keys).map(|s| s - 200))?;
                Some((s, c))
            })
            .collect();
        if !q.is_empty() {
            items.sort_by_key(|a| std::cmp::Reverse(a.0));
        }
        let items: Vec<Command> = items.into_iter().map(|(_, c)| c).collect();
        let basics: Vec<&(&str, &str)> =
            BASICS.iter().filter(|(d, k)| fuzzy::score(q, d).is_some() || fuzzy::score(q, k).is_some()).collect();

        let (mut up, mut down, mut enter) = (false, false, false);
        if recording.is_none() {
            ctx.input_mut(|i| {
                up = i.consume_key(egui::Modifiers::NONE, Key::ArrowUp);
                down = i.consume_key(egui::Modifiers::NONE, Key::ArrowDown);
                enter = i.consume_key(egui::Modifiers::NONE, Key::Enter);
            });
        }
        *cursor = crate::app::step(*cursor, up, down, items.len());
        if enter && let Some(&c) = items.get(*cursor) {
            out.run = Some(c);
        }

        let mut ops = Vec::new();
        let body_h = (ctx.content_rect().height() - 260.0).clamp(220.0, 520.0);
        egui::ScrollArea::vertical().max_height(body_h).min_scrolled_height(body_h).auto_shrink([false, true]).show(
            ui,
            |ui| {
                ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
                if items.is_empty() && basics.is_empty() {
                    ui.add_space(20.0);
                    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 24.0), Sense::hover());
                    text(ui.painter(), r.center(), Align2::CENTER_CENTER, "No matching commands", 13.5, pal.text_faint);
                }
                for (i, &c) in items.iter().enumerate() {
                    let rec = recording.filter(|(rc, _)| *rc == c).map(|(_, s)| s);
                    let r = command_row(ui, pal, c, i == *cursor, rec, &mut ops);
                    if r.clicked() {
                        *cursor = i;
                    }
                    if r.double_clicked() {
                        ops.push(Op::Run(c));
                    }
                    if i == *cursor && (up || down) {
                        r.scroll_to_me(None);
                    }
                }
                if !basics.is_empty() {
                    ui.add_space(12.0);
                    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 22.0), Sense::hover());
                    super::text_bold(
                        ui.painter(),
                        r.left_center() + vec2(8.0, 0.0),
                        Align2::LEFT_CENTER,
                        "ALWAYS",
                        11.0,
                        pal.text_dim,
                    );
                    for (desc, keys) in basics {
                        let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 28.0), Sense::hover());
                        let p = ui.painter();
                        text(p, pos2(r.left() + 10.0, r.center().y), Align2::LEFT_CENTER, desc, 13.5, pal.text_dim);
                        text(p, pos2(r.right() - 10.0, r.center().y), Align2::RIGHT_CENTER, keys, 12.5, pal.text_faint);
                    }
                }
            },
        );

        // Footer: hints, and Reset all when anything was changed.
        ui.add_space(8.0);
        let (fr, _) = ui.allocate_exact_size(vec2(ui.available_width(), 30.0), Sense::hover());
        let hint = if recording.is_some() {
            "Press the new key combination  ·  Esc cancels".to_string()
        } else {
            "↑↓ move  ·  Enter runs  ·  click a key to change it  ·  right-click removes it  ·  + adds one".to_string()
        };
        let hint_color = if recording.is_some() { pal.accent } else { pal.text_faint };
        text(ui.painter(), fr.left_center() + vec2(8.0, 0.0), Align2::LEFT_CENTER, hint, 12.0, hint_color);
        if commands::ALL.iter().any(|c| c.is_customized()) {
            let g = ui.painter().layout_no_wrap("Reset all".into(), font(13.0), pal.text);
            let br = Rect::from_min_max(pos2(fr.right() - g.size().x - 24.0, fr.top()), fr.right_bottom());
            let resp = ui.interact(br, Id::new("shortcuts_reset_all"), Sense::click());
            let p = ui.painter();
            p.rect_filled(br, pal.round(5.0), if resp.hovered() { pal.tab_hover } else { pal.input });
            p.rect_stroke(br, pal.round(5.0), Stroke::new(1.0, pal.border), StrokeKind::Inside);
            p.galley(br.center() - g.size() / 2.0, g, pal.text);
            if resp.on_hover_text("Put every shortcut back to its default").clicked() {
                ops.push(Op::ResetAll);
            }
        }

        for op in ops {
            match op {
                Op::Record(c, slot) => {
                    // Clicking the key being recorded again cancels.
                    *record = if *record == Some((c, slot)) { None } else { Some((c, slot)) };
                }
                Op::Remove(c, i) => {
                    let mut keys = c.shortcuts();
                    if i < keys.len() {
                        keys.remove(i);
                    }
                    self.set_shortcuts(c, keys);
                    *record = None;
                }
                Op::Reset(c) => {
                    self.set_shortcuts(c, c.default_shortcuts());
                    *record = None;
                }
                Op::ResetAll => {
                    self.cfg.keymap.clear();
                    commands::set_keymap(&self.cfg.keymap);
                    self.cfg.save();
                    *record = None;
                }
                Op::Run(c) => out.run = Some(c),
            }
        }
        if out.run.is_some() {
            out.close = true;
        }
        if record.is_some() {
            ctx.request_repaint(); // the recording chip pulses
        }
        out
    }
}

/// One command: name on the left; its keys, add, reset and run on the right.
fn command_row(
    ui: &mut Ui,
    pal: &Palette,
    c: Command,
    selected: bool,
    recording: Option<Option<usize>>,
    ops: &mut Vec<Op>,
) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_H), Sense::click());
    let ctx = ui.ctx().clone();
    let p = ui.painter().clone();
    let item = rect.shrink2(vec2(2.0, 1.0));
    if selected {
        p.rect_filled(item, pal.round(5.0), pal.accent_dim);
    } else if resp.hovered() {
        p.rect_filled(item, pal.round(5.0), pal.hover);
    }
    let cy = rect.center().y;
    let id = Id::new(("shortcut_row", c.id()));
    let mut right = item.right() - 6.0;

    // Run.
    let run = Rect::from_min_max(pos2(right - 26.0, cy - 12.0), pos2(right, cy + 12.0));
    let run_resp = ui.interact(run, id.with("run"), Sense::click());
    if run_resp.hovered() {
        p.rect_filled(run, pal.round(5.0), pal.tab_hover);
    }
    let color = if run_resp.hovered() { pal.accent } else { pal.text_dim };
    let t = run.center();
    p.add(egui::Shape::convex_polygon(
        vec![t + vec2(-4.0, -6.0), t + vec2(6.0, 0.0), t + vec2(-4.0, 6.0)],
        color,
        Stroke::NONE,
    ));
    if run_resp.on_hover_text("Run").clicked() {
        ops.push(Op::Run(c));
    }
    right = run.left() - 8.0;

    // Reset (only when changed).
    if c.is_customized() {
        let r = Rect::from_min_max(pos2(right - 24.0, cy - 12.0), pos2(right, cy + 12.0));
        let resp = ui.interact(r, id.with("reset"), Sense::click());
        if resp.hovered() {
            p.rect_filled(r, pal.round(5.0), pal.tab_hover);
        }
        text(&p, r.center(), Align2::CENTER_CENTER, "↺", 15.0, if resp.hovered() { pal.text } else { pal.text_dim });
        let defaults = c.default_shortcuts().iter().map(|s| commands::format_shortcut(&ctx, s)).collect::<Vec<_>>();
        let tip = if defaults.is_empty() {
            "Reset (no shortcut)".into()
        } else {
            format!("Reset to {}", defaults.join(", "))
        };
        if resp.on_hover_text(tip).clicked() {
            ops.push(Op::Reset(c));
        }
        right = r.left() - 6.0;
    }

    // Add a key.
    let add = Rect::from_min_max(pos2(right - 24.0, cy - 11.0), pos2(right, cy + 11.0));
    let add_resp = ui.interact(add, id.with("add"), Sense::click());
    let adding = recording == Some(None);
    if add_resp.hovered() || adding {
        p.rect_filled(add, pal.round(5.0), pal.tab_hover);
    }
    crate::icons::plus(&p, add.shrink(5.0), if add_resp.hovered() { pal.text } else { pal.text_dim });
    if add_resp.on_hover_text("Add a shortcut").clicked() {
        ops.push(Op::Record(c, None));
    }
    right = add.left() - 8.0;

    // The keys, right to left; a new one being recorded comes last.
    let keys: Vec<String> = c.shortcut_texts(&ctx);
    let mut chips: Vec<(Option<usize>, String)> = keys.into_iter().enumerate().map(|(i, k)| (Some(i), k)).collect();
    if adding {
        chips.push((None, String::new()));
    }
    let pulse = (ctx.input(|i| i.time) * 4.0).sin() as f32 * 0.5 + 0.5;
    let label_right = {
        let mut r = right;
        for (slot, k) in chips.iter().rev() {
            let rec = recording == Some(*slot);
            let shown = if rec { "Press keys…" } else { k.as_str() };
            let g = p.layout_no_wrap(shown.to_string(), font(12.0), pal.text);
            let w = g.size().x + 16.0;
            let chip = Rect::from_min_max(pos2(r - w, cy - 11.0), pos2(r, cy + 11.0));
            let cresp = ui.interact(chip, id.with(("key", *slot)), Sense::click());
            let (fill, stroke, fg) = if rec {
                (pal.accent.gamma_multiply(0.15 + 0.15 * pulse), Stroke::new(1.5, pal.accent), pal.text_strong)
            } else if cresp.hovered() {
                (pal.tab_hover, Stroke::new(1.0, pal.text_dim), pal.text_strong)
            } else {
                (Color32::TRANSPARENT, Stroke::new(1.0, pal.border), pal.text_dim)
            };
            p.rect_filled(chip, pal.round(4.0), fill);
            p.rect_stroke(chip, pal.round(4.0), stroke, StrokeKind::Inside);
            text(&p, chip.center(), Align2::CENTER_CENTER, shown, 12.0, fg);
            if let Some(i) = slot {
                if cresp.secondary_clicked() {
                    ops.push(Op::Remove(c, *i));
                }
                if cresp.on_hover_text("Click to change, right-click to remove").clicked() {
                    ops.push(Op::Record(c, Some(*i)));
                }
            }
            r = chip.left() - 6.0;
        }
        r
    };

    // The name, and a dot when the keys were changed.
    let x = item.left() + 10.0;
    let name_color = if selected { pal.text_strong } else { pal.text };
    let g = elided(&p, c.label(), 13.5, name_color, label_right - x - 16.0);
    let gw = g.size().x;
    p.galley(pos2(x, cy - g.size().y / 2.0), g, name_color);
    if c.is_customized() {
        p.circle_filled(pos2(x + gw + 8.0, cy), 3.0, pal.accent);
    }
    resp
}
