// --- shared helpers ---

fn to_ndc(screen: vec2<f32>, viewport: vec2<f32>) -> vec4<f32> {
    let ndc = vec2<f32>(
        screen.x / viewport.x * 2.0 - 1.0,
        1.0 - screen.y / viewport.y * 2.0,
    );
    return vec4<f32>(ndc, 0.0, 1.0);
}

fn checker(screen: vec2<f32>) -> vec3<f32> {
    let cell = 8.0;
    let cx = i32(floor(screen.x / cell));
    let cy = i32(floor(screen.y / cell));
    if ((cx + cy) & 1) == 0 {
        return vec3<f32>(0.62, 0.62, 0.65);
    }
    return vec3<f32>(0.82, 0.82, 0.84);
}

fn image_to_screen(
    image: vec2<f32>,
    center: vec2<f32>,
    image_size: vec2<f32>,
    zoom: f32,
    rotation: f32,
) -> vec2<f32> {
    let local = (image - image_size * 0.5) * zoom;
    let c = cos(rotation);
    let s = sin(rotation);
    return center + vec2<f32>(local.x * c - local.y * s, local.x * s + local.y * c);
}

fn screen_to_uv(
    screen: vec2<f32>,
    center: vec2<f32>,
    image_size: vec2<f32>,
    zoom: f32,
    rotation: f32,
) -> vec2<f32> {
    let delta = screen - center;
    let c = cos(-rotation);
    let s = sin(-rotation);
    let local = vec2<f32>(delta.x * c - delta.y * s, delta.x * s + delta.y * c) / max(zoom, 1e-6);
    return local / image_size + vec2<f32>(0.5, 0.5);
}

// --- checker background ---

struct ViewUniforms {
    viewport: vec2<f32>,
    center: vec2<f32>,
    image_size: vec2<f32>,
    zoom: f32,
    rotation: f32,
    opacity: f32,
    _pad: f32,
}

@group(0) @binding(0) var<uniform> vu: ViewUniforms;
@group(0) @binding(1) var img: texture_2d<f32>;
@group(0) @binding(2) var samp_near: sampler;

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) screen: vec2<f32>,
}

@vertex
fn vs_image(@builtin(vertex_index) idx: u32) -> VsOut {
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(0.0, 0.0),
        vec2<f32>(1.0, 0.0),
        vec2<f32>(1.0, 1.0),
        vec2<f32>(0.0, 0.0),
        vec2<f32>(1.0, 1.0),
        vec2<f32>(0.0, 1.0),
    );
    let uv = corners[idx];
    let screen = image_to_screen(uv * vu.image_size, vu.center, vu.image_size, vu.zoom, vu.rotation);
    var out: VsOut;
    out.pos = to_ndc(screen, vu.viewport);
    out.uv = uv;
    out.screen = screen;
    return out;
}

@fragment
fn fs_checker(in: VsOut) -> @location(0) vec4<f32> {
    return vec4<f32>(checker(in.screen), 1.0);
}

@fragment
fn fs_layer(in: VsOut) -> @location(0) vec4<f32> {
    let tex = textureSampleLevel(img, samp_near, in.uv, 0.0);
    let a = tex.a * vu.opacity;
    return vec4<f32>(tex.rgb, a);
}

// Active layer during transform: same pixels, but the grabbed mask is hidden
// so it can be redrawn as the floating quad. Color stays in this texture.

struct PunchUniforms {
    viewport: vec2<f32>,
    center: vec2<f32>,
    image_size: vec2<f32>,
    zoom: f32,
    rotation: f32,
    opacity: f32,
    _pad: f32,
    origin: vec2<f32>,
    size: vec2<f32>,
}

@group(0) @binding(0) var<uniform> pu: PunchUniforms;
@group(0) @binding(1) var punch_img: texture_2d<f32>;
@group(0) @binding(2) var punch_samp: sampler;
@group(0) @binding(3) var punch_mask: texture_2d<f32>;

@vertex
fn vs_punch(@builtin(vertex_index) idx: u32) -> VsOut {
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(0.0, 0.0),
        vec2<f32>(1.0, 0.0),
        vec2<f32>(1.0, 1.0),
        vec2<f32>(0.0, 0.0),
        vec2<f32>(1.0, 1.0),
        vec2<f32>(0.0, 1.0),
    );
    let uv = corners[idx];
    let screen = image_to_screen(uv * pu.image_size, pu.center, pu.image_size, pu.zoom, pu.rotation);
    var out: VsOut;
    out.pos = to_ndc(screen, pu.viewport);
    out.uv = uv;
    out.screen = screen;
    return out;
}

@fragment
fn fs_punch(in: VsOut) -> @location(0) vec4<f32> {
    let tex = textureSampleLevel(punch_img, punch_samp, in.uv, 0.0);
    let image = in.uv * pu.image_size;
    let local = image - pu.origin;
    if (local.x >= 0.0 && local.y >= 0.0 && local.x < pu.size.x && local.y < pu.size.y) {
        let mc = vec2<i32>(floor(clamp(local, vec2<f32>(0.0), pu.size - vec2<f32>(1.0))));
        if (textureLoad(punch_mask, mc, 0).r > 0.5) {
            return vec4<f32>(0.0);
        }
    }
    return vec4<f32>(tex.rgb, tex.a * pu.opacity);
}

