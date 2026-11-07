# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project follows [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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
