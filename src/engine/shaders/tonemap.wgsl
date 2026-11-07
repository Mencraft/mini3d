// Maps the linear HDR image to the screen: exposure, then a tone curve.
// The swapchain is sRGB, so the GPU applies gamma encoding on write.

struct Settings {
    params: vec4<f32>, // x exposure multiplier, y mode (0 ACES, 1 Reinhard, 2 clamp)
};

@group(0) @binding(0) var t_hdr: texture_2d<f32>;
@group(0) @binding(1) var s_hdr: sampler;
@group(0) @binding(2) var<uniform> settings: Settings;

struct VsOut {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) i: u32) -> VsOut {
    let ndc = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u)) * 2.0 - 1.0;
    var out: VsOut;
    out.position = vec4<f32>(ndc, 0.0, 1.0);
    out.uv = vec2<f32>(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5);
    return out;
}

// Narkowicz's fit of the ACES filmic curve.
fn aces(x: vec3<f32>) -> vec3<f32> {
    return clamp((x * (2.51 * x + 0.03)) / (x * (2.43 * x + 0.59) + 0.14), vec3<f32>(0.0), vec3<f32>(1.0));
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let hdr = textureSample(t_hdr, s_hdr, in.uv).rgb * settings.params.x;
    let mode = u32(settings.params.y);
    var ldr: vec3<f32>;
    if mode == 0u {
        ldr = aces(hdr);
    } else if mode == 1u {
        ldr = hdr / (1.0 + hdr);
    } else {
        ldr = clamp(hdr, vec3<f32>(0.0), vec3<f32>(1.0));
    }
    return vec4<f32>(ldr, 1.0);
}
