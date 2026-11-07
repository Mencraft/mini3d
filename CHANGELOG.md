# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project follows [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Texture system: `Image` loads PNG/JPEG files or memory, accepts raw RGBA8 pixels, or generates
  solid, checker, grid and custom (`from_fn`) images.
- `Scene::add_texture` and `Scene::spawn_textured`; objects have `texture`, `uv_scale` and `uv_offset`.
- Mipmaps generated on upload (averaged in linear color space), with trilinear and 16× anisotropic filtering.
- Texture coordinates on every primitive, oriented so images appear upright and unmirrored.
- The demo has a checkered ground, textured shapes, and an optional image path argument for the cube and torus.
- Unit tests for mesh UVs and winding, mipmap generation, and image loading.

### Changed

- `Scene::new` now takes `&Renderer`, which it uses to create the default white texture.
- `Vertex` has a `uv` field, and the shader multiplies each object's color by its texture.
- Sphere, cylinder, cone and torus now go around clockwise when seen from above, so textures
  aren't mirrored. Their geometry is otherwise unchanged.

## [0.1.0] - 2026-11-07

### Added

- wgpu renderer (Metal backend on macOS) with depth buffer and 4× MSAA.
- Blinn-Phong shader with a directional light, ambient term, hemisphere sky fill and specular highlights.
- Procedural mesh generators: cube, plane, UV sphere, cylinder, cone and torus.
- Automatic triangle-winding correction so back-face culling works for every generated mesh.
- `Scene` with shared GPU meshes and objects that each have a transform and color.
- `Transform` with position, rotation and uniform scale.
- Orbit camera: left-drag to rotate, scroll to zoom.
- Demo scene with five animated primitives above a ground plane.
- Keyboard controls: Space pauses and resumes the animation, Esc quits.
- Window resize handling and recovery from lost or outdated surfaces.
