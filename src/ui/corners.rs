//! Rounded window corners, as GNOME draws its windows.
//!
//! The window is ours (no OS frame), so its corners would be square. With a transparent
//! window, a last paint step multiplies each corner by a smooth round mask, so the
//! desktop shows through outside the curve. It runs on the GPU after everything else
//! is drawn, whatever reaches the corners.

use std::sync::{Arc, Mutex};

use eframe::egui_glow;
use eframe::glow::{self, HasContext};

/// The compiled mask program and the (empty) vertex array it draws with.
static PROGRAM: Mutex<Option<Option<(glow::Program, glow::VertexArray)>>> = Mutex::new(None);

/// Cuts the window's corners round, `radius` points across. Call last in the frame.
pub fn round(ctx: &egui::Context, radius: f32) {
    let rect = ctx.content_rect();
    let callback = egui_glow::CallbackFn::new(move |info, painter| {
        let gl = painter.gl();
        let Ok(mut g) = PROGRAM.lock() else { return };
        let Some((program, vao)) = *g.get_or_insert_with(|| unsafe { compile(gl) }) else { return };
        let vp = info.viewport_in_pixels();
        unsafe {
            gl.use_program(Some(program));
            gl.uniform_2_f32(
                gl.get_uniform_location(program, "u_size").as_ref(),
                vp.width_px as f32,
                vp.height_px as f32,
            );
            gl.uniform_2_f32(
                gl.get_uniform_location(program, "u_origin").as_ref(),
                vp.left_px as f32,
                vp.from_bottom_px as f32,
            );
            gl.uniform_1_f32(gl.get_uniform_location(program, "u_radius").as_ref(), radius * info.pixels_per_point);
            gl.disable(glow::SCISSOR_TEST);
            // Multiply what's drawn (colors and alpha, premultiplied) by the mask.
            gl.blend_func_separate(glow::ZERO, glow::SRC_ALPHA, glow::ZERO, glow::SRC_ALPHA);
            gl.bind_vertex_array(Some(vao));
            gl.draw_arrays(glow::TRIANGLES, 0, 3);
            gl.bind_vertex_array(None);
            gl.use_program(None);
        }
    });
    let layer = egui::LayerId::new(egui::Order::Debug, egui::Id::new("window_corners"));
    ctx.layer_painter(layer).add(egui::PaintCallback { rect, callback: Arc::new(callback) });
}

/// Compiles the mask shader. None if this GL can't run it (the corners stay square).
unsafe fn compile(gl: &glow::Context) -> Option<(glow::Program, glow::VertexArray)> {
    let version = egui_glow::ShaderVersion::get(gl);
    if !version.is_new_shader_interface() {
        return None;
    }
    let header = version.version_declaration();
    // One triangle that covers the whole viewport.
    let vertex = format!(
        "{header}\nvoid main() {{\n    vec2 p = vec2(float((gl_VertexID << 1) & 2), float(gl_VertexID & 2));\n    gl_Position = vec4(p * 2.0 - 1.0, 0.0, 1.0);\n}}\n"
    );
    let fragment = format!(
        "{header}\nprecision highp float;\nuniform vec2 u_size;\nuniform vec2 u_origin;\nuniform float u_radius;\nout vec4 out_color;\nvoid main() {{\n    vec2 p = gl_FragCoord.xy - u_origin;\n    vec2 q = min(p, u_size - p);\n    float r = u_radius;\n    float cover = 1.0;\n    if (q.x < r && q.y < r) {{\n        cover = clamp(r - length(vec2(r) - q) + 0.5, 0.0, 1.0);\n    }}\n    out_color = vec4(cover);\n}}\n"
    );
    unsafe {
        let program = gl.create_program().ok()?;
        let mut shaders = Vec::new();
        for (kind, src) in [(glow::VERTEX_SHADER, vertex), (glow::FRAGMENT_SHADER, fragment)] {
            let s = gl.create_shader(kind).ok()?;
            gl.shader_source(s, &src);
            gl.compile_shader(s);
            if !gl.get_shader_compile_status(s) {
                eprintln!("window corners: {}", gl.get_shader_info_log(s));
                return None;
            }
            gl.attach_shader(program, s);
            shaders.push(s);
        }
        gl.link_program(program);
        for s in shaders {
            gl.detach_shader(program, s);
            gl.delete_shader(s);
        }
        if !gl.get_program_link_status(program) {
            eprintln!("window corners: {}", gl.get_program_info_log(program));
            return None;
        }
        let vao = gl.create_vertex_array().ok()?;
        Some((program, vao))
    }
}
