struct View {
    viewport: vec2<f32>, center: vec2<f32>, image_size: vec2<f32>, zoom: f32, rotation: f32,
    opacity: f32, selected: f32, hue: f32, saturation: f32, value: f32, pad: f32,
}
@group(0) @binding(0) var<uniform> view: View;
@group(0) @binding(1) var source: texture_2d<f32>;
@group(0) @binding(2) var mask: texture_2d<f32>;
struct Vertex {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) destination: vec2<f32>,
}
fn vertex(position: vec2<f32>, uv: vec2<f32>) -> Vertex {
    let p = (position-view.image_size*0.5)*view.zoom;
    let c = cos(view.rotation); let s = sin(view.rotation);
    let screen = view.center + vec2<f32>(p.x*c-p.y*s,p.x*s+p.y*c);
    var out: Vertex;
    out.position = vec4<f32>(screen.x/view.viewport.x*2.0-1.0,1.0-screen.y/view.viewport.y*2.0,0.0,1.0);
    out.uv = uv; out.destination = position;
    return out;
}
@vertex fn vs_mesh(@location(0) position: vec2<f32>, @location(1) uv: vec2<f32>) -> Vertex { return vertex(position,uv); }
@vertex fn vs_base(@builtin(vertex_index) i: u32) -> Vertex {
    let corners = array<vec2<f32>,6>(vec2<f32>(0,0),vec2<f32>(1,0),vec2<f32>(1,1),vec2<f32>(0,0),vec2<f32>(1,1),vec2<f32>(0,1));
    let p = corners[i]*view.image_size; return vertex(p,p);
}
fn selected(p: vec2<f32>) -> bool {
    if view.selected < 0.5 { return true; }
    let size = vec2<i32>(textureDimensions(mask));
    return textureLoad(mask,clamp(vec2<i32>(floor(p)),vec2<i32>(0),size-1),0).r > 0.5;
}
fn pixel(p: vec2<i32>) -> vec4<f32> {
    let size = vec2<i32>(textureDimensions(source));
    let v = textureLoad(source,clamp(p,vec2<i32>(0),size-1),0);
    return vec4<f32>(v.rgb*v.a,v.a);
}
fn sample_source(p: vec2<f32>) -> vec4<f32> {
    let q = p-0.5; let base = vec2<i32>(floor(q)); let t = fract(q);
    let v = mix(mix(pixel(base),pixel(base+vec2<i32>(1,0)),t.x),mix(pixel(base+vec2<i32>(0,1)),pixel(base+vec2<i32>(1,1)),t.x),t.y);
    if v.a < 0.000001 { return vec4<f32>(0); }
    return vec4<f32>(v.rgb/v.a,v.a*view.opacity);
}
@fragment fn fs_mesh(in: Vertex) -> @location(0) vec4<f32> {
    if !selected(in.destination) { discard; }
    return sample_source(in.uv);
}
@fragment fn fs_base(in: Vertex) -> @location(0) vec4<f32> {
    if selected(in.destination) { discard; }
    let v = textureLoad(source,vec2<i32>(floor(in.uv)),0);
    return vec4<f32>(v.rgb,v.a*view.opacity);
}
