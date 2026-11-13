//! GPU post-processing for the emulation viewport (glow backend).
//!
//! The emulator frame is uploaded to a private GL texture and drawn as a
//! fullscreen triangle into the viewport callback rect. All presets — the
//! scanline/LCD/CRT overlays and the composite NTSC filters — are fragment
//! shaders, so the CPU does not touch the frame after the PPU produces it.
//!
//! egui_glow restores its own program, viewport and scissor after every
//! callback, so this module only has to leave the frame in a sane state.

use std::sync::{Arc, Mutex};

use eframe::glow;
use eframe::glow::HasContext;
use egui::{PaintCallbackInfo, Rect};

/// Per-frame shader parameters.
#[derive(Debug, Clone, Copy)]
pub struct ShaderParams {
    /// 1 = scanlines, 2 = LCD grid, 3 = CRT, 4 = NTSC budget, 5 = NTSC premium.
    pub preset: u32,
    /// Output brightness multiplier.
    pub brightness: f32,
    /// Luma sharpening amount (0 = off).
    pub sharpness: f32,
    /// Use linear magnification for the source texture.
    pub linear: bool,
}

/// Shared handle stored by the app and captured by paint callbacks.
pub type Handle = Arc<Mutex<GpuShader>>;

/// GPU resources for the viewport post-process.
pub struct GpuShader {
    program: glow::Program,
    vao: glow::VertexArray,
    texture: glow::Texture,
    size: (u32, u32),
    u_uv_min: Option<glow::UniformLocation>,
    u_uv_max: Option<glow::UniformLocation>,
    u_source: Option<glow::UniformLocation>,
    u_brightness: Option<glow::UniformLocation>,
    u_sharpness: Option<glow::UniformLocation>,
    u_preset: Option<glow::UniformLocation>,
    u_tex: Option<glow::UniformLocation>,
    failed: bool,
}

