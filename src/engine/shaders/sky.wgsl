// Draws the HDR environment behind everything, as a fullscreen triangle at the far plane.

const PI: f32 = 3.14159265;

struct View {
    view_proj: mat4x4<f32>,
    inv_view_proj: mat4x4<f32>,
    eye: vec4<f32>,
    clip: vec4<f32>,
    viewport: vec4<f32>,
};

struct Light {
    position: vec4<f32>,
    direction: vec4<f32>,
    color: vec4<f32>,
    params: vec4<f32>,
};

struct World {
    lights: array<Light, 16>,
    shadow_view_proj: mat4x4<f32>,
    params: vec4<f32>,
    shadow_params: vec4<f32>,
};

@group(0) @binding(0) var<uniform> view: View;
@group(1) @binding(0) var<uniform> world: World;
@group(1) @binding(3) var t_env: texture_2d<f32>;
@group(1) @binding(4) var s_env: sampler;

struct VsOut {
    @builtin(position) position: vec4<f32>,
    @location(0) ndc: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) i: u32) -> VsOut {
    let ndc = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u)) * 2.0 - 1.0;
    var out: VsOut;
    out.position = vec4<f32>(ndc, 1.0, 1.0);
    out.ndc = ndc;
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    // Un-project near and far points to get this pixel's view ray. Works for the mirrored
    // reflection camera too, since the mirror is baked into inv_view_proj.
    let near = view.inv_view_proj * vec4<f32>(in.ndc, 0.0, 1.0);
    let far = view.inv_view_proj * vec4<f32>(in.ndc, 1.0, 1.0);
    let d = normalize(far.xyz / far.w - near.xyz / near.w);
    let uv = vec2<f32>(atan2(d.z, d.x) / (2.0 * PI) + 0.5, acos(clamp(d.y, -1.0, 1.0)) / PI);
    let color = textureSampleLevel(t_env, s_env, uv, 0.0).rgb * world.params.y;
    return vec4<f32>(color, 1.0);
}
