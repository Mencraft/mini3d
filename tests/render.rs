//! End-to-end rendering tests: build a scene, render it on the GPU without a window, and check
//! pixels at the screen positions of known 3D points.
//!
//! The tests check behavior ("the spot in the shadow is darker than the lit spot"), not exact
//! images, so they keep passing across GPUs and small shading tweaks but catch real breakage:
//! inverted light directions, missing shadows, broken reflections, culled faces, color-space
//! mistakes, GPU validation errors.
//!
//! Each test skips (passes with a message) on machines with no GPU.

use glam::{Quat, Vec3};
use mini3d::engine::{HdrImage, Image, Light, Material, Mesh, OrbitCamera, Renderer, Scene, Tonemap, Transform};

const W: u32 = 320;
const H: u32 = 200;

/// Test fixture: a headless renderer, an empty scene and a camera looking at the origin.
struct Fixture {
    renderer: Renderer,
    scene: Scene,
    camera: OrbitCamera,
}

impl Fixture {
    /// Returns `None` (and the test should return early) if there is no GPU.
    /// The environment starts out black so only the lights a test adds contribute.
    fn new() -> Option<Self> {
        let _ = env_logger::builder().is_test(true).try_init();
        let Some(mut renderer) = pollster::block_on(Renderer::headless(W, H)) else {
            eprintln!("skipping: no GPU adapter available");
            return None;
        };
        renderer.set_environment(&solid_env([0.0; 3]));
        let scene = Scene::new(&renderer);
        let mut camera = OrbitCamera::new(W as f32 / H as f32);
        camera.target = Vec3::ZERO;
        camera.distance = 8.0;
        camera.yaw = 0.0; // looking from +Z toward -Z
        camera.pitch = 0.6;
        Some(Self { renderer, scene, camera })
    }

    fn mesh(&mut self, mesh: &Mesh) -> usize {
        self.scene.add_mesh(&self.renderer, mesh)
    }

    fn spawn(&mut self, mesh: &Mesh, position: Vec3, material: Material) -> usize {
        let m = self.mesh(mesh);
        self.scene.spawn(&self.renderer, m, Transform::at(position), material)
    }

    /// A large matte floor at y = 0.
    fn floor(&mut self, material: Material) -> usize {
        self.spawn(&Mesh::plane(30.0, 1), Vec3::ZERO, material)
    }

    /// Renders and fails the test on any wgpu validation error.
    fn render(&mut self) -> Frame {
        let device = self.renderer.device().clone();
        device.push_error_scope(wgpu::ErrorFilter::Validation);
        let image = self.renderer.screenshot(&self.scene, &self.camera);
        if let Some(err) = pollster::block_on(device.pop_error_scope()) {
            panic!("GPU validation error: {err}");
        }
        Frame { image, camera: self.camera.view_proj() }
    }
}

struct Frame {
    image: Image,
    camera: glam::Mat4,
}

impl Frame {
    /// Screen pixel of a world-space point.
    fn project(&self, p: Vec3) -> (u32, u32) {
        let ndc = self.camera.project_point3(p);
        assert!(ndc.x.abs() < 1.0 && ndc.y.abs() < 1.0, "{p} is off screen (ndc {ndc})");
        let w = self.image.width as f32;
        let h = self.image.height as f32;
        (((ndc.x * 0.5 + 0.5) * w) as u32, ((0.5 - ndc.y * 0.5) * h) as u32)
    }

    /// sRGB color (0–255) averaged over 3x3 pixels around a screen position.
    fn pixel_at(&self, x: u32, y: u32) -> [f32; 3] {
        let mut sum = [0.0; 3];
        let mut n = 0.0;
        for dy in -1i32..=1 {
            for dx in -1i32..=1 {
                let px = (x as i32 + dx).clamp(0, self.image.width as i32 - 1) as u32;
                let py = (y as i32 + dy).clamp(0, self.image.height as i32 - 1) as u32;
                let i = ((py * self.image.width + px) * 4) as usize;
                for c in 0..3 {
                    sum[c] += self.image.pixels[i + c] as f32;
                }
                n += 1.0;
            }
        }
        sum.map(|s| s / n)
    }

