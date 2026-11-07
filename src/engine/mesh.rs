//! CPU-side mesh data and procedural generators for primitive shapes.

use glam::Vec3;
use std::f32::consts::{PI, TAU};

#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Vertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
}

impl Vertex {
    pub const ATTRIBS: [wgpu::VertexAttribute; 2] =
        wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3];

    pub fn layout() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Vertex>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &Self::ATTRIBS,
        }
    }
}

#[derive(Default, Clone)]
pub struct Mesh {
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u32>,
}

impl Mesh {
    fn push(&mut self, p: Vec3, n: Vec3) -> u32 {
        self.vertices.push(Vertex { position: p.into(), normal: n.normalize_or_zero().into() });
        (self.vertices.len() - 1) as u32
    }

    /// Appends a (cols x rows) grid of vertices produced by `f(u, v)` with u, v in [0, 1].
    fn grid(&mut self, cols: u32, rows: u32, f: impl Fn(f32, f32) -> (Vec3, Vec3)) {
        let base = self.vertices.len() as u32;
        for r in 0..=rows {
            for c in 0..=cols {
                let (p, n) = f(c as f32 / cols as f32, r as f32 / rows as f32);
                self.push(p, n);
            }
        }
        let stride = cols + 1;
        for r in 0..rows {
            for c in 0..cols {
                let k1 = base + r * stride + c;
                let k2 = k1 + stride;
                self.indices.extend_from_slice(&[k1, k1 + 1, k2, k1 + 1, k2 + 1, k2]);
            }
        }
    }

    /// Appends a flat disc (triangle fan) facing `normal`.
    fn disc(&mut self, center: Vec3, radius: f32, normal: Vec3, segments: u32) {
        let c = self.push(center, normal);
        let first = self.vertices.len() as u32;
        for i in 0..=segments {
            let t = i as f32 / segments as f32 * TAU;
            self.push(center + Vec3::new(t.cos() * radius, 0.0, t.sin() * radius), normal);
        }
        for i in 0..segments {
            self.indices.extend_from_slice(&[c, first + i, first + i + 1]);
        }
    }

    /// Makes every triangle counter-clockwise when seen from the side its normals point to,
    /// so generators only need correct normals and back-face culling just works.
    fn fix_winding(mut self) -> Self {
        for tri in self.indices.chunks_exact_mut(3) {
            let [a, b, c] = [tri[0], tri[1], tri[2]].map(|i| self.vertices[i as usize]);
            let (pa, pb, pc) = (Vec3::from(a.position), Vec3::from(b.position), Vec3::from(c.position));
            let face = (pb - pa).cross(pc - pa);
            let avg = Vec3::from(a.normal) + Vec3::from(b.normal) + Vec3::from(c.normal);
            if face.dot(avg) < 0.0 {
                tri.swap(1, 2);
            }
        }
        self
    }

    // ---------------------------------------------------------------- primitives

    pub fn cube(size: f32) -> Self {
        let h = size * 0.5;
        let mut m = Mesh::default();
        for n in [Vec3::X, -Vec3::X, Vec3::Y, -Vec3::Y, Vec3::Z, -Vec3::Z] {
            let u = if n.y.abs() > 0.5 { Vec3::X } else { Vec3::Y };
            let v = n.cross(u);
            let base = m.vertices.len() as u32;
            for (su, sv) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
                m.push((n + u * su + v * sv) * h, n);
            }
            m.indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
        }
        m.fix_winding()
    }

    pub fn plane(size: f32, subdivisions: u32) -> Self {
        let mut m = Mesh::default();
        m.grid(subdivisions, subdivisions, |u, v| {
            (Vec3::new((u - 0.5) * size, 0.0, (v - 0.5) * size), Vec3::Y)
        });
        m.fix_winding()
    }

    pub fn sphere(radius: f32, sectors: u32, stacks: u32) -> Self {
        let mut m = Mesh::default();
        m.grid(sectors, stacks, |u, v| {
            let (theta, phi) = (u * TAU, v * PI);
            let n = Vec3::new(phi.sin() * theta.cos(), phi.cos(), phi.sin() * theta.sin());
            (n * radius, n)
        });
        m.fix_winding()
    }

    pub fn cylinder(radius: f32, height: f32, segments: u32) -> Self {
        let h = height * 0.5;
        let mut m = Mesh::default();
        m.grid(segments, 1, |u, v| {
            let t = u * TAU;
            let n = Vec3::new(t.cos(), 0.0, t.sin());
            (n * radius + Vec3::Y * (h - v * height), n)
        });
        m.disc(Vec3::Y * h, radius, Vec3::Y, segments);
        m.disc(-Vec3::Y * h, radius, -Vec3::Y, segments);
        m.fix_winding()
    }

    pub fn cone(radius: f32, height: f32, segments: u32) -> Self {
        let h = height * 0.5;
        let mut m = Mesh::default();
        m.grid(segments, 1, |u, v| {
            let t = u * TAU;
            let n = Vec3::new(t.cos() * height, radius, t.sin() * height);
            let r = radius * v; // v = 0 at the apex, 1 at the base
            (Vec3::new(t.cos() * r, h - v * height, t.sin() * r), n)
        });
        m.disc(-Vec3::Y * h, radius, -Vec3::Y, segments);
        m.fix_winding()
    }

    pub fn torus(major: f32, minor: f32, segments: u32, sides: u32) -> Self {
        let mut m = Mesh::default();
        m.grid(segments, sides, |u, v| {
            let (theta, phi) = (u * TAU, v * TAU);
            let n = Vec3::new(phi.cos() * theta.cos(), phi.sin(), phi.cos() * theta.sin());
            let center = Vec3::new(theta.cos(), 0.0, theta.sin()) * major;
            (center + n * minor, n)
        });
        m.fix_winding()
    }
}
