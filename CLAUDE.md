# CLAUDE.md

mini3d is a small real-time 3D engine in Rust: wgpu 26 (Metal on macOS) + winit 0.30, glam for math.
The engine is a library (`src/lib.rs` → `mini3d::engine`). `src/main.rs` is the demo app that uses it.
The README covers the features and public API.

## Commands

```sh
cargo build --release
cargo run --release                                   # demo window
cargo run --release -- --screenshot out.png           # render one frame to a PNG and quit
cargo run --release -- --keys "1h" --screenshot x.png # press keys first (see State::key in main.rs)
cargo test --release                                  # unit + end-to-end GPU tests (~1 s)
cargo test --release --test render <name-filter>
```

Use `--release` for tests too: in debug builds, generating the procedural sky and mipmaps on the CPU is slow.
The build must stay free of warnings.

## Layout

- `src/engine/renderer.rs`: all GPU work. Pipelines, bind groups, render targets, and the four
  passes per frame: shadow → planar reflection → main HDR (4× MSAA, `Rgba16Float`) → tonemap
  to the surface. Also `Transform`, `Tonemap`, `screenshot`, and `headless`.
- `src/engine/shaders/`: `pbr.wgsl` (lighting/shadows/reflections), `shadow.wgsl`, `sky.wgsl`,
  `tonemap.wgsl`.
- `scene.rs` (objects, meshes, textures, lights; ids are `usize` indices), `material.rs`,
  `light.rs`, `mesh.rs` (primitive generators), `texture.rs` (`Image`, 8-bit sRGB),
  `environment.rs` (`HdrImage`, equirectangular, linear float), `camera.rs` (orbit camera).
- `tests/render.rs`: end-to-end tests using `Renderer::headless`.

## Rules that are easy to break

- **Uniform structs exist twice.** Each `#[repr(C)]` uniform in `renderer.rs` (`ViewUniform`,
  `ObjectUniform`, `LightUniform`, `WorldUniform`, `TonemapUniform`) has a matching WGSL struct.
  `View`, `Object` and `World` are copied into several shaders. When changing one, update every copy.
  Use only `vec4`/`mat4x4` fields so the layouts match without padding rules. `MAX_LIGHTS` (16) is
  also hard-coded as `array<Light, 16>` in the WGSL.
- **Bind group slots**: lit pipeline = 0 view, 1 object, 2 albedo texture, 3 world. Sky = 0 view,
  1 world. Shadow = 0 view, 1 object. Changing a pipeline invalidates incompatible groups, so
  `draw_scene` rebinds after switching to the sky pipeline.
- **The reflection pass uses a mirrored camera**, which flips triangle winding. That's why
  `mirror_pipeline` uses `FrontFace::Cw`. It also clips fragments below the mirror plane
  (`view.clip`). A view must never bind a texture it is rendering into, so the mirror and shadow
  views bind a 1×1 dummy texture.
- **Winding**: mesh generators only need correct normals. `Mesh::fix_winding` makes every triangle
  counter-clockwise toward its normal. New primitives should end with `.fix_winding()`.
- **WGSL uniformity**: call `textureSample` before any `discard` or branch. Inside loops and branches,
  use `textureSampleLevel` / `textureSampleCompareLevel`.
- **Color spaces**: material colors, light colors and `HdrImage` are linear. `Image` pixels are sRGB,
  and the GPU decodes them (`Rgba8UnormSrgb`). The surface is sRGB, so the tonemap shader outputs linear.
- **HDR uploads** go through f16; `environment::upload` clamps to 65000 to avoid infinities.
- `Transform` has only uniform scale. The shader transforms normals with the model matrix.
- Light direction is the direction the light *travels* (the sun points down, e.g. `-Y`).
  Equirectangular mapping must match between `environment::equirect_direction` and `env_uv` in the
  shaders.

## Testing approach

- Unit tests live next to the code (`#[cfg(test)]`) and don't need a GPU.
- End-to-end tests render with `Fixture` in `tests/render.rs`. They start with a black environment,
  project world points to pixels with `Frame::at`, and assert behavior (brighter / darker / tinted),
  not exact images. `Fixture::render` fails on any wgpu validation error. Without a GPU, tests skip.
- Thresholds compare sRGB 0–255 values, so dark linear values look brighter than you'd expect
  (linear 0.04 ≈ 50/255). Prefer relative assertions.
- When adding a rendering feature, add an end-to-end test that fails if the feature is broken.
  Check it by temporarily breaking the code. A closed, uniformly emissive object can hide culling bugs.
- You can't see the demo window. To check visuals, render with `--screenshot` into the scratchpad
  and look at the PNG. `docs/screenshot.png` is the README image; regenerate it with
  `--screenshot` and `sips -Z 1280` when the look changes.

## Conventions

- Record user-visible changes in `CHANGELOG.md` under `## [Unreleased]` (Keep a Changelog format),
  and update `README.md` when the API, controls or features change.
- Public engine API uses builders for materials (`Material::color(..).metallic(..)`) and
  constructors for lights (`Light::spot(..)`). Keep `main.rs` demo-only; engine logic belongs in `src/engine/`.
- Git author for this repo: `mencraft <ekemboy@gmail.com>`. Commit only when asked.
