# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project follows [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- **Physically based lighting**: Cook-Torrance GGX shading with a metallic-roughness `Material`
  (base color, texture, metallic, roughness, emissive, mirror, UV scale/offset).
- **Multiple light types**: directional, point and spot lights (`Light`), up to 16 at once, each with
  color, intensity, range and an on/off switch. Point and spot lights use windowed inverse-square falloff,
  and spot lights have soft cone edges.
- **Shadows**: a 2048² shadow map for the first shadow-casting directional light, with a 5×5 PCF
  soft edge, slope-scaled depth bias and normal offset. `Scene::shadow_center` / `shadow_radius` set the
  area it covers. Toggle with `Renderer::shadows`.
- **Reflections**:
  - Image-based lighting: every surface reflects the environment, blurrier as roughness increases.
  - Planar mirror: one object (`Material::mirror`) reflects the actual scene, using an extra render pass
    from a mirrored camera. Toggle with `Renderer::reflections`.
- **HDR rendering**: the scene renders into a 16-bit float (`Rgba16Float`) 4× MSAA target, then a
  tonemap pass applies `Renderer::exposure` and an ACES, Reinhard or clamp curve (`Tonemap`).
- **HDR environments**: `HdrImage` loads Radiance `.hdr` panoramas (or PNG/JPEG) and generates a
  procedural sky with an HDR sun. It is drawn as the sky and lights the scene. Use
  `Renderer::set_environment` and `Renderer::environment_intensity`.
- Emissive materials with values above 1, which stay bright through tone mapping.
- **Textures**: `Image` loads PNG/JPEG from files or memory, takes raw RGBA8 pixels, or generates
  solid, checker, grid and custom (`from_fn`) images. Mipmaps are generated on upload (averaged in
  linear color space) and sampled with trilinear and 16× anisotropic filtering.
- Texture coordinates on every primitive, oriented so images appear upright and unmirrored.
- `Renderer::screenshot` renders a frame offscreen and returns it as an `Image`, and `Image::save` writes it to a file.
- Demo: sun, orbiting point light with a glowing bulb, and a spotlight. Chrome, gold, copper, plastic and
  textured shapes on a reflective checkered floor. New keys: `1`/`2`/`3` toggle lights, `H` shadows,
  `R` reflections, `T` tonemap curve, `-`/`=` exposure, `[`/`]` sky intensity, `P` screenshot.
  The window title shows the current settings.
- Command-line options: an image path (texture for the cube), a `.hdr` path (environment),
  `--screenshot <file>` (save the first frame and quit) and `--keys <chars>` (press keys at startup).
- `Renderer::headless(width, height)` renders without a window, for tests and offline rendering.
  `Renderer::device()` and `Renderer::size()` are new accessors.
- The engine is now a library (`mini3d::engine`) and the demo is a binary that uses it, so tests and
  other programs can import the engine.
- 31 end-to-end GPU tests in `tests/render.rs` that render scenes headless and check pixels. They cover
  lights, shadows, reflections, HDR, textures, culling of every primitive, and GPU validation errors.
- Unit tests for mesh UVs and winding, mipmaps, image and HDR loading, the sky generator, lights,
  and the shadow and mirror matrices (21 in total).

### Changed

- `Scene::spawn` now takes a `Material` instead of an RGB color.
- `Scene::new` now takes `&Renderer`, which it uses to create the default white texture.
- `Scene::light_dir` is replaced by `Scene::lights` and `Scene::add_light`.
- The background is now the environment sky, so `Renderer::clear_color` is gone.
- `Vertex` has a `uv` field.
- Shaders moved to `src/engine/shaders/`: `pbr.wgsl`, `shadow.wgsl`, `sky.wgsl` and `tonemap.wgsl`.
- Sphere, cylinder, cone and torus now go around clockwise when seen from above, so textures
  aren't mirrored. Their geometry is otherwise unchanged.

### Removed

- The Blinn-Phong shader, replaced by the PBR shader.

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
