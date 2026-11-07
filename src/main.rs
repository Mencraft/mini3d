mod engine;

use engine::{HdrImage, Image, Light, Material, Mesh, OrbitCamera, Renderer, Scene, Transform};
use glam::{Quat, Vec3};
use std::{sync::Arc, time::Instant};
use winit::{
    application::ApplicationHandler,
    event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent},
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop},
    keyboard::{Key, NamedKey},
    window::{Window, WindowId},
};

const HELP: &str = "drag: orbit · scroll: zoom · 1/2/3: sun/point/spot light · H: shadows · R: reflections · \
                    T: tonemap · -/=: exposure · [/]: sky intensity · P: screenshot · space: pause · esc: quit";

struct State {
    window: Arc<Window>,
    renderer: Renderer,
    scene: Scene,
    camera: OrbitCamera,
    spinning: Vec<usize>,
    /// (light, marker object) pairs for the orbiting point light.
    point_light: (usize, usize),
    lights: [usize; 3],
    start: Instant,
    paused_at: Option<f32>,
    dragging: bool,
    last_cursor: Option<(f64, f64)>,
    /// From `--screenshot <path>`: save the first frame there, then quit.
    screenshot_and_exit: Option<String>,
}

impl State {
    fn new(window: Arc<Window>) -> Self {
        let mut renderer = pollster::block_on(Renderer::new(window.clone()));
        let mut scene = Scene::new(&renderer);

        let cube = scene.add_mesh(&renderer, &Mesh::cube(1.4));
        let sphere = scene.add_mesh(&renderer, &Mesh::sphere(0.8, 64, 48));
        let cylinder = scene.add_mesh(&renderer, &Mesh::cylinder(0.6, 1.6, 48));
        let cone = scene.add_mesh(&renderer, &Mesh::cone(0.75, 1.6, 48));
        let torus = scene.add_mesh(&renderer, &Mesh::torus(0.7, 0.25, 64, 24));
        let plane = scene.add_mesh(&renderer, &Mesh::plane(14.0, 1));
        let bulb = scene.add_mesh(&renderer, &Mesh::sphere(0.12, 16, 12));

        let checker = scene.add_texture(&renderer, &Image::checker(256, 2, [235; 4], [60, 60, 60, 255]));
        let grid = scene.add_texture(&renderer, &Image::grid(256, 8, [40, 40, 40, 255], [255; 4]));

        // ---- lights
        let sun_direction = Vec3::new(-0.5, -1.0, -0.4);
        let sun = scene.add_light(Light::directional(sun_direction, [1.0, 0.95, 0.85], 3.0).with_shadows());
        let point = scene.add_light(Light::point(Vec3::new(4.0, 1.5, 0.0), [0.35, 0.55, 1.0], 30.0, 9.0));
        let spot = scene.add_light(Light::spot(
            Vec3::new(0.0, 5.0, 0.0),
            -Vec3::Y,
            [1.0, 0.55, 0.25],
            80.0,
            12.0,
            14.0,
            22.0,
        ));
        renderer.set_environment(&HdrImage::sky(1024, 512, -sun_direction));

        // ---- command line:
        //   --screenshot <out.png>   render one frame to a file and quit
        //   --keys <chars>           press these keys first (e.g. "h2" = shadows off, point light off)
        //   <file>.hdr               sky / environment;  any other image file → texture on the cube
        let mut cube_texture = (grid, [0.9, 0.3, 0.25]);
        let (mut screenshot_and_exit, mut keys) = (None, String::new());
        let mut args = std::env::args().skip(1);
        while let Some(path) = args.next() {
            if path == "--screenshot" {
                screenshot_and_exit = args.next();
            } else if path == "--keys" {
                keys = args.next().unwrap_or_default();
            } else if path.to_lowercase().ends_with(".hdr") {
                match HdrImage::from_file(&path) {
                    Ok(img) => renderer.set_environment(&img),
                    Err(e) => log::error!("could not load environment {path}: {e}"),
                }
            } else {
                match Image::from_file(&path) {
                    Ok(img) => cube_texture = (scene.add_texture(&renderer, &img), [1.0; 3]),
                    Err(e) => log::error!("could not load texture {path}: {e}"),
                }
            }
        }

        // ---- objects
        let floor = Material::color([0.8; 3]).texture(checker).roughness(0.6).mirror(0.25).uv_scale(7.0, 7.0);
        scene.spawn(&renderer, plane, Transform::at(Vec3::new(0.0, -0.8, 0.0)), floor);

        let shapes = [
            (cube, Material::color(cube_texture.1).texture(cube_texture.0).roughness(0.55)),
            (sphere, Material::color([0.95; 3]).metallic(1.0).roughness(0.05)), // chrome
            (cylinder, Material::color([0.2, 0.75, 0.4]).roughness(0.25)),      // glossy plastic
            (cone, Material::color([0.95, 0.64, 0.54]).metallic(1.0).roughness(0.35)), // copper
            (torus, Material::color([1.0, 0.78, 0.34]).metallic(1.0).roughness(0.2)), // gold
        ];
        let mut spinning = Vec::new();
        for (i, (mesh, material)) in shapes.into_iter().enumerate() {
            let angle = i as f32 / shapes.len() as f32 * std::f32::consts::TAU;
            let pos = Vec3::new(angle.sin() * 3.0, 0.2, angle.cos() * 3.0);
            spinning.push(scene.spawn(&renderer, mesh, Transform::at(pos), material));
        }
        // Matte white ball under the spotlight.
        scene.spawn(&renderer, sphere, Transform { scale: 0.6, ..Transform::at(Vec3::new(0.0, -0.32, 0.0)) }, Material::color([0.9; 3]).roughness(0.8));
        // Glowing bulb that follows the point light; HDR emissive so it stays white-hot after tonemapping.
        let marker = scene.spawn(&renderer, bulb, Transform::at(Vec3::ZERO), Material::color([0.0; 3]).emissive([0.35, 0.55, 1.0], 12.0));

        let camera = OrbitCamera::new(renderer.aspect());
        let mut state = Self {
            window,
            renderer,
            scene,
            camera,
            spinning,
            point_light: (point, marker),
            lights: [sun, point, spot],
            start: Instant::now(),
            paused_at: None,
            dragging: false,
            last_cursor: None,
            screenshot_and_exit,
        };
        for k in keys.chars() {
            if !state.key(&k.to_string()) {
                log::warn!("--keys: unknown key {k:?}");
            }
        }
        state.update_title();
        log::info!("{HELP}");
        state
    }

