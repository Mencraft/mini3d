//! wgpu renderer (Metal on macOS).
//!
//! Each frame runs up to four passes:
//! 1. **Shadow** – depth from the sun's point of view into a shadow map.
//! 2. **Reflection** – the scene mirrored about the mirror plane, into an HDR texture.
//! 3. **Main** – PBR shading + sky into a 4× MSAA HDR (`Rgba16Float`) target.
//! 4. **Tonemap** – exposure + tone curve from HDR to the sRGB window surface.

use super::environment::{self, HdrImage};
use super::light::{LightKind, MAX_LIGHTS};
use super::texture::{self, GpuTexture, Image};
use super::{camera::OrbitCamera, mesh::Mesh, mesh::Vertex, scene::Scene};
use bytemuck::Zeroable;
use glam::{Mat4, Quat, Vec3};
use std::sync::Arc;
use wgpu::util::DeviceExt;
use winit::window::Window;

const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
const HDR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
const SAMPLES: u32 = 4;
const SHADOW_MAP_SIZE: u32 = 2048;

// ---------------------------------------------------------------- GPU data layouts (match the WGSL)

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct ViewUniform {
    view_proj: [[f32; 4]; 4],
    inv_view_proj: [[f32; 4]; 4],
    eye: [f32; 4],
    clip: [f32; 4],
    viewport: [f32; 4],
}

