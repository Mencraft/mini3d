//! Scene graph (flat): meshes and textures shared by index, objects with transform + material, lights.

use super::light::Light;
use super::material::Material;
use super::mesh::Mesh;
use super::renderer::{GpuMesh, ObjectGpu, Renderer, Transform};
use super::texture::{GpuTexture, Image};
use glam::Vec3;

pub type MeshId = usize;
pub type TextureId = usize;
pub type ObjectId = usize;
pub type LightId = usize;

pub struct Object {
    pub mesh: MeshId,
    pub transform: Transform,
    pub material: Material,
    pub(crate) gpu: ObjectGpu,
}

pub struct Scene {
    pub(crate) meshes: Vec<GpuMesh>,
    pub(crate) textures: Vec<GpuTexture>,
    pub objects: Vec<Object>,
    pub lights: Vec<Light>,
    /// The shadow map covers a sphere of `shadow_radius` around `shadow_center`. Smaller
    /// regions give sharper shadows; objects outside it don't receive shadows.
    pub shadow_center: Vec3,
    pub shadow_radius: f32,
}

impl Scene {
    /// A 1x1 white texture that leaves a material's color unchanged; the default.
    pub const WHITE_TEXTURE: TextureId = 0;

    pub fn new(renderer: &Renderer) -> Self {
        Self {
            meshes: Vec::new(),
            textures: vec![renderer.upload_texture(&Image::solid([255; 4]))],
            objects: Vec::new(),
            lights: Vec::new(),
            shadow_center: Vec3::ZERO,
            shadow_radius: 8.0,
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

    pub fn add_light(&mut self, light: Light) -> LightId {
        self.lights.push(light);
        self.lights.len() - 1
    }

    pub fn spawn(&mut self, renderer: &Renderer, mesh: MeshId, transform: Transform, material: Material) -> ObjectId {
        assert!(mesh < self.meshes.len(), "unknown mesh id {mesh}");
        assert!(material.texture < self.textures.len(), "unknown texture id {}", material.texture);
        self.objects.push(Object { mesh, transform, material, gpu: renderer.create_object_gpu() });
        self.objects.len() - 1
    }
}
