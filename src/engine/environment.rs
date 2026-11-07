//! HDR environment maps: the sky that is drawn behind the scene, lights it, and shows up
//! in reflections. Stored as equirectangular (latitude/longitude) images in linear RGB.

use super::texture::srgb_to_linear;
use glam::Vec3;
use std::f32::consts::{PI, TAU};
use std::path::Path;

/// A floating-point linear RGBA image, top row first. Values may go far above 1.
#[derive(Clone, Debug, PartialEq)]
pub struct HdrImage {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<[f32; 4]>,
}

/// World direction for an equirectangular texture coordinate (u, v in [0, 1], v = 0 is straight up).
/// Must match `env_uv` in the shaders.
pub fn equirect_direction(u: f32, v: f32) -> Vec3 {
    let phi = (u - 0.5) * TAU;
    let theta = v * PI;
    Vec3::new(theta.sin() * phi.cos(), theta.cos(), theta.sin() * phi.sin())
}

impl HdrImage {
    pub fn new(width: u32, height: u32, pixels: Vec<[f32; 4]>) -> Self {
        assert!(width > 0 && height > 0, "image must not be empty");
        assert_eq!(pixels.len(), (width * height) as usize, "need width * height pixels");
        Self { width, height, pixels }
    }

    pub fn from_fn(width: u32, height: u32, f: impl Fn(u32, u32) -> [f32; 4]) -> Self {
        let pixels = (0..height).flat_map(|y| (0..width).map(move |x| (x, y))).map(|(x, y)| f(x, y)).collect();
        Self::new(width, height, pixels)
    }

    /// Loads a Radiance `.hdr` file (linear HDR), or a PNG/JPEG (sRGB, converted to linear).
    /// The image should be an equirectangular panorama, twice as wide as it is tall.
    pub fn from_file(path: impl AsRef<Path>) -> Result<Self, image::ImageError> {
        let img = image::open(path)?;
        let is_float = matches!(img, image::DynamicImage::ImageRgb32F(_) | image::DynamicImage::ImageRgba32F(_));
        let rgba = img.to_rgba32f();
        let pixels = rgba
            .pixels()
            .map(|p| {
                let [r, g, b, a] = p.0;
                if is_float { [r, g, b, a] } else { [srgb_to_linear(r), srgb_to_linear(g), srgb_to_linear(b), a] }
            })
            .collect();
        Ok(Self::new(rgba.width(), rgba.height(), pixels))
    }

    /// A procedural daytime sky with a bright sun disc in `towards_sun` direction.
    pub fn sky(width: u32, height: u32, towards_sun: Vec3) -> Self {
        let sun = towards_sun.normalize();
        let zenith = Vec3::new(0.10, 0.25, 0.65);
        let horizon = Vec3::new(0.65, 0.72, 0.85);
        let ground = Vec3::new(0.10, 0.09, 0.08);
        Self::from_fn(width, height, |x, y| {
            let d = equirect_direction((x as f32 + 0.5) / width as f32, (y as f32 + 0.5) / height as f32);
            let mut c = if d.y >= 0.0 {
                horizon.lerp(zenith, d.y.sqrt())
            } else {
                (horizon * 0.5).lerp(ground, (-d.y).powf(0.4))
            };
            let cos = d.dot(sun).max(0.0);
            c += Vec3::new(1.0, 0.8, 0.6) * (cos.powf(48.0) * 0.8 + cos.powf(6.0) * 0.1);
            if cos > 0.9996 {
                c += Vec3::new(1.0, 0.95, 0.85) * 30.0; // the sun itself, far brighter than white
            }
            [c.x, c.y, c.z, 1.0]
        })
    }

    /// Full mip chain down to 1x1 (box filter). Blurrier levels stand in for rougher reflections.
    pub fn mip_chain(&self) -> Vec<HdrImage> {
        let mut chain = vec![self.clone()];
        while let Some(last) = chain.last().filter(|i| i.width > 1 || i.height > 1) {
            let (w, h) = ((last.width / 2).max(1), (last.height / 2).max(1));
            let at = |x: u32, y: u32| last.pixels[(y.min(last.height - 1) * last.width + x.min(last.width - 1)) as usize];
            let next = HdrImage::from_fn(w, h, |x, y| {
                let s = [at(x * 2, y * 2), at(x * 2 + 1, y * 2), at(x * 2, y * 2 + 1), at(x * 2 + 1, y * 2 + 1)];
                std::array::from_fn(|c| s.iter().map(|p| p[c]).sum::<f32>() / 4.0)
            });
            chain.push(next);
        }
        chain
    }
}

