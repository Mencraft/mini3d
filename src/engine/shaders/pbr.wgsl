// Physically based forward shading: Cook-Torrance GGX for direct lights, image-based lighting
// from the HDR environment, a PCF-filtered shadow map, and an optional planar mirror reflection.
// Output is linear HDR; the tonemap pass maps it to the screen.

const PI: f32 = 3.14159265;

struct View {
    view_proj: mat4x4<f32>,
    inv_view_proj: mat4x4<f32>,
    eye: vec4<f32>,
    // x: clip below plane (1/0), y: plane height, z: planar reflection available (1/0)
    clip: vec4<f32>,
    // xy: render target size in pixels
    viewport: vec4<f32>,
};

struct Object {
    model: mat4x4<f32>,
    base_color: vec4<f32>,
    uv_transform: vec4<f32>, // xy scale, zw offset
    material: vec4<f32>,     // x metallic, y roughness, z mirror strength
    emissive: vec4<f32>,
};

struct Light {
    position: vec4<f32>,  // w: 0 directional, 1 point, 2 spot
    direction: vec4<f32>,
    color: vec4<f32>,     // rgb * intensity
    params: vec4<f32>,    // x range, y cos(inner), z cos(outer)
};

struct World {
    lights: array<Light, 16>,
    shadow_view_proj: mat4x4<f32>,
    params: vec4<f32>,        // x light count, y environment intensity, z shadow light index (-1 none), w env max mip
    shadow_params: vec4<f32>, // x shadow texel size (uv), y normal offset (world units)
};

@group(0) @binding(0) var<uniform> view: View;
@group(0) @binding(1) var t_reflection: texture_2d<f32>;
@group(0) @binding(2) var s_linear: sampler;
@group(1) @binding(0) var<uniform> object: Object;
@group(2) @binding(0) var t_albedo: texture_2d<f32>;
@group(2) @binding(1) var s_albedo: sampler;
@group(3) @binding(0) var<uniform> world: World;
@group(3) @binding(1) var t_shadow: texture_depth_2d;
@group(3) @binding(2) var s_shadow: sampler_comparison;
@group(3) @binding(3) var t_env: texture_2d<f32>;
@group(3) @binding(4) var s_env: sampler;

struct VsIn {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
};

struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) world_pos: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
};

@vertex
fn vs_main(in: VsIn) -> VsOut {
    let world_pos = object.model * vec4<f32>(in.position, 1.0);
    var out: VsOut;
    out.clip = view.view_proj * world_pos;
    out.world_pos = world_pos.xyz;
    // Valid for rotations + uniform scale, which is all the engine uses.
    out.normal = (object.model * vec4<f32>(in.normal, 0.0)).xyz;
    out.uv = in.uv * object.uv_transform.xy + object.uv_transform.zw;
    return out;
}

// ------------------------------------------------------------------ environment

fn env_uv(d: vec3<f32>) -> vec2<f32> {
    return vec2<f32>(atan2(d.z, d.x) / (2.0 * PI) + 0.5, acos(clamp(d.y, -1.0, 1.0)) / PI);
}

fn sample_env(d: vec3<f32>, lod: f32) -> vec3<f32> {
    return textureSampleLevel(t_env, s_env, env_uv(d), lod).rgb;
}

// ------------------------------------------------------------------ BRDF

fn distribution_ggx(nh: f32, roughness: f32) -> f32 {
    let a = roughness * roughness;
    let a2 = a * a;
    let d = nh * nh * (a2 - 1.0) + 1.0;
    return a2 / (PI * d * d);
}

fn geometry_smith(nv: f32, nl: f32, roughness: f32) -> f32 {
    let k = (roughness + 1.0) * (roughness + 1.0) / 8.0;
    return (nv / (nv * (1.0 - k) + k)) * (nl / (nl * (1.0 - k) + k));
}

fn fresnel_schlick(cos_theta: f32, f0: vec3<f32>) -> vec3<f32> {
    return f0 + (1.0 - f0) * pow(1.0 - cos_theta, 5.0);
}

fn fresnel_schlick_roughness(cos_theta: f32, f0: vec3<f32>, roughness: f32) -> vec3<f32> {
    return f0 + (max(vec3<f32>(1.0 - roughness), f0) - f0) * pow(1.0 - cos_theta, 5.0);
}

// Analytic fit of the split-sum environment BRDF (Karis, "Physically Based Shading on Mobile").
fn env_brdf(f0: vec3<f32>, roughness: f32, nv: f32) -> vec3<f32> {
    let c0 = vec4<f32>(-1.0, -0.0275, -0.572, 0.022);
    let c1 = vec4<f32>(1.0, 0.0425, 1.04, -0.04);
    let r = roughness * c0 + c1;
    let a004 = min(r.x * r.x, exp2(-9.28 * nv)) * r.x + r.y;
    let ab = vec2<f32>(-1.04, 1.04) * a004 + r.zw;
    return f0 * ab.x + ab.y;
}

