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
    hue: f32,
    saturation: f32,
    value: f32,
    _hsv_pad: f32,
    camera_center: vec2<f32>,
    camera_zoom: f32,
    camera_fade: f32,
    camera_enabled: f32,
    _camera_pad: f32,
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
    if vu.camera_enabled > 0.5 { return vec4<f32>(0.0, 0.0, 0.0, 1.0); }
    return vec4<f32>(checker(in.screen), 1.0);
}

@fragment
fn fs_layer(in: VsOut) -> @location(0) vec4<f32> {
    if vu.camera_enabled > 0.5 {
        let uv = screen_to_uv(in.screen, vu.camera_center, vu.image_size, vu.camera_zoom, vu.rotation);
        if any(uv < vec2<f32>(0.0)) || any(uv > vec2<f32>(1.0)) { discard; }
    }
    let tex = textureSampleLevel(img, samp_near, in.uv, 0.0);
    let a = tex.a * vu.opacity;
    if (vu.hue == 0.0 && vu.saturation == 0.0 && vu.value == 0.0) { return vec4<f32>(tex.rgb * (1.0-vu.camera_fade),a); }
    return vec4<f32>(adjust_hsv(tex.rgb) * (1.0-vu.camera_fade),a);
}

fn hsv_srgb(linear:vec3<f32>) -> vec3<f32> {
    return select(1.055*pow(linear,vec3<f32>(1.0/2.4))-vec3<f32>(0.055),
        linear*12.92,linear<=vec3<f32>(0.0031308));
}

fn hsv_linear(srgb:vec3<f32>) -> vec3<f32> {
    return select(pow((srgb+vec3<f32>(0.055))/1.055,vec3<f32>(2.4)),
        srgb/12.92,srgb<=vec3<f32>(0.04045));
}

fn adjust_hsv(linear:vec3<f32>) -> vec3<f32> {
    // Recover the exact 8-bit input before chroma amplification; tiny hardware
    // sRGB decode errors otherwise become visible on almost-neutral colors.
    let rgb = round(clamp(hsv_srgb(linear),vec3<f32>(0.0),vec3<f32>(1.0))*255.0)/255.0;
    let high = max(rgb.r,max(rgb.g,rgb.b));
    var chroma = high-min(rgb.r,min(rgb.g,rgb.b));
    var hue = 0.0;
    if (chroma>1.0e-6) {
        if (rgb.r==high) { hue=(rgb.g-rgb.b)/chroma; }
        else if (rgb.g==high) { hue=2.0+(rgb.b-rgb.r)/chroma; }
        else { hue=4.0+(rgb.r-rgb.g)/chroma; }
        hue=fract((hue+clamp(vu.hue,-180.0,180.0)/60.0)/6.0)*6.0;
    }
    let saturation=clamp(vu.saturation,-100.0,100.0)/100.0;
    let gain=select(1.0+saturation,1.0+saturation+2.0*saturation*saturation,saturation>0.0);
    chroma=min(chroma*gain,1.0);
    let value=clamp(vu.value,-100.0,100.0)/100.0;
    let v=high*(1.0-abs(value))+max(value,0.0);
    chroma=min(chroma*(1.0-abs(value)),v);
    let x=chroma*(1.0-abs((hue%2.0)-1.0));
    var out=vec3<f32>(chroma,0.0,x);
    switch u32(hue) {
        case 0u: {out=vec3<f32>(chroma,x,0.0);}
        case 1u: {out=vec3<f32>(x,chroma,0.0);}
        case 2u: {out=vec3<f32>(0.0,chroma,x);}
        case 3u: {out=vec3<f32>(0.0,x,chroma);}
        case 4u: {out=vec3<f32>(x,0.0,chroma);}
        default: {}
    }
    // Match the committed 8-bit result, including quantization, during preview.
    return hsv_linear(round(clamp(out+vec3<f32>(v-chroma),vec3<f32>(0.0),vec3<f32>(1.0))*255.0)/255.0);
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
    filter_scale: f32,
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
    var corners = array<vec2<f32>, 4>(fu.c0, fu.c1, fu.c2, fu.c3);
    var uv = corner_uv[idx];
    // Dense selections need room for the bilinear edge; tiled quads stay disjoint.
    if (fu.origin.x == 0.0) {
        let pad = vec2<f32>(1.0 / fu.filter_scale) / fu.size;
        uv = uv * (vec2<f32>(1.0) + 2.0 * pad) - pad;
    }
    let screen = corners[0] + (corners[1]-corners[0])*uv.x + (corners[3]-corners[0])*uv.y;
    var out: VsOut;
    out.pos = to_ndc(screen, fu.viewport);
    out.uv = uv;
    out.screen = screen;
    return out;
}

@fragment
fn fs_float(in: VsOut) -> @location(0) vec4<f32> {
    let source = in.uv * fu.size + fu.origin;
    let footprint = max(length(dpdx(source)),length(dpdy(source)));
    // Fixed halo supports up to level 5, including the minimum 5% Transform scale.
    let lod = min(5.0,log2(max(1.0/fu.filter_scale,max(footprint,1.0))));
    let tex = textureSampleLevel(fimg,fsamp,source/fu.image_size,lod);
    if (tex.a < 0.5/255.0) { return vec4<f32>(0.0); }
    // Dense selections have a transparent border outside their source rectangle.
    // Sparse tiles already contain neighbouring pixels and transparent halos.
    var edge = 1.0;
    if (fu.origin.x == 0.0) {
        let half_width = exp2(lod)*0.5;
        let low = clamp(source/(2.0*half_width)+vec2<f32>(0.5),vec2<f32>(0.0),vec2<f32>(1.0));
        let high = clamp((fu.size-source)/(2.0*half_width)+vec2<f32>(0.5),vec2<f32>(0.0),vec2<f32>(1.0));
        edge = low.x*low.y*high.x*high.y;
    }
    return vec4<f32>(clamp(tex.rgb/tex.a,vec3<f32>(0.0),vec3<f32>(1.0)),tex.a*fu.opacity*edge);
}

// --- Transform preview mip generation (runs only when the source changes) ---
@group(0) @binding(0) var mip_img: texture_2d<f32>;
@group(0) @binding(1) var mip_mask: texture_2d<f32>;
@group(0) @binding(2) var mip_sampler: sampler;

@vertex
fn vs_mip(@builtin(vertex_index) idx: u32) -> VsOut {
    var uv = array<vec2<f32>,3>(vec2<f32>(0.0,0.0),vec2<f32>(2.0,0.0),vec2<f32>(0.0,2.0));
    var out: VsOut;
    out.uv = uv[idx];
    out.pos = vec4<f32>(out.uv.x*2.0-1.0,1.0-out.uv.y*2.0,0.0,1.0);
    out.screen = vec2<f32>(0.0);
    return out;
}

@fragment
fn fs_mip_init(in: VsOut) -> @location(0) vec4<f32> {
    let coord = vec2<i32>(in.uv*vec2<f32>(textureDimensions(mip_img)));
    let tex = textureLoad(mip_img,coord,0);
    let alpha = tex.a*textureLoad(mip_mask,coord,0).r;
    return vec4<f32>(tex.rgb*alpha,alpha);
}

@fragment
fn fs_mip_down(in: VsOut) -> @location(0) vec4<f32> {
    // Hardware bilinear averages each 2x2 group in premultiplied linear light.
    return textureSampleLevel(mip_img,mip_sampler,in.uv,0.0);
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
