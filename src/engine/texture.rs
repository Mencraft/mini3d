//! Textures: CPU-side RGBA8 images (loaded from disk or generated) and their GPU upload.

use std::path::Path;

/// An 8-bit sRGB RGBA image in memory, row-major, top row first.
#[derive(Clone, Debug, PartialEq)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

impl Image {
    pub fn new(width: u32, height: u32, pixels: Vec<u8>) -> Self {
        assert!(width > 0 && height > 0, "image must not be empty");
        assert_eq!(pixels.len(), (width * height * 4) as usize, "pixel data must be width * height * 4 bytes");
        Self { width, height, pixels }
    }

    /// Loads a PNG or JPEG file.
    pub fn from_file(path: impl AsRef<Path>) -> Result<Self, image::ImageError> {
        let img = image::open(path)?.to_rgba8();
        Ok(Self::new(img.width(), img.height(), img.into_raw()))
    }

    /// Decodes PNG or JPEG data already in memory (e.g. from `include_bytes!`).
    #[allow(dead_code)] // engine API, not used by the demo
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, image::ImageError> {
        let img = image::load_from_memory(bytes)?.to_rgba8();
        Ok(Self::new(img.width(), img.height(), img.into_raw()))
    }

    /// Builds an image by calling `f(x, y)` for every pixel.
    pub fn from_fn(width: u32, height: u32, f: impl Fn(u32, u32) -> [u8; 4]) -> Self {
        let mut pixels = Vec::with_capacity((width * height * 4) as usize);
        for y in 0..height {
            for x in 0..width {
                pixels.extend_from_slice(&f(x, y));
            }
        }
        Self::new(width, height, pixels)
    }

    pub fn solid(rgba: [u8; 4]) -> Self {
        Self::new(1, 1, rgba.to_vec())
    }

    /// A checkerboard with `cells` squares per side.
    pub fn checker(size: u32, cells: u32, a: [u8; 4], b: [u8; 4]) -> Self {
        let cell = (size / cells).max(1);
        Self::from_fn(size, size, |x, y| if (x / cell + y / cell) % 2 == 0 { a } else { b })
    }

    /// Grid lines over a background; handy for checking UV layout.
    pub fn grid(size: u32, cells: u32, line: [u8; 4], background: [u8; 4]) -> Self {
        let cell = (size / cells).max(1);
        let thickness = (size / 128).max(1);
        Self::from_fn(size, size, |x, y| {
            if x % cell < thickness || y % cell < thickness { line } else { background }
        })
    }

    /// Returns the full mip chain, starting with this image and halving down to 1x1.
    /// Filtering happens in linear space so mips don't darken.
    pub fn mip_chain(&self) -> Vec<Image> {
        let mut chain = vec![self.clone()];
        while let Some(last) = chain.last().filter(|i| i.width > 1 || i.height > 1) {
            chain.push(last.downsample());
        }
        chain
    }

    fn downsample(&self) -> Image {
        let (w, h) = ((self.width / 2).max(1), (self.height / 2).max(1));
        let lut: Vec<f32> = (0..256).map(|v| srgb_to_linear(v as f32 / 255.0)).collect();
        let at = |x: u32, y: u32, c: usize| {
            let (x, y) = (x.min(self.width - 1), y.min(self.height - 1));
            self.pixels[((y * self.width + x) * 4) as usize + c]
        };
        Image::from_fn(w, h, |x, y| {
            let (sx, sy) = (x * 2, y * 2);
            let mut out = [0u8; 4];
            for c in 0..4 {
                let samples = [at(sx, sy, c), at(sx + 1, sy, c), at(sx, sy + 1, c), at(sx + 1, sy + 1, c)];
                out[c] = if c == 3 {
                    // Alpha is linear already.
                    (samples.iter().map(|&s| s as u32).sum::<u32>() as f32 / 4.0).round() as u8
                } else {
                    let avg = samples.iter().map(|&s| lut[s as usize]).sum::<f32>() / 4.0;
                    (linear_to_srgb(avg) * 255.0).round() as u8
                };
            }
            out
        })
    }
}

fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
}

fn linear_to_srgb(c: f32) -> f32 {
    if c <= 0.0031308 { c * 12.92 } else { 1.055 * c.powf(1.0 / 2.4) - 0.055 }
}

/// A texture uploaded to the GPU, ready to bind to objects.
pub struct GpuTexture {
    #[allow(dead_code)] // kept alive for the view / bind group; also useful for later readback
    pub(crate) texture: wgpu::Texture,
    pub(crate) bind_group: wgpu::BindGroup,
}

pub(crate) fn upload(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    layout: &wgpu::BindGroupLayout,
    sampler: &wgpu::Sampler,
    image: &Image,
) -> GpuTexture {
    let mips = image.mip_chain();
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("texture"),
        size: wgpu::Extent3d { width: image.width, height: image.height, depth_or_array_layers: 1 },
        mip_level_count: mips.len() as u32,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8UnormSrgb,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    for (level, mip) in mips.iter().enumerate() {
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: level as u32,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &mip.pixels,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(mip.width * 4),
                rows_per_image: Some(mip.height),
            },
            wgpu::Extent3d { width: mip.width, height: mip.height, depth_or_array_layers: 1 },
        );
    }
    let view = texture.create_view(&Default::default());
    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("texture bg"),
        layout,
        entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) },
            wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(sampler) },
        ],
    });
    GpuTexture { texture, bind_group }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mip_chain_halves_to_one_pixel() {
        let img = Image::checker(64, 4, [255; 4], [0, 0, 0, 255]);
        let sizes: Vec<_> = img.mip_chain().iter().map(|m| (m.width, m.height)).collect();
        assert_eq!(sizes, [(64, 64), (32, 32), (16, 16), (8, 8), (4, 4), (2, 2), (1, 1)]);
    }

    #[test]
    fn mip_chain_handles_non_square_and_odd_sizes() {
        let img = Image::from_fn(5, 2, |_, _| [10, 20, 30, 255]);
        let sizes: Vec<_> = img.mip_chain().iter().map(|m| (m.width, m.height)).collect();
        assert_eq!(sizes, [(5, 2), (2, 1), (1, 1)]);
    }

    #[test]
    fn downsampling_averages_in_linear_space() {
        // Half black, half white: the linear average is 0.5, which is ~188 in sRGB, not 128.
        let img = Image::from_fn(2, 1, |x, _| if x == 0 { [0, 0, 0, 255] } else { [255; 4] });
        let mip = &img.mip_chain()[1];
        assert_eq!(mip.pixels[0], 188);
        assert_eq!(mip.pixels[3], 255);
    }

    #[test]
    fn solid_color_survives_mipmapping() {
        let img = Image::from_fn(8, 8, |_, _| [200, 100, 50, 255]);
        assert_eq!(img.mip_chain().last().unwrap().pixels, [200, 100, 50, 255]);
    }

    #[test]
    fn png_round_trip_through_file() {
        let path = std::env::temp_dir().join(format!("mini3d-test-{}.png", std::process::id()));
        let img = Image::checker(16, 2, [255, 0, 0, 255], [0, 0, 255, 255]);
        image::save_buffer(&path, &img.pixels, img.width, img.height, image::ColorType::Rgba8).unwrap();
        let loaded = Image::from_file(&path);
        std::fs::remove_file(&path).ok();
        assert_eq!(loaded.unwrap(), img);
    }

    #[test]
    fn missing_file_is_an_error() {
        assert!(Image::from_file("/definitely/not/here.png").is_err());
    }
}
