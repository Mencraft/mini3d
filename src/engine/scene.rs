//! Scene graph (flat): meshes and textures shared by index, objects with transform + material.

use super::mesh::Mesh;
use super::renderer::{GpuMesh, ObjectGpu, Renderer, Transform};
use super::texture::{GpuTexture, Image};
use glam::{Vec2, Vec3};

pub type MeshId = usize;
pub type TextureId = usize;
pub type ObjectId = usize;

pub struct Object {
    pub mesh: MeshId,
    pub transform: Transform,
    /// Linear RGBA tint, multiplied with the texture.
    pub color: [f32; 4],
    pub texture: TextureId,
    /// How many times the texture repeats across the mesh's UV range.
    pub uv_scale: Vec2,
    /// Shifts the texture; animate it to scroll.
    pub uv_offset: Vec2,
    pub(crate) gpu: ObjectGpu,
}

pub struct Scene {
    pub(crate) meshes: Vec<GpuMesh>,
    pub(crate) textures: Vec<GpuTexture>,
    pub objects: Vec<Object>,
    /// Direction the light travels (from the light toward the scene).
    pub light_dir: Vec3,
}

impl Scene {
    /// A 1x1 white texture that leaves an object's color unchanged; used by `spawn`.
    pub const WHITE_TEXTURE: TextureId = 0;

    pub fn new(renderer: &Renderer) -> Self {
        Self {
            meshes: Vec::new(),
            textures: vec![renderer.upload_texture(&Image::solid([255; 4]))],
            objects: Vec::new(),
            light_dir: Vec3::new(-0.5, -1.0, -0.4),
        }
    }

    pub fn add_mesh(&mut self, renderer: &Renderer, mesh: &Mesh) -> MeshId {
        self.meshes.push(renderer.upload_mesh(mesh));
        self.meshes.len() - 1
    }

    pub fn add_texture(&mut self, renderer: &Renderer, image: &Image) -> TextureId {
        self.textures.push(renderer.upload_texture(image));
        self.textures.len() - 1
    }

    /// Spawns an untextured object with a solid color.
    #[allow(dead_code)] // engine API; the demo textures everything
    pub fn spawn(&mut self, renderer: &Renderer, mesh: MeshId, transform: Transform, rgb: [f32; 3]) -> ObjectId {
        self.spawn_textured(renderer, mesh, transform, Self::WHITE_TEXTURE, rgb)
    }

    /// Spawns an object whose texture is tinted by `rgb` (use `[1.0; 3]` for no tint).
    pub fn spawn_textured(
        &mut self,
        renderer: &Renderer,
        mesh: MeshId,
        transform: Transform,
        texture: TextureId,
        rgb: [f32; 3],
    ) -> ObjectId {
        assert!(texture < self.textures.len(), "unknown texture id {texture}");
        self.objects.push(Object {
            mesh,
            transform,
            color: [rgb[0], rgb[1], rgb[2], 1.0],
            texture,
            uv_scale: Vec2::ONE,
            uv_offset: Vec2::ZERO,
            gpu: renderer.create_object_gpu(),
        });
        self.objects.len() - 1
    }
}
