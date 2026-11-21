struct Vertex {
    @builtin(position) position: vec4f,
    @location(0) color: vec4f,
}

@vertex
fn vs_main(@location(0) position: vec4f, @location(1) color: vec4f) -> Vertex {
    return Vertex(position, color);
}

@fragment
fn fs_main(vertex: Vertex) -> @location(0) vec4f {
    return vertex.color;
}
