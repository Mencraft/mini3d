// Depth-only pass from the shadow-casting light's point of view.

struct View {
    view_proj: mat4x4<f32>,
    inv_view_proj: mat4x4<f32>,
    eye: vec4<f32>,
    clip: vec4<f32>,
    viewport: vec4<f32>,
};

struct Object {
    model: mat4x4<f32>,
    base_color: vec4<f32>,
    uv_transform: vec4<f32>,
    material: vec4<f32>,
    emissive: vec4<f32>,
};

@group(0) @binding(0) var<uniform> view: View;
@group(1) @binding(0) var<uniform> object: Object;

@vertex
fn vs_main(@location(0) position: vec3<f32>) -> @builtin(position) vec4<f32> {
    return view.view_proj * object.model * vec4<f32>(position, 1.0);
}
