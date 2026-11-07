# mini3d

A small 3D engine in Rust that renders lit primitive shapes in real time. It uses
[wgpu](https://wgpu.rs) for graphics (Metal on macOS) and [winit](https://github.com/rust-windowing/winit)
for the window and input.

The demo scene shows a cube, sphere, cylinder, cone and torus spinning above a ground plane.

## Requirements

- macOS on Apple Silicon or Intel with Metal support
- Rust 1.85 or newer (2021 edition); tested with Rust 1.89

wgpu is cross-platform, so the engine should also build on Windows and Linux. Only macOS has been tested.

## Running

```sh
cargo run --release
```

### Controls

| Input              | Action              |
| ------------------ | ------------------- |
| Left-drag          | Orbit the camera    |
| Scroll             | Zoom in / out       |
| Space              | Pause / resume      |
| Esc                | Quit                |

Set `RUST_LOG=info` (the default for this crate) or `RUST_LOG=debug` to see more log output,
including which GPU adapter was selected.

## Features

- Procedural meshes: cube, plane, UV sphere, cylinder, cone, torus
- Automatic triangle-winding correction, so back-face culling works for every generator
- Blinn-Phong lighting: one directional light, ambient term, hemisphere sky fill, specular highlights
- Depth buffer and 4× MSAA anti-aliasing
- Orbit camera with clamped pitch and zoom
- Window resizing and surface-loss recovery

## Project layout

```
src/
├── main.rs              App loop, input handling, demo scene
└── engine/
    ├── mod.rs           Public re-exports
    ├── mesh.rs          Vertex format and primitive generators
    ├── camera.rs        Orbit camera
    ├── scene.rs         Meshes and objects (transform + color)
    ├── renderer.rs      wgpu setup, pipeline, render pass, Transform
    └── shader.wgsl      Vertex and fragment shaders
```

## Usage

Upload a mesh once, then spawn as many objects using it as you like:

```rust
use engine::{Mesh, Scene, Transform};
use glam::Vec3;

let sphere = scene.add_mesh(&renderer, &Mesh::sphere(0.5, 32, 16));
let id = scene.spawn(&renderer, sphere, Transform::at(Vec3::new(0.0, 2.0, 0.0)), [1.0, 0.5, 0.2]);

// Animate it later, e.g. in State::update:
scene.objects[id].transform.position.y += 0.01;
```

Available generators:

| Function                                   | Shape                          |
| ------------------------------------------ | ------------------------------ |
| `Mesh::cube(size)`                         | Axis-aligned cube              |
| `Mesh::plane(size, subdivisions)`          | Flat square facing +Y          |
| `Mesh::sphere(radius, sectors, stacks)`    | UV sphere                      |
| `Mesh::cylinder(radius, height, segments)` | Capped cylinder                |
| `Mesh::cone(radius, height, segments)`     | Cone with a base cap           |
| `Mesh::torus(major, minor, segments, sides)` | Torus (ring)                 |

Change the light direction with `scene.light_dir`, and the background with `renderer.clear_color`.

## Limitations

- `Transform` supports uniform scale only. The shader transforms normals with the model matrix,
  which is only correct without non-uniform scaling.
- One draw call and one uniform buffer per object, which is fine for small scenes but not for thousands of objects.
- No textures, shadows or multiple lights yet.

## License

No license has been chosen yet.