    fn save_screenshot(&mut self, path: &str) {
        match self.renderer.screenshot(&self.scene, &self.camera).save(path) {
            Ok(()) => log::info!("saved screenshot {path}"),
            Err(e) => log::error!("could not save screenshot {path}: {e}"),
        }
    }

    fn time(&self) -> f32 {
        self.paused_at.unwrap_or_else(|| self.start.elapsed().as_secs_f32())
    }

    fn update(&mut self) {
        let t = self.time();
        for (i, &id) in self.spinning.iter().enumerate() {
            let tr = &mut self.scene.objects[id].transform;
            let phase = i as f32 * 1.3;
            tr.rotation = Quat::from_euler(glam::EulerRot::YXZ, t * 0.8 + phase, t * 0.5 + phase, 0.0);
            tr.position.y = 0.4 + (t * 1.5 + phase).sin() * 0.25;
        }
        let (light, marker) = self.point_light;
        let p = Vec3::new((t * 0.7).cos() * 4.6, 1.4 + (t * 1.1).sin() * 0.4, (t * 0.7).sin() * 4.6);
        self.scene.lights[light].set_position(p);
        let on = self.scene.lights[light].enabled;
        self.scene.objects[marker].transform.position = p;
        self.scene.objects[marker].transform.scale = if on { 1.0 } else { 0.0 };
    }