impl GpuShader {
    /// Compiles the program and creates the texture. Returns `None` when the
    /// driver rejects the shaders; the caller then falls back to egui drawing.
    pub fn new(gl: &glow::Context) -> Option<Self> {
        let header = if gl.version().is_embedded {
            "#version 300 es\nprecision highp float;\n"
        } else {
            "#version 330 core\n"
        };
        let vertex = format!("{header}{VERTEX}");
        let fragment = format!("{header}{FRAGMENT}");
        // SAFETY: called on the render thread with the GL context current.
        unsafe {
            let program = gl.create_program().ok()?;
            let vs = compile(gl, glow::VERTEX_SHADER, &vertex)?;
            let fs = compile(gl, glow::FRAGMENT_SHADER, &fragment)?;
            gl.attach_shader(program, vs);
            gl.attach_shader(program, fs);
            gl.link_program(program);
            gl.detach_shader(program, vs);
            gl.detach_shader(program, fs);
            gl.delete_shader(vs);
            gl.delete_shader(fs);
            if !gl.get_program_link_status(program) {
                eprintln!(
                    "unko: GPU shader link failed: {}",
                    gl.get_program_info_log(program)
                );
                gl.delete_program(program);
                return None;
            }
            let vao = gl.create_vertex_array().ok()?;
            let texture = gl.create_texture().ok()?;
            gl.bind_texture(glow::TEXTURE_2D, Some(texture));
            gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_MIN_FILTER,
                glow::LINEAR as i32,
            );
            gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_MAG_FILTER,
                glow::LINEAR as i32,
            );
            gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_WRAP_S,
                glow::CLAMP_TO_EDGE as i32,
            );
            gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_WRAP_T,
                glow::CLAMP_TO_EDGE as i32,
            );
            gl.bind_texture(glow::TEXTURE_2D, None);
            Some(Self {
                program,
                vao,
                texture,
                size: (0, 0),
                u_uv_min: gl.get_uniform_location(program, "u_uv_min"),
                u_uv_max: gl.get_uniform_location(program, "u_uv_max"),
                u_source: gl.get_uniform_location(program, "u_source"),
                u_brightness: gl.get_uniform_location(program, "u_brightness"),
                u_sharpness: gl.get_uniform_location(program, "u_sharpness"),
                u_preset: gl.get_uniform_location(program, "u_preset"),
                u_tex: gl.get_uniform_location(program, "u_tex"),
                failed: false,
            })
        }
    }

    /// Queues the shader draw for this frame.
    #[allow(clippy::too_many_arguments)]
    pub fn queue(
        handle: &Handle,
        ui: &egui::Ui,
        target: Rect,
        uv: Rect,
        params: ShaderParams,
        pixels: Vec<u8>,
        width: u32,
        height: u32,
    ) {
        let shader = Arc::clone(handle);
        let callback = eframe::egui_glow::CallbackFn::new(move |info, painter| {
            if let Ok(mut shader) = shader.lock() {
                // SAFETY: callback runs on the render thread.
                unsafe {
                    shader.draw(
                        info,
                        painter.gl(),
                        target,
                        uv,
                        params,
                        &pixels,
                        width,
                        height,
                    );
                }
            }
        });
        ui.painter().add(egui::PaintCallback {
            rect: target,
            callback: Arc::new(callback),
        });
    }

    #[allow(clippy::too_many_arguments)]
    unsafe fn draw(
        &mut self,
        info: PaintCallbackInfo,
        gl: &glow::Context,
        _target: Rect,
        uv: Rect,
        params: ShaderParams,
        pixels: &[u8],
        width: u32,
        height: u32,
    ) {
        if self.failed || width == 0 || height == 0 {
            return;
        }
        gl.bind_texture(glow::TEXTURE_2D, Some(self.texture));
        gl.pixel_store_i32(glow::UNPACK_ALIGNMENT, 1);
        let filter = if params.linear {
            glow::LINEAR
        } else {
            glow::NEAREST
        };
        gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MIN_FILTER, filter as i32);
        gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MAG_FILTER, filter as i32);
        if self.size == (width, height) {
            gl.tex_sub_image_2d(
                glow::TEXTURE_2D,
                0,
                0,
                0,
                width as i32,
                height as i32,
                glow::RGBA,
                glow::UNSIGNED_BYTE,
                glow::PixelUnpackData::Slice(Some(pixels)),
            );
        } else {
            gl.tex_image_2d(
                glow::TEXTURE_2D,
                0,
                glow::RGBA8 as i32,
                width as i32,
                height as i32,
                0,
                glow::RGBA,
                glow::UNSIGNED_BYTE,
                glow::PixelUnpackData::Slice(Some(pixels)),
            );
            self.size = (width, height);
        }

        gl.use_program(Some(self.program));
        // Texture row 0 is the top of the source frame; the fullscreen
        // triangle's bottom edge (p.y = 0) samples the bottom source row.
        gl.uniform_2_f32(self.u_uv_min.as_ref(), uv.left(), uv.bottom());
        gl.uniform_2_f32(self.u_uv_max.as_ref(), uv.right(), uv.top());
        gl.uniform_2_f32(self.u_source.as_ref(), width as f32, height as f32);
        gl.uniform_1_f32(
            self.u_brightness.as_ref(),
            params.brightness.clamp(0.25, 2.0),
        );
        gl.uniform_1_f32(self.u_sharpness.as_ref(), params.sharpness.clamp(0.0, 1.0));
        gl.uniform_1_i32(self.u_preset.as_ref(), params.preset as i32);
        gl.uniform_1_i32(self.u_tex.as_ref(), 0);
        gl.active_texture(glow::TEXTURE0);
        gl.bind_texture(glow::TEXTURE_2D, Some(self.texture));

        let viewport = info.viewport_in_pixels();
        gl.viewport(
            viewport.left_px,
            viewport.from_bottom_px,
            viewport.width_px,
            viewport.height_px,
        );
        let clip = info.clip_rect_in_pixels();
        gl.enable(glow::SCISSOR_TEST);
        gl.scissor(
            clip.left_px,
            clip.from_bottom_px,
            clip.width_px,
            clip.height_px,
        );
        gl.disable(glow::DEPTH_TEST);
        gl.disable(glow::CULL_FACE);
        gl.disable(glow::BLEND);
        gl.bind_vertex_array(Some(self.vao));
        gl.draw_arrays(glow::TRIANGLES, 0, 3);
        gl.bind_vertex_array(None);
        gl.enable(glow::BLEND);
    }
}

impl GpuShader {
    /// Releases the GL objects while the context is still current.
    pub fn destroy(&mut self, gl: &glow::Context) {
        if self.failed {
            return;
        }
        self.failed = true;
        // SAFETY: called from `App::on_exit` with the context current.
        unsafe {
            gl.delete_program(self.program);
            gl.delete_vertex_array(self.vao);
            gl.delete_texture(self.texture);
        }
    }
}