/// Uploads as an `Rgba16Float` texture with mips. Returns the view and the highest mip index.
pub(crate) fn upload(device: &wgpu::Device, queue: &wgpu::Queue, image: &HdrImage) -> (wgpu::TextureView, f32) {
    let mips = image.mip_chain();
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("environment"),
        size: wgpu::Extent3d { width: image.width, height: image.height, depth_or_array_layers: 1 },
        mip_level_count: mips.len() as u32,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba16Float,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    for (level, mip) in mips.iter().enumerate() {
        // f16 tops out at 65504; clamp so very bright pixels don't become infinity.
        let halves: Vec<u16> =
            mip.pixels.iter().flatten().map(|&v| half::f16::from_f32(v.clamp(0.0, 65000.0)).to_bits()).collect();
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: level as u32,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            bytemuck::cast_slice(&halves),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(mip.width * 8),
                rows_per_image: Some(mip.height),
            },
            wgpu::Extent3d { width: mip.width, height: mip.height, depth_or_array_layers: 1 },
        );
    }
    (texture.create_view(&Default::default()), (mips.len() - 1) as f32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn equirect_directions_hit_poles_and_axes() {
        assert!((equirect_direction(0.5, 0.0) - Vec3::Y).length() < 1e-5);
        assert!((equirect_direction(0.5, 1.0) + Vec3::Y).length() < 1e-5);
        assert!((equirect_direction(0.5, 0.5) - Vec3::X).length() < 1e-5);
        assert!((equirect_direction(0.75, 0.5) - Vec3::Z).length() < 1e-5);
    }

    #[test]
    fn sky_is_brightest_at_the_sun() {
        let sun = Vec3::new(0.3, 0.8, 0.2).normalize();
        let sky = HdrImage::sky(512, 256, sun);
        let brightest = sky.pixels.iter().enumerate().max_by(|a, b| a.1[0].total_cmp(&b.1[0])).unwrap();
        assert!(brightest.1[0] > 10.0, "sun should be HDR-bright");
        let (x, y) = (brightest.0 as u32 % 512, brightest.0 as u32 / 512);
        let dir = equirect_direction((x as f32 + 0.5) / 512.0, (y as f32 + 0.5) / 256.0);
        assert!(dir.dot(sun) > 0.999);
    }

    #[test]
    fn hdr_mips_preserve_average_energy() {
        let img = HdrImage::from_fn(8, 4, |x, _| [x as f32 * 10.0, 1.0, 0.0, 1.0]);
        let avg = img.pixels.iter().map(|p| p[0]).sum::<f32>() / img.pixels.len() as f32;
        let chain = img.mip_chain();
        assert_eq!(chain.len(), 4); // 8x4, 4x2, 2x1, 1x1
        assert!((chain.last().unwrap().pixels[0][0] - avg).abs() < 1e-4);
    }

    #[test]
    fn radiance_hdr_file_round_trip_keeps_values_above_one() {
        let path = std::env::temp_dir().join(format!("mini3d-test-{}.hdr", std::process::id()));
        let data: Vec<f32> = [[4.0f32, 2.0, 0.5], [0.25, 8.0, 1.0]].iter().flatten().copied().collect();
        image::save_buffer(&path, bytemuck::cast_slice(&data), 2, 1, image::ColorType::Rgb32F).unwrap();
        let loaded = HdrImage::from_file(&path);
        std::fs::remove_file(&path).ok();
        let loaded = loaded.unwrap();
        // Radiance files store 8-bit mantissas, so allow a little rounding.
        assert!((loaded.pixels[0][0] - 4.0).abs() < 0.05);
        assert!((loaded.pixels[1][1] - 8.0).abs() < 0.1);
    }
}