    /// Color at the screen position of a world-space point.
    fn at(&self, p: Vec3) -> [f32; 3] {
        let (x, y) = self.project(p);
        self.pixel_at(x, y)
    }

    fn center(&self) -> [f32; 3] {
        self.pixel_at(self.image.width / 2, self.image.height / 2)
    }

    fn mean_brightness(&self) -> f32 {
        self.image.pixels.chunks_exact(4).map(|p| luma([p[0] as f32, p[1] as f32, p[2] as f32])).sum::<f32>()
            / (self.image.width * self.image.height) as f32
    }
}

fn luma(c: [f32; 3]) -> f32 {
    0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]
}

fn solid_env(rgb: [f32; 3]) -> HdrImage {
    HdrImage::from_fn(8, 4, |_, _| [rgb[0], rgb[1], rgb[2], 1.0])
}

/// Linear 0–1 → sRGB 0–255, what the screen should show for a clamped linear value.
fn srgb8(linear: f32) -> f32 {
    let c = if linear <= 0.0031308 { linear * 12.92 } else { 1.055 * linear.powf(1.0 / 2.4) - 0.055 };
    c * 255.0
}

macro_rules! fixture {
    () => {
        match Fixture::new() {
            Some(f) => f,
            None => return,
        }
    };
}

fn matte_white() -> Material {
    Material::color([0.8; 3]).roughness(1.0)
}

// ------------------------------------------------------------------ basics

#[test]
fn sky_shows_environment_color_in_correct_color_space() {
    let mut f = fixture!();
    f.renderer.set_environment(&solid_env([0.2, 0.4, 0.8]));
    f.renderer.tonemap = Tonemap::Clamp; // no curve, so the output is exactly linear → sRGB
    let px = f.render().center();
    let expected = [srgb8(0.2), srgb8(0.4), srgb8(0.8)];
    for c in 0..3 {
        assert!((px[c] - expected[c]).abs() <= 3.0, "channel {c}: got {px:?}, expected {expected:?}");
    }
}

#[test]
fn screenshot_matches_size_and_follows_resize() {
    let mut f = fixture!();
    let frame = f.render();
    assert_eq!((frame.image.width, frame.image.height), (W, H));
    f.renderer.resize(200, 100);
    assert_eq!(f.renderer.size(), (200, 100));
    let frame = f.render();
    assert_eq!((frame.image.width, frame.image.height), (200, 100));
}

#[test]
fn rendering_is_deterministic() {
    let mut f = fixture!();
    f.renderer.set_environment(&HdrImage::sky(256, 128, Vec3::new(0.3, 1.0, 0.2)));
    f.floor(Material::color([0.8; 3]).mirror(0.5));
    f.spawn(&Mesh::sphere(0.8, 32, 16), Vec3::Y, Material::color([1.0, 0.5, 0.2]).metallic(1.0).roughness(0.2));
    f.scene.add_light(Light::directional(Vec3::new(-1.0, -1.0, -0.5), [1.0; 3], 3.0).with_shadows());
    let a = f.render().image;
    let b = f.render().image;
    assert!(a == b, "two renders of the same scene differ");
}

