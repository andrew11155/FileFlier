//! Drag-and-drop helpers: spring-loaded folders and scrolling while dragging.

use egui::{Context, Id, Rect, Ui, vec2};

use crate::app::DragPaths;

/// How long a drag has to rest on a folder (or tab) before it opens.
pub const SPRING_DELAY: f64 = 0.7;

/// Spring-loaded targets: call every frame while a drag hovers the target `id`.
/// Returns how far the timer has run (0..1) and `true` on the one frame it fires.
/// Moving to another target restarts the timer; a target fires once per hover.
pub fn spring(ctx: &Context, id: Id, delay: f64) -> (f32, bool) {
    let key = Id::new("dnd_spring");
    let now = ctx.input(|i| i.time);
    let frame = ctx.cumulative_frame_nr();
    let (prev, start, last, fired) =
        ctx.data(|d| d.get_temp::<(Id, f64, u64, bool)>(key)).unwrap_or((Id::NULL, now, 0, false));
    let same = prev == id && last + 1 >= frame;
    let (start, fired) = if same { (start, fired) } else { (now, false) };
    let t = ((now - start) / delay).clamp(0.0, 1.0) as f32;
    let fire = !fired && t >= 1.0;
    ctx.data_mut(|d| d.insert_temp(key, (id, start, frame, fired || fire)));
    if !fired && !fire {
        // Keep frames coming while the pointer rests, so the timer can run out.
        ctx.request_repaint();
    }
    (if fired { 0.0 } else { t }, fire)
}

/// Scrolling while files are dragged: the mouse wheel (egui ignores it while any
/// widget is being dragged), and automatic scrolling near the top or bottom edge of
/// `rect` (or just past it). Call inside the scroll area's contents.
pub fn drag_scroll(ui: &Ui, rect: Rect) {
    let ctx = ui.ctx();
    if !egui::DragAndDrop::has_payload_of_type::<DragPaths>(ctx) {
        return;
    }
    let Some(pos) = ctx.pointer_hover_pos() else { return };
    if !rect.x_range().contains(pos.x) {
        return;
    }
    let none = egui::style::ScrollAnimation::none();
    if rect.contains(pos) && ctx.dragged_id().is_some() {
        let wheel = ctx.input(|i| i.smooth_scroll_delta.y);
        if wheel != 0.0 {
            ui.scroll_with_delta_animation(vec2(0.0, wheel), none);
        }
    }
    let zone = (rect.height() * 0.15).clamp(18.0, 48.0);
    // Positive = towards the top; speed grows the deeper into the zone (or past the edge).
    let depth = if pos.y < rect.top() + zone && pos.y > rect.top() - 60.0 {
        (rect.top() + zone - pos.y) / zone
    } else if pos.y > rect.bottom() - zone && pos.y < rect.bottom() + 60.0 {
        -(pos.y - (rect.bottom() - zone)) / zone
    } else {
        return;
    };
    let speed = 900.0 * depth.clamp(-1.6, 1.6);
    let dt = ctx.input(|i| i.stable_dt).min(0.05);
    ui.scroll_with_delta_animation(vec2(0.0, speed * dt), none);
    ctx.request_repaint();
}