impl ViewUniform {
    fn new(view_proj: Mat4, eye: Vec3, clip: [f32; 4], viewport: [f32; 4]) -> Self {
        Self {
            view_proj: view_proj.to_cols_array_2d(),
            inv_view_proj: view_proj.inverse().to_cols_array_2d(),
            eye: eye.extend(1.0).into(),
            clip,
            viewport,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct ObjectUniform {
    model: [[f32; 4]; 4],
    base_color: [f32; 4],
    uv_transform: [f32; 4],
    material: [f32; 4],
    emissive: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct LightUniform {
    position: [f32; 4],
    direction: [f32; 4],
    color: [f32; 4],
    params: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct WorldUniform {
    lights: [LightUniform; MAX_LIGHTS],
    shadow_view_proj: [[f32; 4]; 4],
    params: [f32; 4],
    shadow_params: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct TonemapUniform {
    params: [f32; 4],
}

// ---------------------------------------------------------------- public types

/// How HDR colors are squeezed into the screen's 0–1 range.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tonemap {
    /// Filmic curve: rich contrast, highlights roll off smoothly. The default.
    Aces,
    /// Simple `x / (1 + x)`: soft and low-contrast, never clips.
    Reinhard,
    /// No curve: everything above 1 clips to white. Shows what HDR is buying you.
    Clamp,
}

impl Tonemap {
    pub fn next(self) -> Self {
        match self {
            Tonemap::Aces => Tonemap::Reinhard,
            Tonemap::Reinhard => Tonemap::Clamp,
            Tonemap::Clamp => Tonemap::Aces,
        }
    }
}

pub struct GpuMesh {
    vertex_buffer: wgpu::Buffer,
    index_buffer: wgpu::Buffer,
    index_count: u32,
}

/// GPU resources owned by one scene object.
pub struct ObjectGpu {
    buffer: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
}

/// Position / rotation / uniform scale.
#[derive(Clone, Copy, Debug)]
pub struct Transform {
    pub position: Vec3,
    pub rotation: Quat,
    pub scale: f32,
}

impl Transform {
    pub fn at(position: Vec3) -> Self {
        Self { position, rotation: Quat::IDENTITY, scale: 1.0 }
    }

    pub fn matrix(&self) -> Mat4 {
        Mat4::from_scale_rotation_translation(Vec3::splat(self.scale), self.rotation, self.position)
    }
}

// ---------------------------------------------------------------- renderer

/// A camera's uniform buffer and bind group (main, mirrored, or the shadow light).
struct ViewGpu {
    buffer: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
}

/// Window-sized render targets, recreated on resize.
struct Targets {
    depth: wgpu::TextureView,
    msaa_hdr: wgpu::TextureView,
    hdr: wgpu::TextureView,
    reflection: wgpu::TextureView,
    reflection_depth: wgpu::TextureView,
}

impl Targets {
    fn new(device: &wgpu::Device, width: u32, height: u32) -> Self {
        let make = |label, format, samples, usage| {
            device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some(label),
                    size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
                    mip_level_count: 1,
                    sample_count: samples,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage,
                    view_formats: &[],
                })
                .create_view(&Default::default())
        };
        let attach = wgpu::TextureUsages::RENDER_ATTACHMENT;
        let attach_and_sample = attach | wgpu::TextureUsages::TEXTURE_BINDING;
        Self {
            depth: make("depth", DEPTH_FORMAT, SAMPLES, attach),
            msaa_hdr: make("msaa hdr", HDR_FORMAT, SAMPLES, attach),
            hdr: make("hdr", HDR_FORMAT, 1, attach_and_sample),
            reflection: make("reflection", HDR_FORMAT, 1, attach_and_sample),
            reflection_depth: make("reflection depth", DEPTH_FORMAT, 1, attach),
        }
    }
}

pub struct Renderer {
    /// `None` for a headless renderer (tests, offline rendering).
    surface: Option<wgpu::Surface<'static>>,
    pub(crate) device: wgpu::Device,
    pub(crate) queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,

    lit_pipeline: wgpu::RenderPipeline,
    mirror_pipeline: wgpu::RenderPipeline,
    shadow_pipeline: wgpu::RenderPipeline,
    sky_pipeline: wgpu::RenderPipeline,
    sky_mirror_pipeline: wgpu::RenderPipeline,
    tonemap_pipeline: wgpu::RenderPipeline,

    view_layout: wgpu::BindGroupLayout,
    object_layout: wgpu::BindGroupLayout,
    texture_layout: wgpu::BindGroupLayout,
    world_layout: wgpu::BindGroupLayout,
    tonemap_layout: wgpu::BindGroupLayout,

    albedo_sampler: wgpu::Sampler,
    linear_sampler: wgpu::Sampler,
    shadow_sampler: wgpu::Sampler,
    env_sampler: wgpu::Sampler,

    targets: Targets,
    shadow_map: wgpu::TextureView,
    env_view: wgpu::TextureView,
    env_max_lod: f32,

    main_view: ViewGpu,
    mirror_view: ViewGpu,
    shadow_view: ViewGpu,
    world_buffer: wgpu::Buffer,
    world_bind_group: wgpu::BindGroup,
    tonemap_buffer: wgpu::Buffer,
    tonemap_bind_group: wgpu::BindGroup,

    /// Brightness multiplier applied before tone mapping (1.0 = unchanged).
    pub exposure: f32,
    pub tonemap: Tonemap,
    /// Multiplies the sky and all light coming from it.
    pub environment_intensity: f32,
    pub shadows: bool,
    pub reflections: bool,
}

// ---------------------------------------------------------------- bind group layout helpers

fn uniform_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

fn texture_entry(binding: u32, sample_type: wgpu::TextureSampleType) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type,
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

fn sampler_entry(binding: u32, ty: wgpu::SamplerBindingType) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry { binding, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Sampler(ty), count: None }
}

const FLOAT: wgpu::TextureSampleType = wgpu::TextureSampleType::Float { filterable: true };

fn uniform_buffer(device: &wgpu::Device, label: &str, size: usize) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size: size as u64,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

fn view_bind_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    buffer: &wgpu::Buffer,
    reflection: &wgpu::TextureView,
    sampler: &wgpu::Sampler,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("view bg"),
        layout,
        entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: buffer.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(reflection) },
            wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(sampler) },
        ],
    })
}

fn tonemap_bind_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    hdr: &wgpu::TextureView,
    sampler: &wgpu::Sampler,
    buffer: &wgpu::Buffer,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("tonemap bg"),
        layout,
        entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(hdr) },
            wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(sampler) },
            wgpu::BindGroupEntry { binding: 2, resource: buffer.as_entire_binding() },
        ],
    })
}

/// Orthographic light-space matrix covering a sphere of `radius` around `center`.
fn shadow_matrix(direction: Vec3, center: Vec3, radius: f32) -> Mat4 {
    let d = direction.normalize();
    let up = if d.y.abs() > 0.99 { Vec3::Z } else { Vec3::Y };
    let view = Mat4::look_at_rh(center - d * radius * 2.0, center, up);
    let proj = Mat4::orthographic_rh(-radius, radius, -radius, radius, 0.0, radius * 4.0);
    proj * view
}