#[test]
fn every_feature_at_once_has_no_validation_errors() {
    let mut f = fixture!();
    f.renderer.set_environment(&HdrImage::sky(256, 128, Vec3::new(0.3, 1.0, 0.2)));
    let checker = f.scene.add_texture(&f.renderer, &Image::checker(64, 4, [255; 4], [0, 0, 0, 255]));
    f.floor(Material::color([0.8; 3]).texture(checker).mirror(0.4).uv_scale(8.0, 8.0));
    for (i, mesh) in [Mesh::cube(1.0), Mesh::sphere(0.6, 24, 16), Mesh::cylinder(0.5, 1.0, 24), Mesh::cone(0.5, 1.0, 24), Mesh::torus(0.5, 0.2, 24, 12)]
        .iter()
        .enumerate()
    {
        let x = i as f32 * 1.5 - 3.0;
        f.spawn(mesh, Vec3::new(x, 0.8, 0.0), Material::color([0.7; 3]).metallic(i as f32 / 4.0).roughness(0.3));
    }
    f.spawn(&Mesh::sphere(0.1, 8, 6), Vec3::new(0.0, 2.0, 1.0), Material::color([0.0; 3]).emissive([1.0; 3], 10.0));
    f.scene.add_light(Light::directional(Vec3::new(-1.0, -1.0, -0.5), [1.0; 3], 3.0).with_shadows());
    f.scene.add_light(Light::point(Vec3::new(0.0, 2.0, 1.0), [0.5, 0.7, 1.0], 20.0, 8.0));
    f.scene.add_light(Light::spot(Vec3::new(0.0, 4.0, 0.0), -Vec3::Y, [1.0; 3], 50.0, 10.0, 15.0, 25.0));
    for tonemap in [Tonemap::Aces, Tonemap::Reinhard, Tonemap::Clamp] {
        f.renderer.tonemap = tonemap;
        for (shadows, reflections) in [(true, true), (false, true), (true, false), (false, false)] {
            f.renderer.shadows = shadows;
            f.renderer.reflections = reflections;
            f.render();
        }
    }
}

#[test]
fn more_lights_than_the_limit_are_ignored_safely() {
    let mut f = fixture!();
    f.floor(matte_white());
    for i in 0..40 {
        let a = i as f32;
        f.scene.add_light(Light::point(Vec3::new(a.sin() * 3.0, 1.0, a.cos() * 3.0), [1.0; 3], 5.0, 6.0));
    }
    assert!(f.render().mean_brightness() > 5.0);
}

// ------------------------------------------------------------------ geometry

#[test]
fn every_primitive_is_visible_from_every_side() {
    // Catches inverted winding / wrong culling: a face pointing the wrong way vanishes.
    let meshes = [
        ("cube", Mesh::cube(1.6)),
        ("sphere", Mesh::sphere(0.9, 32, 16)),
        ("cylinder", Mesh::cylinder(0.8, 1.6, 32)),
        ("cone", Mesh::cone(0.9, 1.6, 32)),
        ("torus", Mesh::torus(0.75, 0.35, 32, 16)), // viewed off-center below, since its middle is a hole
    ];
    let views = [(0.0, 0.0), (1.57, 0.0), (3.14, 0.0), (-1.57, 0.0), (0.0, 1.45), (0.0, -1.45)];
    for (name, mesh) in meshes {
        let mut f = fixture!();
        f.renderer.set_environment(&solid_env([0.0, 0.0, 1.0]));
        f.renderer.tonemap = Tonemap::Clamp;
        // Emissive red so the color doesn't depend on lighting; the sky is blue.
        f.spawn(&mesh, Vec3::ZERO, Material::color([0.0; 3]).roughness(1.0).emissive([1.0, 0.0, 0.0], 1.0));
        for (yaw, pitch) in views {
            f.camera.yaw = yaw;
            f.camera.pitch = pitch;
            let frame = f.render();
            let probe = if name == "torus" {
                // A point on the ring that faces the camera.
                let eye = f.camera.eye().normalize();
                let side = if eye.y.abs() > 0.9 { Vec3::X } else { Vec3::new(eye.x, 0.0, eye.z).normalize() };
                side * 0.75
            } else {
                Vec3::ZERO
            };
            let px = frame.at(probe);
            assert!(px[0] > 200.0 && px[2] < 50.0, "{name} missing from yaw {yaw}, pitch {pitch}: {px:?}");
        }
    }
}