unsafe fn compile(gl: &glow::Context, kind: u32, source: &str) -> Option<glow::Shader> {
    let shader = gl.create_shader(kind).ok()?;
    gl.shader_source(shader, source);
    gl.compile_shader(shader);
    if !gl.get_shader_compile_status(shader) {
        eprintln!(
            "unko: GPU shader compile failed: {}",
            gl.get_shader_info_log(shader)
        );
        gl.delete_shader(shader);
        return None;
    }
    Some(shader)
}

const VERTEX: &str = r#"
uniform vec2 u_uv_min;
uniform vec2 u_uv_max;
out vec2 v_uv;
void main() {
    vec2 p = vec2(float((gl_VertexID << 1) & 2), float(gl_VertexID & 2));
    gl_Position = vec4(p * 2.0 - 1.0, 0.0, 1.0);
    v_uv = mix(u_uv_min, u_uv_max, p);
}
"#;

const FRAGMENT: &str = r#"
uniform sampler2D u_tex;
uniform vec2 u_source;
uniform float u_brightness;
uniform float u_sharpness;
uniform int u_preset;
in vec2 v_uv;
out vec4 frag_color;

vec3 rgb_to_yiq(vec3 c) {
    return vec3(
        dot(c, vec3(0.299, 0.587, 0.114)),
        dot(c, vec3(0.5959, -0.2746, -0.3213)),
        dot(c, vec3(0.2115, -0.5227, 0.3112))
    );
}

vec3 yiq_to_rgb(vec3 c) {
    return vec3(
        c.x + 0.956 * c.y + 0.619 * c.z,
        c.x - 0.272 * c.y - 0.647 * c.z,
        c.x - 1.106 * c.y + 1.703 * c.z
    );
}

// Luma unsharp mask using the two horizontal neighbours.
vec3 sharpen(vec3 color, vec2 uv) {
    if (u_sharpness <= 0.0) {
        return color;
    }
    float luma = rgb_to_yiq(color).x;
    float left = rgb_to_yiq(texture(u_tex, uv - vec2(1.0 / u_source.x, 0.0)).rgb).x;
    float right = rgb_to_yiq(texture(u_tex, uv + vec2(1.0 / u_source.x, 0.0)).rgb).x;
    float amount = (luma - 0.5 * (left + right)) * u_sharpness;
    return clamp(color + vec3(amount), 0.0, 1.0);
}

vec3 ntsc(vec2 uv, int taps, float spread) {
    vec3 yiq = rgb_to_yiq(texture(u_tex, uv).rgb);
    vec2 chroma = vec2(0.0);
    float weight_sum = 0.0;
    for (int i = -taps; i <= taps; i++) {
        float weight = 1.0 - abs(float(i)) / float(taps + 1);
        vec2 offset = vec2(float(i) * spread / u_source.x, 0.0);
        vec3 sample_yiq = rgb_to_yiq(texture(u_tex, uv + offset).rgb);
        chroma += sample_yiq.yz * weight;
        weight_sum += weight;
    }
    yiq.yz = chroma / weight_sum;
    vec3 color = yiq_to_rgb(yiq);
    // Composite video loses some vertical definition; keep it subtle.
    float line = abs(sin(3.14159265 * v_uv.y * u_source.y));
    color *= mix(1.0, 0.84 + 0.16 * line, 0.35);
    return color;
}

void main() {
    vec3 color = texture(u_tex, v_uv).rgb;
    float line = abs(sin(3.14159265 * v_uv.y * u_source.y));
    if (u_preset == 1) {
        color *= mix(0.70, 1.0, line);
    } else if (u_preset == 2) {
        vec2 cell = abs(fract(v_uv * u_source) - 0.5);
        float grid = smoothstep(0.0, 0.09, min(cell.x, cell.y));
        color *= mix(0.62, 1.0, grid);
    } else if (u_preset == 3) {
        color *= mix(0.65, 1.0, line);
        vec2 centered = v_uv - 0.5;
        color *= 1.0 - dot(centered, centered) * 0.55;
    } else if (u_preset == 4) {
        color = ntsc(v_uv, 1, 1.0);
    } else if (u_preset == 5) {
        color = ntsc(v_uv, 3, 1.25);
    }
    color = sharpen(color, v_uv);
    frag_color = vec4(clamp(color * u_brightness, 0.0, 1.0), 1.0);
}
"#;
