//! Physically based surface description.

use super::scene::TextureId;
use glam::{Vec2, Vec3};

/// Metallic-roughness material, as used by glTF and most real-time engines.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Material {
    /// Linear RGB, multiplied with `texture`.
    pub base_color: Vec3,
    /// Color texture; `Scene::WHITE_TEXTURE` for none.
    pub texture: TextureId,
    /// 0 = dielectric (plastic, wood, stone), 1 = metal.
    pub metallic: f32,
    /// 0 = mirror-smooth, 1 = completely matte.
    pub roughness: f32,
    /// Light the surface gives off, in linear HDR units; values above 1 glow brighter than white.
    pub emissive: Vec3,
    /// Strength (0–1) of the planar mirror reflection. The first object with a value
    /// above 0 becomes the scene's mirror, treated as a horizontal plane at its height.
    pub mirror: f32,
    /// How many times the texture repeats across the mesh.
    pub uv_scale: Vec2,
    /// Shifts the texture; animate it to scroll.
    pub uv_offset: Vec2,
}

impl Default for Material {
    fn default() -> Self {
        Self {
            base_color: Vec3::ONE,
            texture: 0,
            metallic: 0.0,
            roughness: 0.5,
            emissive: Vec3::ZERO,
            mirror: 0.0,
            uv_scale: Vec2::ONE,
            uv_offset: Vec2::ZERO,
        }
    }
}

impl Material {
    pub fn color(rgb: [f32; 3]) -> Self {
        Self { base_color: rgb.into(), ..Default::default() }
    }

    pub fn texture(mut self, texture: TextureId) -> Self {
        self.texture = texture;
        self
    }

    pub fn metallic(mut self, metallic: f32) -> Self {
        self.metallic = metallic.clamp(0.0, 1.0);
        self
    }

    pub fn roughness(mut self, roughness: f32) -> Self {
        self.roughness = roughness.clamp(0.0, 1.0);
        self
    }

    /// `strength` scales `rgb`; e.g. strength 8 makes a light bulb that blooms past white.
    pub fn emissive(mut self, rgb: [f32; 3], strength: f32) -> Self {
        self.emissive = Vec3::from(rgb) * strength;
        self
    }

    pub fn mirror(mut self, strength: f32) -> Self {
        self.mirror = strength.clamp(0.0, 1.0);
        self
    }

    pub fn uv_scale(mut self, u: f32, v: f32) -> Self {
        self.uv_scale = Vec2::new(u, v);
        self
    }
}
