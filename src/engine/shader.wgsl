struct Camera {
    view_proj: mat4x4<f32>,
    eye: vec4<f32>,
    light_dir: vec4<f32>,
};

struct Object {
    model: mat4x4<f32>,
    color: vec4<f32>,
};

@group(0) @binding(0) var<uniform> camera: Camera;
@group(1) @binding(0) var<uniform> object: Object;

struct VsIn {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
};

struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) world_pos: vec3<f32>,
    @location(1) normal: vec3<f32>,
};

@vertex
fn vs_main(in: VsIn) -> VsOut {
    let world = object.model * vec4<f32>(in.position, 1.0);
    var out: VsOut;
    out.clip = camera.view_proj * world;
    out.world_pos = world.xyz;
    // Valid for rotations + uniform scale, which is all the engine uses.
    out.normal = (object.model * vec4<f32>(in.normal, 0.0)).xyz;
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let n = normalize(in.normal);
    let l = normalize(-camera.light_dir.xyz);
    let v = normalize(camera.eye.xyz - in.world_pos);
    let h = normalize(l + v);

    let ambient = 0.15;
    let diffuse = max(dot(n, l), 0.0);
    let specular = pow(max(dot(n, h), 0.0), 64.0) * 0.5;
    // Cool sky-colored fill from above so shadowed sides aren't flat.
    let hemi = mix(vec3<f32>(0.08, 0.07, 0.06), vec3<f32>(0.15, 0.18, 0.25), n.y * 0.5 + 0.5);

    let base = object.color.rgb;
    let color = base * (ambient + diffuse * 0.85) + hemi * base + vec3<f32>(specular);
    return vec4<f32>(color, 1.0);
}
