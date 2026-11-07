//! Scene graph (flat): meshes shared by index, objects with transform + color.

use super::mesh::Mesh;
use super::renderer::{GpuMesh, ObjectGpu, Renderer, Transform};
use glam::Vec3;

pub type MeshId = usize;
pub type ObjectId = usize;

pub struct Object {
    pub mesh: MeshId,
    pub transform: Transform,
    pub color: [f32; 4],
    pub(crate) gpu: ObjectGpu,
}

pub struct Scene {
    pub(crate) meshes: Vec<GpuMesh>,
    pub objects: Vec<Object>,
    /// Direction the light travels (from the light toward the scene).
    pub light_dir: Vec3,
}

impl Scene {
    pub fn new() -> Self {
        Self { meshes: Vec::new(), objects: Vec::new(), light_dir: Vec3::new(-0.5, -1.0, -0.4) }
    }

    pub fn add_mesh(&mut self, renderer: &Renderer, mesh: &Mesh) -> MeshId {
        self.meshes.push(renderer.upload_mesh(mesh));
        self.meshes.len() - 1
    }

    pub fn spawn(&mut self, renderer: &Renderer, mesh: MeshId, transform: Transform, rgb: [f32; 3]) -> ObjectId {
        self.objects.push(Object {
            mesh,
            transform,
            color: [rgb[0], rgb[1], rgb[2], 1.0],
            gpu: renderer.create_object_gpu(),
        });
        self.objects.len() - 1
    }
}