#[test]
fn texture_is_applied_and_tinted_by_base_color() {
    let mut f = fixture!();
    f.renderer.set_environment(&solid_env([1.0; 3]));
    let red = f.scene.add_texture(&f.renderer, &Image::solid([255, 0, 0, 255]));
    let left = f.spawn(&Mesh::cube(1.5), Vec3::new(-1.5, 0.0, 0.0), Material::color([1.0; 3]).texture(red).roughness(1.0));
    let right = f.spawn(&Mesh::cube(1.5), Vec3::new(1.5, 0.0, 0.0), Material::color([0.0, 1.0, 0.0]).texture(red).roughness(1.0));
    f.camera.pitch = 0.2;
    let frame = f.render();
    let l = frame.at(f.scene.objects[left].transform.position + Vec3::Z * 0.75);
    let r = frame.at(f.scene.objects[right].transform.position + Vec3::Z * 0.75);
    assert!(l[0] > 2.0 * l[1] && l[0] > 2.0 * l[2], "red texture not visible: {l:?}");
    // Green tint × red texture = black diffuse (only a little white specular remains).
    assert!(luma(r) < luma(l) * 0.5, "tint not multiplied: red {l:?} vs tinted {r:?}");
}

// ------------------------------------------------------------------ lights

#[test]
fn directional_light_lights_the_floor_and_can_be_switched_off() {
    let mut f = fixture!();
    f.floor(matte_white());
    let sun = f.scene.add_light(Light::directional(-Vec3::Y, [1.0; 3], 3.0));
    let lit = luma(f.render().at(Vec3::ZERO));
    f.scene.lights[sun].enabled = false;
    let off = luma(f.render().at(Vec3::ZERO));
    assert!(lit > 100.0, "floor should be well lit, got {lit}");
    assert!(off < 3.0, "floor should be black with the light off, got {off}");
}

#[test]
fn directional_light_from_below_does_not_light_the_top_of_the_floor() {
    // Catches a flipped light direction convention.
    let mut f = fixture!();
    f.floor(matte_white());
    f.scene.add_light(Light::directional(Vec3::Y, [1.0; 3], 3.0)); // travelling upward
    let px = luma(f.render().at(Vec3::ZERO));
    assert!(px < 3.0, "light below the floor leaked through: {px}");
}

#[test]
fn brighter_lights_give_brighter_pixels() {
    let mut f = fixture!();
    f.floor(matte_white());
    let sun = f.scene.add_light(Light::directional(-Vec3::Y, [1.0; 3], 0.5));
    let dim = luma(f.render().at(Vec3::ZERO));
    f.scene.lights[sun].intensity = 2.0;
    let bright = luma(f.render().at(Vec3::ZERO));
    assert!(bright > dim + 20.0, "intensity had no effect: {dim} → {bright}");
}

#[test]
fn colored_light_tints_a_white_surface() {
    let mut f = fixture!();
    f.floor(matte_white());
    f.scene.add_light(Light::directional(-Vec3::Y, [0.0, 0.0, 1.0], 3.0));
    let px = f.render().at(Vec3::ZERO);
    assert!(px[2] > 100.0 && px[0] < 10.0 && px[1] < 10.0, "expected blue, got {px:?}");
}

#[test]
fn point_light_falls_off_with_distance_and_stops_at_its_range() {
    let mut f = fixture!();
    f.floor(matte_white());
    f.scene.add_light(Light::point(Vec3::new(0.0, 1.0, 0.0), [1.0; 3], 10.0, 3.0));
    let frame = f.render();
    let under = luma(frame.at(Vec3::ZERO));
    let near = luma(frame.at(Vec3::new(1.5, 0.0, 0.0)));
    let beyond = luma(frame.at(Vec3::new(3.5, 0.0, 0.0))); // farther than range 3
    assert!(under > near && near > beyond, "no falloff: {under} / {near} / {beyond}");
    assert!(beyond < 2.0, "light reaches past its range: {beyond}");
}

#[test]
fn point_light_can_be_moved() {
    let mut f = fixture!();
    f.floor(matte_white());
    let lamp = f.scene.add_light(Light::point(Vec3::new(-2.0, 0.5, 0.0), [1.0; 3], 10.0, 2.5));
    let before = f.render();
    f.scene.lights[lamp].set_position(Vec3::new(2.0, 0.5, 0.0));
    let after = f.render();
    let (l, r) = (Vec3::new(-2.0, 0.0, 0.0), Vec3::new(2.0, 0.0, 0.0));
    assert!(luma(before.at(l)) > luma(before.at(r)) + 30.0);
    assert!(luma(after.at(r)) > luma(after.at(l)) + 30.0);
}