/// Reflection across the horizontal plane y = `height`.
fn mirror_matrix(height: f32) -> Mat4 {
    Mat4::from_translation(Vec3::Y * 2.0 * height) * Mat4::from_scale(Vec3::new(1.0, -1.0, 1.0))
}

impl Renderer {
    pub async fn new(window: Arc<Window>) -> Self {
        let size = window.inner_size();
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::default());
        let surface = instance.create_surface(window).expect("create surface");
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: Some(&surface),
                force_fallback_adapter: false,
            })
            .await
            .expect("no suitable GPU adapter");
        let caps = surface.get_capabilities(&adapter);
        let format = caps.formats.iter().copied().find(|f| f.is_srgb()).unwrap_or(caps.formats[0]);
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width: size.width.max(1),
            height: size.height.max(1),
            present_mode: wgpu::PresentMode::AutoVsync,
            alpha_mode: caps.alpha_modes[0],
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };
        Self::init(adapter, Some(surface), config).await
    }

    /// A renderer without a window, for tests and offline rendering. Draw with
    /// [`Renderer::screenshot`]. Returns `None` if the machine has no usable GPU.
    pub async fn headless(width: u32, height: u32) -> Option<Self> {
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::default());
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: None,
                force_fallback_adapter: false,
            })
            .await
            .ok()?;
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            width: width.max(1),
            height: height.max(1),
            present_mode: wgpu::PresentMode::Fifo,
            alpha_mode: wgpu::CompositeAlphaMode::Auto,
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };
        Some(Self::init(adapter, None, config).await)
    }

    /// The GPU device, e.g. for `push_error_scope` in tests.
    pub fn device(&self) -> &wgpu::Device {
        &self.device
    }

    pub fn size(&self) -> (u32, u32) {
        (self.config.width, self.config.height)
    }

    async fn init(adapter: wgpu::Adapter, surface: Option<wgpu::Surface<'static>>, config: wgpu::SurfaceConfiguration) -> Self {
        log::info!("Using adapter: {:?}", adapter.get_info());
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor::default())
            .await
            .expect("request device");
        if let Some(surface) = &surface {
            surface.configure(&device, &config);
        }
        let format = config.format;

        // ---- layouts
        use wgpu::SamplerBindingType as S;
        let layout = |label, entries: &[wgpu::BindGroupLayoutEntry]| {
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some(label), entries })
        };
        let view_layout = layout("view", &[uniform_entry(0), texture_entry(1, FLOAT), sampler_entry(2, S::Filtering)]);
        let object_layout = layout("object", &[uniform_entry(0)]);
        let texture_layout = layout("albedo", &[texture_entry(0, FLOAT), sampler_entry(1, S::Filtering)]);
        let world_layout = layout(
            "world",
            &[
                uniform_entry(0),
                texture_entry(1, wgpu::TextureSampleType::Depth),
                sampler_entry(2, S::Comparison),
                texture_entry(3, FLOAT),
                sampler_entry(4, S::Filtering),
            ],
        );
        let tonemap_layout = layout("tonemap", &[texture_entry(0, FLOAT), sampler_entry(1, S::Filtering), uniform_entry(2)]);

        // ---- samplers
        // Albedo: repeat for tiling; trilinear + anisotropic keeps grazing-angle surfaces sharp.
        let albedo_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("albedo sampler"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Linear,
            anisotropy_clamp: 16,
            ..Default::default()
        });
        let linear_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("linear clamp"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let shadow_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("shadow sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            compare: Some(wgpu::CompareFunction::LessEqual),
            ..Default::default()
        });
        // Environment: wraps horizontally around the panorama, clamps at the poles.
        let env_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("environment sampler"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        // ---- pipelines
        let pbr = device.create_shader_module(wgpu::include_wgsl!("shaders/pbr.wgsl"));
        let shadow = device.create_shader_module(wgpu::include_wgsl!("shaders/shadow.wgsl"));
        let sky = device.create_shader_module(wgpu::include_wgsl!("shaders/sky.wgsl"));
        let tonemap = device.create_shader_module(wgpu::include_wgsl!("shaders/tonemap.wgsl"));

        let pipeline_layout = |label, layouts: &[&wgpu::BindGroupLayout]| {
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some(label),
                bind_group_layouts: layouts,
                push_constant_ranges: &[],
            })
        };
        let lit_layout = pipeline_layout("lit", &[&view_layout, &object_layout, &texture_layout, &world_layout]);
        let shadow_layout = pipeline_layout("shadow", &[&view_layout, &object_layout]);
        let sky_layout = pipeline_layout("sky", &[&view_layout, &world_layout]);
        let tonemap_pipeline_layout = pipeline_layout("tonemap", &[&tonemap_layout]);

        let color_target = |format| {
            [Some(wgpu::ColorTargetState { format, blend: None, write_mask: wgpu::ColorWrites::ALL })]
        };
        let hdr_targets = color_target(HDR_FORMAT);
        let depth_state = |write, compare| {
            Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: write,
                depth_compare: compare,
                stencil: Default::default(),
                bias: Default::default(),
            })
        };

        // The mirrored camera flips handedness, so the reflection pass treats clockwise as front.
        let lit = |samples, front_face| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("lit pipeline"),
                layout: Some(&lit_layout),
                vertex: wgpu::VertexState {
                    module: &pbr,
                    entry_point: Some("vs_main"),
                    compilation_options: Default::default(),
                    buffers: &[Vertex::layout()],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &pbr,
                    entry_point: Some("fs_main"),
                    compilation_options: Default::default(),
                    targets: &hdr_targets,
                }),
                primitive: wgpu::PrimitiveState { front_face, cull_mode: Some(wgpu::Face::Back), ..Default::default() },
                depth_stencil: depth_state(true, wgpu::CompareFunction::Less),
                multisample: wgpu::MultisampleState { count: samples, ..Default::default() },
                multiview: None,
                cache: None,
            })
        };
        let lit_pipeline = lit(SAMPLES, wgpu::FrontFace::Ccw);
        let mirror_pipeline = lit(1, wgpu::FrontFace::Cw);

        // Sky sits exactly on the far plane (depth 1.0), behind everything already drawn.
        let sky_pipe = |samples| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("sky pipeline"),
                layout: Some(&sky_layout),
                vertex: wgpu::VertexState {
                    module: &sky,
                    entry_point: Some("vs_main"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &sky,
                    entry_point: Some("fs_main"),
                    compilation_options: Default::default(),
                    targets: &hdr_targets,
                }),
                primitive: Default::default(),
                depth_stencil: depth_state(false, wgpu::CompareFunction::LessEqual),
                multisample: wgpu::MultisampleState { count: samples, ..Default::default() },
                multiview: None,
                cache: None,
            })
        };
        let sky_pipeline = sky_pipe(SAMPLES);
        let sky_mirror_pipeline = sky_pipe(1);

        let shadow_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("shadow pipeline"),
            layout: Some(&shadow_layout),
            vertex: wgpu::VertexState {
                module: &shadow,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[Vertex::layout()],
            },
            fragment: None,
            primitive: wgpu::PrimitiveState { cull_mode: Some(wgpu::Face::Back), ..Default::default() },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: true,
                depth_compare: wgpu::CompareFunction::Less,
                stencil: Default::default(),
                // Slope-scaled bias fights shadow acne on surfaces angled away from the light.
                bias: wgpu::DepthBiasState { constant: 2, slope_scale: 2.0, clamp: 0.0 },
            }),
            multisample: Default::default(),
            multiview: None,
            cache: None,
        });

        let surface_targets = color_target(format);
        let tonemap_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("tonemap pipeline"),
            layout: Some(&tonemap_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &tonemap,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &tonemap,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &surface_targets,
            }),
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            multiview: None,
            cache: None,
        });

        // ---- resources
        let targets = Targets::new(&device, config.width, config.height);
        let shadow_map = device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("shadow map"),
                size: wgpu::Extent3d { width: SHADOW_MAP_SIZE, height: SHADOW_MAP_SIZE, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: DEPTH_FORMAT,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            })
            .create_view(&Default::default());
        // Stand-in for the reflection texture in views that don't use it (avoids binding a
        // texture that the same pass is rendering into).
        let dummy = device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("dummy"),
                size: wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: HDR_FORMAT,
                usage: wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            })
            .create_view(&Default::default());

        let view_size = std::mem::size_of::<ViewUniform>();
        let make_view = |label, reflection: &wgpu::TextureView| {
            let buffer = uniform_buffer(&device, label, view_size);
            let bind_group = view_bind_group(&device, &view_layout, &buffer, reflection, &linear_sampler);
            ViewGpu { buffer, bind_group }
        };
        let main_view = make_view("main view", &targets.reflection);
        let mirror_view = make_view("mirror view", &dummy);
        let shadow_view = make_view("shadow view", &dummy);

        let world_buffer = uniform_buffer(&device, "world", std::mem::size_of::<WorldUniform>());
        let tonemap_buffer = uniform_buffer(&device, "tonemap", std::mem::size_of::<TonemapUniform>());
        let tonemap_bind_group = tonemap_bind_group(&device, &tonemap_layout, &targets.hdr, &linear_sampler, &tonemap_buffer);

        let default_sky = HdrImage::sky(1024, 512, Vec3::new(0.5, 1.0, 0.4));
        let (env_view, env_max_lod) = environment::upload(&device, &queue, &default_sky);
        let world_bind_group = world_bind_group(
            &device,
            &world_layout,
            &world_buffer,
            &shadow_map,
            &shadow_sampler,
            &env_view,
            &env_sampler,
        );

        Self {
            surface,
            device,
            queue,
            config,
            lit_pipeline,
            mirror_pipeline,
            shadow_pipeline,
            sky_pipeline,
            sky_mirror_pipeline,
            tonemap_pipeline,
            view_layout,
            object_layout,
            texture_layout,
            world_layout,
            tonemap_layout,
            albedo_sampler,
            linear_sampler,
            shadow_sampler,
            env_sampler,
            targets,
            shadow_map,
            env_view,
            env_max_lod,
            main_view,
            mirror_view,
            shadow_view,
            world_bind_group,
            world_buffer,
            tonemap_buffer,
            tonemap_bind_group,
            exposure: 1.0,
            tonemap: Tonemap::Aces,
            environment_intensity: 1.0,
            shadows: true,
            reflections: true,
        }
    }

    pub fn aspect(&self) -> f32 {
        self.config.width as f32 / self.config.height as f32
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        self.config.width = width;
        self.config.height = height;
        if let Some(surface) = &self.surface {
            surface.configure(&self.device, &self.config);
        }
        self.targets = Targets::new(&self.device, width, height);
        self.main_view.bind_group = view_bind_group(
            &self.device,
            &self.view_layout,
            &self.main_view.buffer,
            &self.targets.reflection,
            &self.linear_sampler,
        );
        self.tonemap_bind_group = tonemap_bind_group(
            &self.device,
            &self.tonemap_layout,
            &self.targets.hdr,
            &self.linear_sampler,
            &self.tonemap_buffer,
        );
    }

    /// Replaces the sky / reflection environment (an equirectangular panorama).
    pub fn set_environment(&mut self, image: &HdrImage) {
        (self.env_view, self.env_max_lod) = environment::upload(&self.device, &self.queue, image);
        self.world_bind_group = world_bind_group(
            &self.device,
            &self.world_layout,
            &self.world_buffer,
            &self.shadow_map,
            &self.shadow_sampler,
            &self.env_view,
            &self.env_sampler,
        );
    }

    pub fn upload_mesh(&self, mesh: &Mesh) -> GpuMesh {
        let vertex_buffer = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("vertices"),
            contents: bytemuck::cast_slice(&mesh.vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let index_buffer = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("indices"),
            contents: bytemuck::cast_slice(&mesh.indices),
            usage: wgpu::BufferUsages::INDEX,
        });
        GpuMesh { vertex_buffer, index_buffer, index_count: mesh.indices.len() as u32 }
    }

    /// Uploads an image (with a generated mip chain) so objects can use it.
    pub fn upload_texture(&self, image: &Image) -> GpuTexture {
        texture::upload(&self.device, &self.queue, &self.texture_layout, &self.albedo_sampler, image)
    }

    pub(crate) fn create_object_gpu(&self) -> ObjectGpu {
        let buffer = uniform_buffer(&self.device, "object", std::mem::size_of::<ObjectUniform>());
        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("object bg"),
            layout: &self.object_layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: buffer.as_entire_binding() }],
        });
        ObjectGpu { buffer, bind_group }
    }

    /// Packs enabled lights for the GPU and returns the shadow-casting light's matrix, if any.
    fn write_world(&self, scene: &Scene) -> Option<Mat4> {
        let mut world = WorldUniform::zeroed();
        let mut count = 0;
        let mut shadow: Option<(usize, Mat4)> = None;
        for light in scene.lights.iter().filter(|l| l.enabled).take(MAX_LIGHTS) {
            let color = (light.color * light.intensity).extend(0.0).into();
            world.lights[count] = match light.kind {
                LightKind::Directional { direction } => {
                    if self.shadows && light.cast_shadows && shadow.is_none() {
                        shadow = Some((count, shadow_matrix(direction, scene.shadow_center, scene.shadow_radius)));
                    }
                    LightUniform {
                        position: [0.0; 4],
                        direction: direction.normalize().extend(0.0).into(),
                        color,
                        params: [0.0; 4],
                    }
                }
                LightKind::Point { position, range } => LightUniform {
                    position: position.extend(1.0).into(),
                    direction: [0.0; 4],
                    color,
                    params: [range, 0.0, 0.0, 0.0],
                },
                LightKind::Spot { position, direction, range, inner_angle, outer_angle } => LightUniform {
                    position: position.extend(2.0).into(),
                    direction: direction.normalize().extend(0.0).into(),
                    color,
                    params: [range, inner_angle.cos(), outer_angle.cos(), 0.0],
                },
            };
            count += 1;
        }
        let texel_world = 2.0 * scene.shadow_radius / SHADOW_MAP_SIZE as f32;
        world.shadow_view_proj = shadow.map_or(Mat4::IDENTITY, |s| s.1).to_cols_array_2d();
        world.params = [
            count as f32,
            self.environment_intensity,
            shadow.map_or(-1.0, |s| s.0 as f32),
            self.env_max_lod,
        ];
        world.shadow_params = [1.0 / SHADOW_MAP_SIZE as f32, texel_world * 1.5, 0.0, 0.0];
        self.queue.write_buffer(&self.world_buffer, 0, bytemuck::bytes_of(&world));
        shadow.map(|s| s.1)
    }

    /// Draws a frame to the window. Does nothing for a headless renderer; use `screenshot`.
    pub fn render(&mut self, scene: &Scene, camera: &OrbitCamera) {
        let Some(surface) = &self.surface else { return };
        let frame = match surface.get_current_texture() {
            Ok(f) => f,
            Err(wgpu::SurfaceError::Lost | wgpu::SurfaceError::Outdated) => {
                surface.configure(&self.device, &self.config);
                return;
            }
            Err(e) => {
                log::warn!("surface error: {e:?}");
                return;
            }
        };
        let surface_view = frame.texture.create_view(&Default::default());
        let encoder = self.encode_frame(scene, camera, &surface_view);
        self.queue.submit([encoder.finish()]);
        frame.present();
    }

    /// Renders one frame offscreen at window size and returns it as an sRGB image.
    pub fn screenshot(&mut self, scene: &Scene, camera: &OrbitCamera) -> Image {
        let (width, height) = (self.config.width, self.config.height);
        let size = wgpu::Extent3d { width, height, depth_or_array_layers: 1 };
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("screenshot"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: self.config.format, // same format the tonemap pipeline was built for
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let mut encoder = self.encode_frame(scene, camera, &texture.create_view(&Default::default()));

        let padded_row = (width * 4).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("screenshot readback"),
            size: (padded_row * height) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        encoder.copy_texture_to_buffer(
            texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded_row),
                    rows_per_image: Some(height),
                },
            },
            size,
        );
        self.queue.submit([encoder.finish()]);

        let slice = buffer.slice(..);
        slice.map_async(wgpu::MapMode::Read, |r| r.expect("map screenshot buffer"));
        self.device.poll(wgpu::PollType::Wait).expect("wait for GPU");
        let data = slice.get_mapped_range();
        let bgra = matches!(self.config.format, wgpu::TextureFormat::Bgra8UnormSrgb | wgpu::TextureFormat::Bgra8Unorm);
        let mut pixels = Vec::with_capacity((width * height * 4) as usize);
        for row in data.chunks(padded_row as usize) {
            for px in row[..(width * 4) as usize].chunks_exact(4) {
                pixels.extend_from_slice(&if bgra { [px[2], px[1], px[0], 255] } else { [px[0], px[1], px[2], 255] });
            }
        }
        Image::new(width, height, pixels)
    }

    /// Records all passes for one frame, ending with the tonemap pass into `target`.
    fn encode_frame(&mut self, scene: &Scene, camera: &OrbitCamera, target: &wgpu::TextureView) -> wgpu::CommandEncoder {
        let viewport = [self.config.width as f32, self.config.height as f32, 0.0, 0.0];

        // ---- per-frame uniforms
        let view_proj = camera.view_proj();
        let eye = camera.eye();
        let mirror = if self.reflections { scene.objects.iter().position(|o| o.material.mirror > 0.0) } else { None };
        let mirror_flag = if mirror.is_some() { 1.0 } else { 0.0 };
        let main = ViewUniform::new(view_proj, eye, [0.0, 0.0, mirror_flag, 0.0], viewport);
        self.queue.write_buffer(&self.main_view.buffer, 0, bytemuck::bytes_of(&main));

        if let Some(index) = mirror {
            let height = scene.objects[index].transform.position.y;
            let m = mirror_matrix(height);
            let u = ViewUniform::new(view_proj * m, m.transform_point3(eye), [1.0, height, 0.0, 0.0], viewport);
            self.queue.write_buffer(&self.mirror_view.buffer, 0, bytemuck::bytes_of(&u));
        }

        let shadow = self.write_world(scene);
        if let Some(light_view_proj) = shadow {
            let u = ViewUniform::new(light_view_proj, Vec3::ZERO, [0.0; 4], viewport);
            self.queue.write_buffer(&self.shadow_view.buffer, 0, bytemuck::bytes_of(&u));
        }

        for obj in &scene.objects {
            let m = &obj.material;
            let u = ObjectUniform {
                model: obj.transform.matrix().to_cols_array_2d(),
                base_color: m.base_color.extend(1.0).into(),
                uv_transform: [m.uv_scale.x, m.uv_scale.y, m.uv_offset.x, m.uv_offset.y],
                material: [m.metallic, m.roughness, m.mirror, 0.0],
                emissive: m.emissive.extend(0.0).into(),
            };
            self.queue.write_buffer(&obj.gpu.buffer, 0, bytemuck::bytes_of(&u));
        }
        let tm = TonemapUniform {
            params: [self.exposure, match self.tonemap { Tonemap::Aces => 0.0, Tonemap::Reinhard => 1.0, Tonemap::Clamp => 2.0 }, 0.0, 0.0],
        };
        self.queue.write_buffer(&self.tonemap_buffer, 0, bytemuck::bytes_of(&tm));

        let mut encoder = self.device.create_command_encoder(&Default::default());

        // ---- 1. shadow map
        if shadow.is_some() {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("shadow pass"),
                color_attachments: &[],
                depth_stencil_attachment: Some(depth_attachment(&self.shadow_map, wgpu::StoreOp::Store)),
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_pipeline(&self.shadow_pipeline);
            pass.set_bind_group(0, &self.shadow_view.bind_group, &[]);
            for obj in &scene.objects {
                pass.set_bind_group(1, &obj.gpu.bind_group, &[]);
                draw_mesh(&mut pass, &scene.meshes[obj.mesh]);
            }
        }

        // ---- 2. planar reflection (everything except the mirror itself, seen upside down)
        if let Some(mirror_index) = mirror {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("reflection pass"),
                color_attachments: &[Some(color_attachment(&self.targets.reflection, None, wgpu::StoreOp::Store))],
                depth_stencil_attachment: Some(depth_attachment(&self.targets.reflection_depth, wgpu::StoreOp::Discard)),
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            self.draw_scene(&mut pass, scene, &self.mirror_pipeline, &self.sky_mirror_pipeline, &self.mirror_view, Some(mirror_index));
        }

        // ---- 3. main HDR pass, MSAA resolved into the HDR texture
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("main pass"),
                color_attachments: &[Some(color_attachment(
                    &self.targets.msaa_hdr,
                    Some(&self.targets.hdr),
                    wgpu::StoreOp::Discard,
                ))],
                depth_stencil_attachment: Some(depth_attachment(&self.targets.depth, wgpu::StoreOp::Discard)),
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            self.draw_scene(&mut pass, scene, &self.lit_pipeline, &self.sky_pipeline, &self.main_view, None);
        }

        // ---- 4. tonemap to the window
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("tonemap pass"),
                color_attachments: &[Some(color_attachment(target, None, wgpu::StoreOp::Store))],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_pipeline(&self.tonemap_pipeline);
            pass.set_bind_group(0, &self.tonemap_bind_group, &[]);
            pass.draw(0..3, 0..1);
        }
        encoder
    }

    fn draw_scene(
        &self,
        pass: &mut wgpu::RenderPass,
        scene: &Scene,
        lit: &wgpu::RenderPipeline,
        sky: &wgpu::RenderPipeline,
        view: &ViewGpu,
        skip: Option<usize>,
    ) {
        pass.set_pipeline(lit);
        pass.set_bind_group(0, &view.bind_group, &[]);
        pass.set_bind_group(3, &self.world_bind_group, &[]);
        for (i, obj) in scene.objects.iter().enumerate() {
            if Some(i) == skip {
                continue;
            }
            pass.set_bind_group(1, &obj.gpu.bind_group, &[]);
            pass.set_bind_group(2, &scene.textures[obj.material.texture].bind_group, &[]);
            draw_mesh(pass, &scene.meshes[obj.mesh]);
        }
        pass.set_pipeline(sky);
        pass.set_bind_group(0, &view.bind_group, &[]);
        pass.set_bind_group(1, &self.world_bind_group, &[]);
        pass.draw(0..3, 0..1);
    }
}