    fn update_title(&self) {
        let r = &self.renderer;
        let onoff = |b: bool| if b { "on" } else { "off" };
        let lights: String = self
            .lights
            .iter()
            .zip(["sun", "point", "spot"])
            .filter(|(&id, _)| self.scene.lights[id].enabled)
            .map(|(_, name)| name)
            .collect::<Vec<_>>()
            .join("+");
        self.window.set_title(&format!(
            "mini3d — lights: {} · shadows {} · reflections {} · {:?} · exposure {:.2} · sky {:.2}",
            if lights.is_empty() { "none" } else { &lights },
            onoff(r.shadows),
            onoff(r.reflections),
            r.tonemap,
            r.exposure,
            r.environment_intensity,
        ));
    }

    fn key(&mut self, key: &str) -> bool {
        let r = &mut self.renderer;
        match key {
            "1" | "2" | "3" => {
                let id = self.lights[key.parse::<usize>().unwrap() - 1];
                self.scene.lights[id].enabled ^= true;
            }
            "h" => r.shadows ^= true,
            "r" => r.reflections ^= true,
            "t" => r.tonemap = r.tonemap.next(),
            "=" | "+" => r.exposure = (r.exposure * 1.25).min(16.0),
            "-" => r.exposure = (r.exposure / 1.25).max(1.0 / 16.0),
            "]" => r.environment_intensity = (r.environment_intensity + 0.25).min(4.0),
            "[" => r.environment_intensity = (r.environment_intensity - 0.25).max(0.0),
            "p" => {
                let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
                self.save_screenshot(&format!("mini3d-{secs}.png"));
                return false; // nothing changed in the title
            }
            _ => return false,
        }
        true
    }
}

#[derive(Default)]
struct App {
    state: Option<State>,
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.state.is_some() {
            return;
        }
        let attrs = Window::default_attributes()
            .with_title("mini3d")
            .with_inner_size(winit::dpi::LogicalSize::new(1280.0, 800.0));
        let window = Arc::new(event_loop.create_window(attrs).expect("create window"));
        self.state = Some(State::new(window));
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        let Some(state) = self.state.as_mut() else { return };
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                state.renderer.resize(size.width, size.height);
                state.camera.aspect = state.renderer.aspect();
            }
            WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed => {
                match &event.logical_key {
                    Key::Named(NamedKey::Escape) => event_loop.exit(),
                    Key::Named(NamedKey::Space) => {
                        state.paused_at = match state.paused_at {
                            Some(t) => {
                                state.start = Instant::now() - std::time::Duration::from_secs_f32(t);
                                None
                            }
                            None => Some(state.time()),
                        };
                    }
                    Key::Character(c) => {
                        if state.key(&c.to_lowercase()) {
                            state.update_title();
                        }
                    }
                    _ => {}
                }
            }
            WindowEvent::MouseInput { state: s, button: MouseButton::Left, .. } => {
                state.dragging = s == ElementState::Pressed;
            }
            WindowEvent::CursorMoved { position, .. } => {
                if let (true, Some((lx, ly))) = (state.dragging, state.last_cursor) {
                    state.camera.rotate((position.x - lx) as f32, (position.y - ly) as f32);
                }
                state.last_cursor = Some((position.x, position.y));
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let amount = match delta {
                    MouseScrollDelta::LineDelta(_, y) => y,
                    MouseScrollDelta::PixelDelta(p) => p.y as f32 * 0.05,
                };
                state.camera.zoom(amount);
            }
            WindowEvent::RedrawRequested => {
                state.update();
                if let Some(path) = state.screenshot_and_exit.take() {
                    state.save_screenshot(&path);
                    event_loop.exit();
                    return;
                }
                state.renderer.render(&state.scene, &state.camera);
                state.window.request_redraw();
            }
            _ => {}
        }
    }
}

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn,mini3d=info")).init();
    let event_loop = EventLoop::new().expect("event loop");
    event_loop.set_control_flow(ControlFlow::Poll);
    event_loop.run_app(&mut App::default()).expect("run app");
}