#[test]
fn spot_light_lights_inside_its_cone_only() {
    let mut f = fixture!();
    f.floor(matte_white());
    // 4 units up, 15° outer half-angle → lit circle radius ≈ 1.07.
    f.scene.add_light(Light::spot(Vec3::new(0.0, 4.0, 0.0), -Vec3::Y, [1.0; 3], 60.0, 12.0, 10.0, 15.0));
    let frame = f.render();
    let inside = luma(frame.at(Vec3::ZERO));
    let outside = luma(frame.at(Vec3::new(2.0, 0.0, 0.0)));
    assert!(inside > 50.0, "spot center too dark: {inside}");
    assert!(outside < 2.0, "light outside the cone: {outside}");
}

#[test]
fn spot_light_points_where_it_is_aimed() {
    let mut f = fixture!();
    f.floor(matte_white());
    // From above the origin, tilted toward +X: should hit x ≈ 4·tan(30°) ≈ 2.3.
    let dir = Quat::from_rotation_z(30f32.to_radians()) * -Vec3::Y;
    f.scene.add_light(Light::spot(Vec3::new(0.0, 4.0, 0.0), dir, [1.0; 3], 60.0, 12.0, 8.0, 12.0));
    let frame = f.render();
    let aimed = luma(frame.at(Vec3::new(2.3, 0.0, 0.0)));
    let mirrored = luma(frame.at(Vec3::new(-2.3, 0.0, 0.0)));
    assert!(aimed > 30.0 && mirrored < 2.0, "spot aimed wrong: +x {aimed}, -x {mirrored}");
}

// ------------------------------------------------------------------ shadows

/// A cube floating above the floor with the sun shining diagonally toward -X.
/// The shadow lands around (-1, 0, 0); (1.5, 0, 0) is in full sun.
fn shadow_scene() -> Option<Fixture> {
    let mut f = Fixture::new()?;
    f.floor(matte_white());
    f.spawn(&Mesh::cube(1.0), Vec3::new(0.0, 1.0, 0.0), matte_white());
    f.scene.add_light(Light::directional(Vec3::new(-1.0, -1.0, 0.0), [1.0; 3], 3.0).with_shadows());
    Some(f)
}

const IN_SHADOW: Vec3 = Vec3::new(-1.0, 0.0, 0.0);
const IN_SUN: Vec3 = Vec3::new(1.5, 0.0, 0.0);

#[test]
fn objects_cast_shadows_away_from_the_light() {
    let Some(mut f) = shadow_scene() else { return };
    let frame = f.render();
    let (shadow, sun) = (luma(frame.at(IN_SHADOW)), luma(frame.at(IN_SUN)));
    assert!(sun > 60.0, "lit floor too dark: {sun}");
    assert!(shadow < sun * 0.3, "no shadow behind the cube: shadow {shadow}, sun {sun}");
}

#[test]
fn no_shadow_on_the_side_facing_the_light() {
    // If the shadow matrix were flipped, the shadow would land on the wrong side.
    let Some(mut f) = shadow_scene() else { return };
    let frame = f.render();
    let mirrored = luma(frame.at(-IN_SHADOW));
    let sun = luma(frame.at(IN_SUN));
    assert!((mirrored - sun).abs() < sun * 0.15, "unexpected shadow on the lit side: {mirrored} vs {sun}");
}

#[test]
fn turning_shadows_off_removes_them() {
    let Some(mut f) = shadow_scene() else { return };
    f.renderer.shadows = false;
    let frame = f.render();
    let (shadow, sun) = (luma(frame.at(IN_SHADOW)), luma(frame.at(IN_SUN)));
    assert!((shadow - sun).abs() < sun * 0.15, "shadow still there: {shadow} vs {sun}");
}

#[test]
fn lights_without_cast_shadows_do_not_cast_shadows() {
    let Some(mut f) = shadow_scene() else { return };
    f.scene.lights[0].cast_shadows = false;
    let frame = f.render();
    let (shadow, sun) = (luma(frame.at(IN_SHADOW)), luma(frame.at(IN_SUN)));
    assert!((shadow - sun).abs() < sun * 0.15, "shadow from a non-casting light: {shadow} vs {sun}");
}