// --- floating transform: same layer texture, placed on the CPU corner quad ---

struct FloatUniforms {
    viewport: vec2<f32>,
    opacity: f32,
    _pad: f32,
    origin: vec2<f32>,
    size: vec2<f32>,
    image_size: vec2<f32>,
    c0: vec2<f32>,
    c1: vec2<f32>,
    c2: vec2<f32>,
    c3: vec2<f32>,
}

@group(0) @binding(0) var<uniform> fu: FloatUniforms;
@group(0) @binding(1) var fimg: texture_2d<f32>;
@group(0) @binding(2) var fsamp: sampler;
@group(0) @binding(3) var fmask: texture_2d<f32>;

@vertex
fn vs_float(@builtin(vertex_index) idx: u32) -> VsOut {
    var corner_uv = array<vec2<f32>, 6>(
        vec2<f32>(0.0, 0.0),
        vec2<f32>(1.0, 0.0),
        vec2<f32>(1.0, 1.0),
        vec2<f32>(0.0, 0.0),
        vec2<f32>(1.0, 1.0),
        vec2<f32>(0.0, 1.0),
    );
    var corner_i = array<u32, 6>(0u, 1u, 2u, 0u, 2u, 3u);
    var corners = array<vec2<f32>, 4>(fu.c0, fu.c1, fu.c2, fu.c3);
    let uv = corner_uv[idx];
    let screen = corners[corner_i[idx]];
    var out: VsOut;
    out.pos = to_ndc(screen, fu.viewport);
    out.uv = uv;
    out.screen = screen;
    return out;
}

@fragment
fn fs_float(in: VsOut) -> @location(0) vec4<f32> {
    let texel = clamp(floor(in.uv * fu.size), vec2<f32>(0.0), fu.size - vec2<f32>(1.0));
    let m = textureLoad(fmask, vec2<i32>(texel), 0).r;
    if (m < 0.5) {
        return vec4<f32>(0.0);
    }
    let src_uv = (fu.origin + texel + vec2<f32>(0.5)) / fu.image_size;
    let tex = textureSampleLevel(fimg, fsamp, src_uv, 0.0);
    return vec4<f32>(tex.rgb, tex.a * fu.opacity);
}

// --- selection overlay (dim + marching ants) ---

struct SelUniforms {
    viewport: vec2<f32>,
    center: vec2<f32>,
    image_size: vec2<f32>,
    zoom: f32,
    rotation: f32,
    has_selection: f32,
    dim_outside: f32,
    time: f32,
    _pad: f32,
}

@group(0) @binding(0) var<uniform> su: SelUniforms;
@group(0) @binding(1) var sel: texture_2d<f32>;
@group(0) @binding(2) var samp_linear: sampler;

struct SelOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) screen: vec2<f32>,
}

@vertex
fn vs_sel(@builtin(vertex_index) idx: u32) -> SelOut {
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(0.0, 0.0),
        vec2<f32>(1.0, 0.0),
        vec2<f32>(1.0, 1.0),
        vec2<f32>(0.0, 0.0),
        vec2<f32>(1.0, 1.0),
        vec2<f32>(0.0, 1.0),
    );
    let uv = corners[idx];
    let screen = image_to_screen(uv * su.image_size, su.center, su.image_size, su.zoom, su.rotation);
    var out: SelOut;
    out.pos = to_ndc(screen, su.viewport);
    out.uv = uv;
    out.screen = screen;
    return out;
}

@fragment
fn fs_sel(in: SelOut) -> @location(0) vec4<f32> {
    if (su.has_selection < 0.5) {
        return vec4<f32>(0.0);
    }
    let m = textureSampleLevel(sel, samp_linear, in.uv, 0.0).r;
    let fw = max(fwidth(m), 1e-4);
    // Marching ants on contour.
    if (abs(m - 0.5) < fw * 0.7) {
        let period = 8.0;
        let phase = in.screen.x + in.screen.y - su.time * 48.0;
        let on = fract(phase / period) < 0.5;
        let rgb = select(vec3<f32>(0.0, 0.0, 0.0), vec3<f32>(1.0, 1.0, 1.0), on);
        return vec4<f32>(rgb, 1.0);
    }
    // Dim everything outside the selection.
    if (su.dim_outside > 0.5 && m < 0.5) {
        return vec4<f32>(0.0, 0.0, 0.0, 0.45);
    }
    return vec4<f32>(0.0);
}

// --- solid screen-space quads for overlays ---

struct SolidUniforms {
    viewport: vec2<f32>,
    _pad: vec2<f32>,
}

@group(0) @binding(0) var<uniform> solid_u: SolidUniforms;

struct SolidIn {
    @location(0) pos: vec2<f32>,
    @location(1) color: vec4<f32>,
}

struct SolidOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) color: vec4<f32>,
}

@vertex
fn vs_solid(input: SolidIn) -> SolidOut {
    var out: SolidOut;
    out.pos = vec4<f32>(
        input.pos.x / solid_u.viewport.x * 2.0 - 1.0,
        1.0 - input.pos.y / solid_u.viewport.y * 2.0,
        0.0,
        1.0,
    );
    out.color = input.color;
    return out;
}

@fragment
fn fs_solid(in: SolidOut) -> @location(0) vec4<f32> {
    return in.color;
}