fn draw_mesh(pass: &mut wgpu::RenderPass, mesh: &GpuMesh) {
    pass.set_vertex_buffer(0, mesh.vertex_buffer.slice(..));
    pass.set_index_buffer(mesh.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
    pass.draw_indexed(0..mesh.index_count, 0, 0..1);
}

fn color_attachment<'a>(
    view: &'a wgpu::TextureView,
    resolve_target: Option<&'a wgpu::TextureView>,
    store: wgpu::StoreOp,
) -> wgpu::RenderPassColorAttachment<'a> {
    wgpu::RenderPassColorAttachment {
        view,
        resolve_target,
        depth_slice: None,
        ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store },
    }
}

fn depth_attachment(view: &wgpu::TextureView, store: wgpu::StoreOp) -> wgpu::RenderPassDepthStencilAttachment<'_> {
    wgpu::RenderPassDepthStencilAttachment {
        view,
        depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store }),
        stencil_ops: None,
    }
}

fn world_bind_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    buffer: &wgpu::Buffer,
    shadow_map: &wgpu::TextureView,
    shadow_sampler: &wgpu::Sampler,
    env: &wgpu::TextureView,
    env_sampler: &wgpu::Sampler,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("world bg"),
        layout,
        entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: buffer.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(shadow_map) },
            wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(shadow_sampler) },
            wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(env) },
            wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::Sampler(env_sampler) },
        ],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mirror_matrix_reflects_across_plane() {
        let m = mirror_matrix(-0.8);
        let p = m.transform_point3(Vec3::new(1.0, 2.0, 3.0));
        assert!((p - Vec3::new(1.0, -3.6, 3.0)).length() < 1e-5);
        // Applying it twice is the identity.
        assert!((m * m).abs_diff_eq(Mat4::IDENTITY, 1e-5));
    }

    #[test]
    fn shadow_matrix_keeps_bounds_inside_clip_space() {
        let m = shadow_matrix(Vec3::new(-0.5, -1.0, -0.4), Vec3::ZERO, 5.0);
        for p in [Vec3::ZERO, Vec3::new(3.0, 0.0, 3.0), Vec3::new(-3.0, 2.0, -2.0)] {
            let c = m.project_point3(p);
            assert!(c.x.abs() <= 1.0 && c.y.abs() <= 1.0, "{p} -> {c}");
            assert!((0.0..=1.0).contains(&c.z), "{p} -> {c}");
        }
    }

    #[test]
    fn uniform_sizes_are_16_byte_multiples() {
        for size in [
            std::mem::size_of::<ViewUniform>(),
            std::mem::size_of::<ObjectUniform>(),
            std::mem::size_of::<WorldUniform>(),
            std::mem::size_of::<TonemapUniform>(),
        ] {
            assert_eq!(size % 16, 0);
        }
    }
}