#[test]
fn open_floor_has_no_shadow_acne() {
    // Self-shadowing artifacts show up as dark speckles on a flat, fully lit floor.
    let Some(mut f) = shadow_scene() else { return };
    let frame = f.render();
    let reference = luma(frame.at(IN_SUN));
    for x in [1.2, 1.8, 2.4, 3.0] {
        for z in [-1.0, 0.0, 1.0] {
            let px = luma(frame.at(Vec3::new(x, 0.0, z)));
            assert!((px - reference).abs() < reference * 0.1, "acne at ({x}, {z}): {px} vs {reference}");
        }
    }
}

// ------------------------------------------------------------------ reflections

/// A black mirror floor with a glowing red cube above it. The cube's reflection appears at the
/// screen position of its mirror image, (0, -1, 0).
fn mirror_scene() -> Option<Fixture> {
    let mut f = Fixture::new()?;
    f.floor(Material::color([0.0; 3]).roughness(1.0).mirror(1.0));
    f.spawn(&Mesh::cube(1.0), Vec3::new(0.0, 1.0, 0.0), Material::color([0.0; 3]).emissive([1.0, 0.0, 0.0], 5.0));
    Some(f)
}

#[test]
fn mirror_floor_reflects_objects_above_it() {
    let Some(mut f) = mirror_scene() else { return };
    let px = f.render().at(Vec3::new(0.0, -1.0, 0.0));
    assert!(px[0] > 100.0 && px[0] > 3.0 * px[1], "no red reflection: {px:?}");
}

#[test]
fn turning_reflections_off_removes_them() {
    let Some(mut f) = mirror_scene() else { return };
    f.renderer.reflections = false;
    let px = f.render().at(Vec3::new(0.0, -1.0, 0.0));
    assert!(px[0] < 20.0, "reflection still visible: {px:?}");
}

#[test]
fn mirror_shows_the_underside_of_objects() {
    // A one-sided square facing down: invisible from above, but its mirror image faces the
    // camera. Catches wrong face culling in the reflection pass (the mirrored camera flips
    // triangle winding), which a closed, uniformly colored object can't reveal.
    let mut f = fixture!();
    f.floor(Material::color([0.0; 3]).roughness(1.0).mirror(1.0));
    let id = f.spawn(&Mesh::plane(2.0, 1), Vec3::new(0.0, 1.0, 0.0), Material::color([0.0; 3]).emissive([1.0, 0.0, 0.0], 5.0));
    f.scene.objects[id].transform.rotation = Quat::from_rotation_x(std::f32::consts::PI);
    let frame = f.render();
    let reflection = frame.at(Vec3::new(0.0, -1.0, 0.0));
    let direct = frame.at(Vec3::new(0.0, 1.0, 0.0));
    assert!(reflection[0] > 100.0, "underside missing from the reflection: {reflection:?}");
    assert!(direct[0] < reflection[0] * 0.5, "the back of a one-sided plane should be culled: {direct:?}");
}

#[test]
fn objects_below_the_mirror_are_not_reflected() {
    // The reflection pass clips everything under the mirror plane; without that, this green
    // cube under the floor would show up in the reflection as if it were above.
    let mut f = fixture!();
    f.floor(Material::color([0.0; 3]).roughness(1.0).mirror(1.0));
    f.spawn(&Mesh::cube(1.0), Vec3::new(0.0, -1.0, 0.0), Material::color([0.0; 3]).emissive([0.0, 1.0, 0.0], 5.0));
    let frame = f.render();
    // Pixels where the floor would show the cube's mirror image, if it weren't clipped.
    for p in [Vec3::new(0.0, 1.0, 0.0), Vec3::new(0.0, 0.7, 0.3)] {
        let px = frame.at(p);
        assert!(px[1] < 20.0, "object below the mirror leaked into the reflection at {p}: {px:?}");
    }
}

