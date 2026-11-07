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

To put your own image on the cube and torus, pass a PNG or JPEG path:

```sh
cargo run --release -- path/to/image.png
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

- Procedural meshes: cube, plane, UV sphere, cylinder, cone, torus, all with texture coordinates
- Textures: load PNG/JPEG files or generate them (solid, checker, grid, or any `f(x, y)`), with
  per-object color tint, tiling and scrolling
- Mipmaps generated automatically (filtered in linear color space), trilinear + 16× anisotropic filtering
- Automatic triangle-winding correction, so back-face culling works for every generator
- Blinn-Phong lighting: one directional light, ambient term, hemisphere sky fill, specular highlights
- Depth buffer and 4× MSAA anti-aliasing
- Orbit camera with clamped pitch and zoom
- Window resizing and surface-loss recovery

## Project layout

```text
src/
├── main.rs              App loop, input handling, demo scene
└── engine/
    ├── mod.rs           Public re-exports
    ├── mesh.rs          Vertex format and primitive generators
    ├── camera.rs        Orbit camera
    ├── scene.rs         Meshes, textures and objects (transform + material)
    ├── texture.rs       Image loading/generation, mipmaps, GPU upload
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

### Textures

Add a texture to the scene once, then use it on any number of objects:

```rust
use engine::Image;

let brick = scene.add_texture(&renderer, &Image::from_file("assets/brick.png")?);
let id = scene.spawn_textured(&renderer, cube, Transform::at(Vec3::ZERO), brick, [1.0, 1.0, 1.0]);

scene.objects[id].uv_scale = Vec2::new(4.0, 4.0); // repeat 4× in each direction
scene.objects[id].uv_offset.x += 0.01;           // scroll (e.g. every frame)
scene.objects[id].texture = Scene::WHITE_TEXTURE; // swap textures at any time
```

The object's color multiplies the texture, so `[1.0, 1.0, 1.0]` shows the image as-is.
`scene.spawn` uses a plain white texture, so untextured objects show their color unchanged.

| Function                                     | Image                                 |
| -------------------------------------------- | ------------------------------------- |
| `Image::from_file(path)`                     | PNG or JPEG from disk                 |
| `Image::from_bytes(bytes)`                   | PNG or JPEG in memory (`include_bytes!`) |
| `Image::new(width, height, rgba_pixels)`     | Raw RGBA8 pixel data                  |
| `Image::from_fn(width, height, \|x, y\| rgba)` | Generated pixel by pixel              |
| `Image::solid(rgba)`                         | 1×1 single color                      |
| `Image::checker(size, cells, a, b)`          | Checkerboard                          |
| `Image::grid(size, cells, line, background)` | Grid lines, good for checking UVs     |

Pixel data is 8-bit sRGB RGBA, top row first. Each primitive's UVs map the image upright
and unmirrored when you look at the surface from outside. Cylinders, cones, spheres and tori
wrap the image once around their circumference.

## Limitations

- `Transform` supports uniform scale only. The shader transforms normals with the model matrix,
  which is only correct without non-uniform scaling.
- One draw call and one uniform buffer per object, which is fine for small scenes but not for thousands of objects.
- One albedo (color) texture per object. No normal maps, transparency, shadows or multiple lights yet.

## License

No license has been chosen yet.
