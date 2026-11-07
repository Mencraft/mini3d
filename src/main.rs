mod engine;

use engine::{Image, Mesh, OrbitCamera, Renderer, Scene, Transform};
use glam::{Quat, Vec3};
use std::{sync::Arc, time::Instant};
use winit::{
    application::ApplicationHandler,
    event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent},
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop},
    keyboard::{Key, NamedKey},
    window::{Window, WindowId},
};

struct State {
    window: Arc<Window>,
    renderer: Renderer,
    scene: Scene,
    camera: OrbitCamera,
    spinning: Vec<usize>,
    start: Instant,
    paused_at: Option<f32>,
    dragging: bool,
    last_cursor: Option<(f64, f64)>,
}

impl State {
    fn new(window: Arc<Window>) -> Self {
        let renderer = pollster::block_on(Renderer::new(window.clone()));
        let mut scene = Scene::new(&renderer);

        let cube = scene.add_mesh(&renderer, &Mesh::cube(1.4));
        let sphere = scene.add_mesh(&renderer, &Mesh::sphere(0.8, 48, 32));
        let cylinder = scene.add_mesh(&renderer, &Mesh::cylinder(0.6, 1.6, 48));
        let cone = scene.add_mesh(&renderer, &Mesh::cone(0.75, 1.6, 48));
        let torus = scene.add_mesh(&renderer, &Mesh::torus(0.7, 0.25, 64, 24));
        let plane = scene.add_mesh(&renderer, &Mesh::plane(14.0, 1));

        let checker = scene.add_texture(&renderer, &Image::checker(256, 2, [230; 4], [90, 90, 90, 255]));
        let grid = scene.add_texture(&renderer, &Image::grid(256, 8, [40, 40, 40, 255], [255; 4]));
        // An image passed on the command line replaces the grid texture on the cube and torus.
        let custom = match std::env::args().nth(1) {
            Some(path) => match Image::from_file(&path) {
                Ok(img) => scene.add_texture(&renderer, &img),
                Err(e) => {
                    log::error!("could not load texture {path}: {e}");
                    grid
                }
            },
            None => grid,
        };

        let ground = scene.spawn_textured(&renderer, plane, Transform::at(Vec3::new(0.0, -0.8, 0.0)), checker, [0.35, 0.37, 0.4]);
        scene.objects[ground].uv_scale = glam::Vec2::splat(7.0);

        let shapes = [
            (cube, custom, [0.90, 0.30, 0.25], [1.0, 1.0]),
            (sphere, checker, [0.25, 0.60, 0.95], [8.0, 4.0]),
            (cylinder, grid, [0.30, 0.80, 0.45], [1.0, 1.0]),
            (cone, Scene::WHITE_TEXTURE, [0.95, 0.75, 0.20], [1.0, 1.0]),
            (torus, custom, [0.75, 0.40, 0.90], [4.0, 1.0]),
        ];
        let mut spinning = Vec::new();
        for (i, (mesh, texture, color, uv_scale)) in shapes.into_iter().enumerate() {
            let angle = i as f32 / shapes.len() as f32 * std::f32::consts::TAU;
            let pos = Vec3::new(angle.sin() * 3.0, 0.2, angle.cos() * 3.0);
            // Show a loaded image in its true colors rather than tinted.
            let color = if texture == custom && custom != grid { [1.0; 3] } else { color };
            let id = scene.spawn_textured(&renderer, mesh, Transform::at(pos), texture, color);
            scene.objects[id].uv_scale = uv_scale.into();
            spinning.push(id);
        }

        let camera = OrbitCamera::new(renderer.aspect());
        Self {
            window,
            renderer,
            scene,
            camera,
            spinning,
            start: Instant::now(),
            paused_at: None,
            dragging: false,
            last_cursor: None,
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
            .with_title("mini3d — drag to orbit, scroll to zoom, space to pause")
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
                match event.logical_key {
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
