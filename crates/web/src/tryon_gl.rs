//! WebGL compositor for the live try-on.
//!
//! Three passes per frame, back to front:
//!   1. the camera frame (mirrored for the selfie camera),
//!   2. the garment artwork warped onto the body as a triangle mesh,
//!   3. the camera frame again, but only where a forearm crosses the torso
//!      and the segmentation mask says "person", so arms and hands appear
//!      in front of the garment.
//!
//! WebGL 1 is used on purpose: it is available on every phone that can run
//! the pose model, and nothing here needs WebGL 2.

use design::tryon::{BodyPose, Mesh};
use wasm_bindgen::{JsCast, JsValue};
use web_sys::{HtmlCanvasElement, HtmlVideoElement, WebGlBuffer, WebGlProgram, WebGlRenderingContext as Gl, WebGlShader, WebGlTexture, WebGlUniformLocation};

const VS: &str = r#"
attribute vec2 a_pos;   // pixels, origin top-left
attribute vec2 a_uv;
uniform vec2 u_res;
varying vec2 v_uv;
varying vec2 v_px;
void main() {
    v_uv = a_uv;
    v_px = a_pos;
    vec2 clip = a_pos / u_res * 2.0 - 1.0;
    gl_Position = vec4(clip.x, -clip.y, 0.0, 1.0);
}"#;

const FS_TEXTURE: &str = r#"
precision mediump float;
uniform sampler2D u_tex;
varying vec2 v_uv;
void main() {
    gl_FragColor = texture2D(u_tex, v_uv);
}"#;

// Distance from point p to segment a-b, used to draw "capsules" around the
// forearms.
const FS_ARMS: &str = r#"
precision mediump float;
uniform sampler2D u_tex;    // camera frame
uniform sampler2D u_mask;   // person probability
uniform vec4 u_arm0;        // elbow.xy, wrist.xy in pixels
uniform vec4 u_arm1;
uniform vec2 u_on;          // 1.0 where that arm is in front
uniform float u_radius;
uniform float u_mirror;
varying vec2 v_uv;
varying vec2 v_px;
float capsule(vec2 p, vec4 arm) {
    vec2 a = arm.xy;
    vec2 b = arm.zw + (arm.zw - arm.xy) * 0.35;   // extend past the wrist to cover the hand
    vec2 ab = b - a;
    float t = clamp(dot(p - a, ab) / max(dot(ab, ab), 0.0001), 0.0, 1.0);
    float d = length(p - (a + ab * t));
    return 1.0 - smoothstep(u_radius * 0.75, u_radius, d);
}
void main() {
    float cover = max(capsule(v_px, u_arm0) * u_on.x, capsule(v_px, u_arm1) * u_on.y);
    // The mask is in camera space, which is not mirrored.
    vec2 muv = vec2(mix(v_uv.x, 1.0 - v_uv.x, u_mirror), v_uv.y);
    float person = smoothstep(0.35, 0.65, texture2D(u_mask, muv).r);
    float a = cover * person;
    vec4 c = texture2D(u_tex, v_uv);
    gl_FragColor = vec4(c.rgb * a, a);   // premultiplied
}"#;

struct Program {
    program: WebGlProgram,
    a_pos: u32,
    a_uv: u32,
    u_res: Option<WebGlUniformLocation>,
}

pub struct Renderer {
    gl: Gl,
    tex_prog: Program,
    arm_prog: Program,
    pos_buf: WebGlBuffer,
    uv_buf: WebGlBuffer,
    idx_buf: WebGlBuffer,
    video_tex: WebGlTexture,
    garment_tex: WebGlTexture,
    mask_tex: WebGlTexture,
    has_garment: bool,
    has_mask: bool,
}

fn compile(gl: &Gl, kind: u32, src: &str) -> Result<WebGlShader, String> {
    let shader = gl.create_shader(kind).ok_or("cannot create shader")?;
    gl.shader_source(&shader, src);
    gl.compile_shader(&shader);
    if gl.get_shader_parameter(&shader, Gl::COMPILE_STATUS).as_bool().unwrap_or(false) {
        Ok(shader)
    } else {
        Err(gl.get_shader_info_log(&shader).unwrap_or_default())
    }
}

fn link(gl: &Gl, fs: &str) -> Result<Program, String> {
    let program = gl.create_program().ok_or("cannot create program")?;
    gl.attach_shader(&program, &compile(gl, Gl::VERTEX_SHADER, VS)?);
    gl.attach_shader(&program, &compile(gl, Gl::FRAGMENT_SHADER, fs)?);
    gl.link_program(&program);
    if !gl.get_program_parameter(&program, Gl::LINK_STATUS).as_bool().unwrap_or(false) {
        return Err(gl.get_program_info_log(&program).unwrap_or_default());
    }
    Ok(Program {
        a_pos: gl.get_attrib_location(&program, "a_pos") as u32,
        a_uv: gl.get_attrib_location(&program, "a_uv") as u32,
        u_res: gl.get_uniform_location(&program, "u_res"),
        program,
    })
}