// ------------------------------------------------------------------ shadows

fn shadow_factor(world_pos: vec3<f32>, n: vec3<f32>, l: vec3<f32>) -> f32 {
    // Push the lookup point out along the normal (more at grazing angles) to avoid acne.
    let offset = n * world.shadow_params.y * (1.5 - dot(n, l));
    let c = world.shadow_view_proj * vec4<f32>(world_pos + offset, 1.0);
    let ndc = c.xyz / c.w;
    let uv = vec2<f32>(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5);
    if any(uv < vec2<f32>(0.0)) || any(uv > vec2<f32>(1.0)) || ndc.z > 1.0 {
        return 1.0;
    }
    // 5x5 percentage-closer filtering; each tap is itself bilinear, giving soft edges.
    let texel = world.shadow_params.x;
    var lit = 0.0;
    for (var y = -2; y <= 2; y++) {
        for (var x = -2; x <= 2; x++) {
            let o = vec2<f32>(f32(x), f32(y)) * texel;
            lit += textureSampleCompareLevel(t_shadow, s_shadow, uv + o, ndc.z);
        }
    }
    return lit / 25.0;
}

// ------------------------------------------------------------------ main

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let tex = textureSample(t_albedo, s_albedo, in.uv);
    if view.clip.x > 0.5 && in.world_pos.y < view.clip.y - 0.001 {
        discard; // reflection pass: drop anything below the mirror plane
    }

    let albedo = object.base_color.rgb * tex.rgb;
    let metallic = object.material.x;
    let roughness = clamp(object.material.y, 0.04, 1.0);
    let n = normalize(in.normal);
    let v = normalize(view.eye.xyz - in.world_pos);
    let nv = max(dot(n, v), 1e-4);
    let f0 = mix(vec3<f32>(0.04), albedo, metallic);

    // Direct lighting
    var lo = vec3<f32>(0.0);
    let count = u32(world.params.x);
    for (var i = 0u; i < count; i++) {
        let light = world.lights[i];
        let kind = light.position.w;
        var l: vec3<f32>;
        var attenuation = 1.0;
        if kind < 0.5 {
            l = normalize(-light.direction.xyz);
        } else {
            let to_light = light.position.xyz - in.world_pos;
            let dist = length(to_light);
            l = to_light / dist;
            // Inverse-square falloff, windowed to reach exactly zero at `range`.
            let window = clamp(1.0 - pow(dist / light.params.x, 4.0), 0.0, 1.0);
            attenuation = window * window / (dist * dist + 1.0);
            if kind > 1.5 {
                let cos_angle = dot(-l, normalize(light.direction.xyz));
                attenuation *= smoothstep(light.params.z, light.params.y, cos_angle);
            }
        }
        let nl = dot(n, l);
        if nl <= 0.0 || attenuation <= 0.0 {
            continue;
        }
        if f32(i) == world.params.z {
            attenuation *= shadow_factor(in.world_pos, n, l);
        }
        let h = normalize(l + v);
        let f = fresnel_schlick(max(dot(v, h), 0.0), f0);
        let specular = distribution_ggx(max(dot(n, h), 0.0), roughness) * geometry_smith(nv, nl, roughness) * f
            / (4.0 * nv * nl + 1e-4);
        let kd = (vec3<f32>(1.0) - f) * (1.0 - metallic);
        lo += (kd * albedo / PI + specular) * light.color.rgb * attenuation * nl;
    }

    // Image-based lighting: blurrier environment mips approximate rougher surfaces.
    let max_lod = world.params.w;
    let r = reflect(-v, n);
    let f = fresnel_schlick_roughness(nv, f0, roughness);
    let kd = (vec3<f32>(1.0) - f) * (1.0 - metallic);
    let irradiance = sample_env(n, max(max_lod - 2.0, 0.0));
    let prefiltered = sample_env(r, roughness * max(max_lod - 1.0, 0.0));
    let ambient = (kd * irradiance * albedo + prefiltered * env_brdf(f0, roughness, nv)) * world.params.y;

    var color = lo + ambient + object.emissive.rgb;

    // Planar mirror: the reflection pass rendered the mirrored scene from the same screen position.
    let mirror = object.material.z;
    if mirror > 0.0 && view.clip.z > 0.5 {
        let screen_uv = in.clip.xy / view.viewport.xy;
        let reflection = textureSampleLevel(t_reflection, s_linear, screen_uv, 0.0).rgb;
        let fresnel = pow(1.0 - nv, 5.0);
        color = mix(color, reflection, mirror * mix(0.3, 1.0, fresnel));
    }

    return vec4<f32>(color, 1.0);
}
