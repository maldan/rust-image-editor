struct Uniforms {
    viewport: vec2<f32>,
    center: vec2<f32>,
    image_size: vec2<f32>,
    zoom: f32,
    rotation: f32,
    has_selection: f32,
    time: f32,
}

@group(0) @binding(0) var<uniform> u: Uniforms;
@group(0) @binding(1) var img: texture_2d<f32>;
@group(0) @binding(2) var samp_near: sampler;
@group(0) @binding(3) var sel: texture_2d<f32>;
@group(0) @binding(4) var samp_linear: sampler;

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) screen: vec2<f32>,
}

fn to_ndc(screen: vec2<f32>) -> vec4<f32> {
    let ndc = vec2<f32>(
        screen.x / u.viewport.x * 2.0 - 1.0,
        1.0 - screen.y / u.viewport.y * 2.0,
    );
    return vec4<f32>(ndc, 0.0, 1.0);
}

@vertex
fn vs_main(@builtin(vertex_index) idx: u32) -> VsOut {
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(0.0, 0.0),
        vec2<f32>(1.0, 0.0),
        vec2<f32>(1.0, 1.0),
        vec2<f32>(0.0, 0.0),
        vec2<f32>(1.0, 1.0),
        vec2<f32>(0.0, 1.0),
    );
    let uv = corners[idx];
    let local = (uv - vec2<f32>(0.5, 0.5)) * u.image_size * u.zoom;
    let c = cos(u.rotation);
    let s = sin(u.rotation);
    let rotated = vec2<f32>(local.x * c - local.y * s, local.x * s + local.y * c);
    let screen = u.center + rotated;
    var out: VsOut;
    out.pos = to_ndc(screen);
    out.uv = uv;
    out.screen = screen;
    return out;
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

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let tex = textureSampleLevel(img, samp_near, in.uv, 0.0);
    let bg = checker(in.screen);
    var rgb = mix(bg, tex.rgb, tex.a);

    // Krita-style marching ants: ~1 screen-pixel contour, zoom-independent via fwidth.
    if (u.has_selection > 0.5) {
        let m = textureSampleLevel(sel, samp_linear, in.uv, 0.0).r;
        let fw = max(fwidth(m), 1e-4);
        // Thin band around the 0.5 isocontour (screen-space thickness).
        if (abs(m - 0.5) < fw * 0.7) {
            let period = 8.0;
            let phase = in.screen.x + in.screen.y - u.time * 48.0;
            let on = fract(phase / period) < 0.5;
            rgb = select(vec3<f32>(0.0, 0.0, 0.0), vec3<f32>(1.0, 1.0, 1.0), on);
        }
    }

    return vec4<f32>(rgb, 1.0);
}

// --- solid screen-space quads for overlays ---

struct SolidUniforms {
    viewport: vec2<f32>,
    _pad: vec2<f32>,
}

@group(0) @binding(0) var<uniform> su: SolidUniforms;

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
        input.pos.x / su.viewport.x * 2.0 - 1.0,
        1.0 - input.pos.y / su.viewport.y * 2.0,
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