fn texture(gl: &Gl) -> Result<WebGlTexture, String> {
    let t = gl.create_texture().ok_or("cannot create texture")?;
    gl.bind_texture(Gl::TEXTURE_2D, Some(&t));
    // Clamp + linear works for non-power-of-two sizes in WebGL 1.
    for (name, value) in [
        (Gl::TEXTURE_WRAP_S, Gl::CLAMP_TO_EDGE),
        (Gl::TEXTURE_WRAP_T, Gl::CLAMP_TO_EDGE),
        (Gl::TEXTURE_MIN_FILTER, Gl::LINEAR),
        (Gl::TEXTURE_MAG_FILTER, Gl::LINEAR),
    ] {
        gl.tex_parameteri(Gl::TEXTURE_2D, name, value as i32);
    }
    Ok(t)
}

impl Renderer {
    pub fn new(canvas: &HtmlCanvasElement) -> Result<Renderer, String> {
        // `preserveDrawingBuffer` keeps the last frame readable, which the
        // snapshot button needs.
        let opts = js_sys::Object::new();
        for (k, v) in [("preserveDrawingBuffer", true), ("alpha", false), ("antialias", false), ("premultipliedAlpha", true)] {
            let _ = js_sys::Reflect::set(&opts, &JsValue::from_str(k), &JsValue::from_bool(v));
        }
        let gl: Gl = canvas
            .get_context_with_context_options("webgl", &opts)
            .ok()
            .flatten()
            .and_then(|c| c.dyn_into().ok())
            .ok_or("WebGL is not available")?;
        let buffer = |gl: &Gl| gl.create_buffer().ok_or("cannot create buffer");
        Ok(Renderer {
            tex_prog: link(&gl, FS_TEXTURE)?,
            arm_prog: link(&gl, FS_ARMS)?,
            pos_buf: buffer(&gl)?,
            uv_buf: buffer(&gl)?,
            idx_buf: buffer(&gl)?,
            video_tex: texture(&gl)?,
            garment_tex: texture(&gl)?,
            mask_tex: texture(&gl)?,
            has_garment: false,
            has_mask: false,
            gl,
        })
    }

    /// Upload the garment artwork (drawn on a 2D canvas) as a texture.
    pub fn set_garment(&mut self, source: &HtmlCanvasElement) {
        let gl = &self.gl;
        gl.bind_texture(Gl::TEXTURE_2D, Some(&self.garment_tex));
        // Canvas pixels are straight alpha; ask for premultiplied on upload
        // so blending with (ONE, ONE_MINUS_SRC_ALPHA) has no dark fringes.
        gl.pixel_storei(Gl::UNPACK_PREMULTIPLY_ALPHA_WEBGL, 1);
        let ok = gl.tex_image_2d_with_u32_and_u32_and_canvas(Gl::TEXTURE_2D, 0, Gl::RGBA as i32, Gl::RGBA, Gl::UNSIGNED_BYTE, source).is_ok();
        gl.pixel_storei(Gl::UNPACK_PREMULTIPLY_ALPHA_WEBGL, 0);
        self.has_garment = ok;
    }

    /// Upload the person mask (one byte per pixel, row 0 at the top).
    pub fn set_mask(&mut self, data: &[u8], width: u32, height: u32) {
        if width == 0 || height == 0 || data.len() < (width * height) as usize {
            self.has_mask = false;
            return;
        }
        let gl = &self.gl;
        gl.bind_texture(Gl::TEXTURE_2D, Some(&self.mask_tex));
        gl.pixel_storei(Gl::UNPACK_ALIGNMENT, 1); // rows are not 4-byte aligned
        self.has_mask = gl
            .tex_image_2d_with_i32_and_i32_and_i32_and_format_and_type_and_opt_u8_array(
                Gl::TEXTURE_2D,
                0,
                Gl::LUMINANCE as i32,
                width as i32,
                height as i32,
                0,
                Gl::LUMINANCE,
                Gl::UNSIGNED_BYTE,
                Some(data),
            )
            .is_ok();
        gl.pixel_storei(Gl::UNPACK_ALIGNMENT, 4);
    }

    fn upload(&self, buf: &WebGlBuffer, data: &[f32]) {
        // `Float32Array::from` copies into a JS-owned array; a view into
        // WASM memory could dangle if the heap grows during the call.
        let arr = js_sys::Float32Array::from(data);
        self.gl.bind_buffer(Gl::ARRAY_BUFFER, Some(buf));
        self.gl.buffer_data_with_array_buffer_view(Gl::ARRAY_BUFFER, &arr, Gl::DYNAMIC_DRAW);
    }