#[test]
fn smooth_metal_reflects_the_environment() {
    let mut f = fixture!();
    f.renderer.set_environment(&solid_env([0.0, 0.8, 0.0]));
    f.spawn(&Mesh::sphere(1.0, 48, 32), Vec3::ZERO, Material::color([1.0; 3]).metallic(1.0).roughness(0.05));
    let px = f.render().center();
    assert!(px[1] > 150.0 && px[0] < 30.0 && px[2] < 30.0, "chrome should mirror the green sky: {px:?}");
}

#[test]
fn metal_tints_its_reflections_with_its_color() {
    let mut f = fixture!();
    f.renderer.set_environment(&solid_env([1.0; 3]));
    f.spawn(&Mesh::sphere(1.0, 48, 32), Vec3::ZERO, Material::color([1.0, 0.7, 0.2]).metallic(1.0).roughness(0.2));
    let px = f.render().center();
    assert!(px[0] > px[1] && px[1] > px[2], "gold should reflect gold-tinted: {px:?}");
}

#[test]
fn environment_intensity_scales_sky_lighting() {
    let mut f = fixture!();
    f.renderer.set_environment(&solid_env([0.5; 3]));
    f.floor(matte_white());
    let normal = luma(f.render().at(Vec3::ZERO));
    f.renderer.environment_intensity = 0.0;
    let off = luma(f.render().at(Vec3::ZERO));
    assert!(normal > 50.0 && off < 2.0, "env intensity ignored: {normal} → {off}");
}

// ------------------------------------------------------------------ HDR

#[test]
fn very_bright_emissive_saturates_instead_of_breaking() {
    // f16 overflow or a broken tone curve would show up as black, NaN or wrapped colors.
    let mut f = fixture!();
    f.spawn(&Mesh::sphere(1.0, 32, 16), Vec3::ZERO, Material::color([0.0; 3]).emissive([1.0; 3], 1000.0));
    for tonemap in [Tonemap::Aces, Tonemap::Reinhard, Tonemap::Clamp] {
        f.renderer.tonemap = tonemap;
        let px = f.render().center();
        assert!(px.iter().all(|&c| c > 240.0), "{tonemap:?}: {px:?}");
    }
}

#[test]
fn hdr_sun_in_the_sky_is_brighter_than_white_before_tonemapping() {
    let mut f = fixture!();
    // The camera looks toward -Z; put the sun straight ahead, slightly above the horizon,
    // and tilt the camera up to center it.
    let sun_dir = Vec3::new(0.0, 0.3, -1.0).normalize();
    f.renderer.set_environment(&HdrImage::sky(1024, 512, sun_dir));
    f.camera.pitch = -0.29;
    f.renderer.exposure = 1.0 / 16.0; // at very low exposure only HDR values stay visible
    let frame = f.render();
    let sun = luma(frame.at(f.camera.eye() + sun_dir * 50.0));
    let sky = luma(frame.at(f.camera.eye() + Vec3::new(0.4, 0.3, -1.0).normalize() * 50.0));
    assert!(sun > 150.0, "sun lost its HDR brightness: {sun}");
    // Regular sky (~0.7 linear) at 1/16 exposure is ~0.04, which sRGB encodes to roughly 50/255.
    assert!(sun > sky * 2.5, "sun should stand out far above the regular sky at low exposure: sun {sun}, sky {sky}");
}

#[test]
fn exposure_brightens_and_darkens() {
    let mut f = fixture!();
    f.renderer.set_environment(&solid_env([0.3; 3]));
    f.floor(matte_white());
    let normal = f.render().mean_brightness();
    f.renderer.exposure = 4.0;
    let bright = f.render().mean_brightness();
    f.renderer.exposure = 0.25;
    let dark = f.render().mean_brightness();
    assert!(dark < normal && normal < bright, "{dark} < {normal} < {bright} failed");
}

#[test]
fn aces_compresses_highlights_more_than_clamp() {
    let mut f = fixture!();
    f.renderer.set_environment(&solid_env([0.9; 3]));
    f.renderer.tonemap = Tonemap::Clamp;
    let clamp = luma(f.render().center());
    f.renderer.tonemap = Tonemap::Aces;
    let aces = luma(f.render().center());
    assert!(aces < clamp, "ACES should roll off near-white values: aces {aces}, clamp {clamp}");
}