    fn draw_triangles(&self, prog: &Program, w: f32, h: f32, positions: &[f32], uvs: &[f32], indices: &[u16]) {
        let gl = &self.gl;
        gl.uniform2f(prog.u_res.as_ref(), w, h);
        self.upload(&self.pos_buf, positions);
        gl.enable_vertex_attrib_array(prog.a_pos);
        gl.vertex_attrib_pointer_with_i32(prog.a_pos, 2, Gl::FLOAT, false, 0, 0);
        self.upload(&self.uv_buf, uvs);
        gl.enable_vertex_attrib_array(prog.a_uv);
        gl.vertex_attrib_pointer_with_i32(prog.a_uv, 2, Gl::FLOAT, false, 0, 0);
        let idx = js_sys::Uint16Array::from(indices);
        gl.bind_buffer(Gl::ELEMENT_ARRAY_BUFFER, Some(&self.idx_buf));
        gl.buffer_data_with_array_buffer_view(Gl::ELEMENT_ARRAY_BUFFER, &idx, Gl::DYNAMIC_DRAW);
        gl.draw_elements_with_i32(Gl::TRIANGLES, indices.len() as i32, Gl::UNSIGNED_SHORT, 0);
    }

    /// Draw one frame. The canvas backing store must already be `w` x `h`
    /// (the video's size). `garment` is the warped mesh in the same pixels.
    pub fn draw(&mut self, video: &HtmlVideoElement, w: f32, h: f32, mirrored: bool, garment: Option<(&Mesh, &BodyPose)>) {
        let gl = &self.gl;
        gl.viewport(0, 0, w as i32, h as i32);
        gl.active_texture(Gl::TEXTURE0);
        gl.bind_texture(Gl::TEXTURE_2D, Some(&self.video_tex));
        if gl.tex_image_2d_with_u32_and_u32_and_video(Gl::TEXTURE_2D, 0, Gl::RGBA as i32, Gl::RGBA, Gl::UNSIGNED_BYTE, video).is_err() {
            return;
        }

        // Full-frame quad; the selfie view flips the texture horizontally.
        let quad_pos = [0.0, 0.0, w, 0.0, w, h, 0.0, h];
        let (u0, u1) = if mirrored { (1.0, 0.0) } else { (0.0, 1.0) };
        let quad_uv = [u0, 0.0, u1, 0.0, u1, 1.0, u0, 1.0];
        let quad_idx = [0u16, 1, 2, 0, 2, 3];

        // Pass 1: camera.
        gl.disable(Gl::BLEND);
        gl.use_program(Some(&self.tex_prog.program));
        self.draw_triangles(&self.tex_prog, w, h, &quad_pos, &quad_uv, &quad_idx);

        let Some((mesh, pose)) = garment.filter(|_| self.has_garment) else { return };

        // Pass 2: garment mesh.
        gl.enable(Gl::BLEND);
        gl.blend_func(Gl::ONE, Gl::ONE_MINUS_SRC_ALPHA);
        gl.bind_texture(Gl::TEXTURE_2D, Some(&self.garment_tex));
        self.draw_triangles(&self.tex_prog, w, h, &mesh.positions, &mesh.uvs, &mesh.indices);

        // Pass 3: forearms in front, where there are any.
        if self.has_mask && (pose.forearm_front_l || pose.forearm_front_r) {
            let p = &self.arm_prog.program;
            gl.use_program(Some(p));
            gl.active_texture(Gl::TEXTURE1);
            gl.bind_texture(Gl::TEXTURE_2D, Some(&self.mask_tex));
            gl.active_texture(Gl::TEXTURE0);
            gl.bind_texture(Gl::TEXTURE_2D, Some(&self.video_tex));
            let loc = |name: &str| gl.get_uniform_location(p, name);
            gl.uniform1i(loc("u_tex").as_ref(), 0);
            gl.uniform1i(loc("u_mask").as_ref(), 1);
            gl.uniform4f(loc("u_arm0").as_ref(), pose.elbow_l[0], pose.elbow_l[1], pose.wrist_l[0], pose.wrist_l[1]);
            gl.uniform4f(loc("u_arm1").as_ref(), pose.elbow_r[0], pose.elbow_r[1], pose.wrist_r[0], pose.wrist_r[1]);
            gl.uniform2f(loc("u_on").as_ref(), f32::from(pose.forearm_front_l), f32::from(pose.forearm_front_r));
            // Forearm thickness relative to shoulder width.
            let shoulder = ((pose.shoulder_r[0] - pose.shoulder_l[0]).powi(2) + (pose.shoulder_r[1] - pose.shoulder_l[1]).powi(2)).sqrt();
            gl.uniform1f(loc("u_radius").as_ref(), shoulder * 0.2);
            gl.uniform1f(loc("u_mirror").as_ref(), if mirrored { 1.0 } else { 0.0 });
            self.draw_triangles(&self.arm_prog, w, h, &quad_pos, &quad_uv, &quad_idx);
        }
    }
}
