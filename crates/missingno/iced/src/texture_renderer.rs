use iced::{Rectangle, wgpu, widget::shader};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

static NEXT_TEXTURE_ID: AtomicU64 = AtomicU64::new(0);

/// Screen pixels per source pixel below which an overlay's darkening lines would
/// merge into flat dimming rather than separate pixels, so they fade out; full
/// strength by [`OVERLAY_FULL_PX`]. Shared with the fragment shader, which reads
/// the same ramp.
pub(crate) const OVERLAY_ONSET_PX: f32 = 3.0;
pub(crate) const OVERLAY_FULL_PX: f32 = 6.0;
/// The LCD grid is a pixel *aperture*, not a darkening laid on top: a fragment
/// either lands inside a lit pixel or in the inter-pixel matrix, which shows the
/// panel behind it. `MATRIX_FRACTION` is the share of the cell pitch, per axis,
/// that the matrix takes; `APERTURE_FRACTION` is the lit remainder. Both paths
/// spend the same energy on the matrix, so the two agree in the mean. Shared
/// with the fragment shader.
const MATRIX_FRACTION: f32 = 1.0 / PRESCALE_MAX as f32;
const APERTURE_FRACTION: f32 = 1.0 - MATRIX_FRACTION;
/// The strongest blend toward the matrix colour a rendered line reaches. The
/// physical fraction conserves below it; past it a larger screen keeps delicate
/// hairlines instead of saturated lines — a perceptual cap, not panel physics.
const MATRIX_CONTRAST: f32 = 0.5;
/// The grid is composed at an integer prescale — one console pixel per N×N
/// texels, matrix baked into the last row and column — so its spacing is
/// perfectly periodic whatever the window size. The matrix width is this cap's
/// reciprocal, so at the cap the band exactly fills its one edge texel, and
/// past it the band spans at least one screen pixel — the analytic aperture
/// takes over at the same strength and the handoff is seamless. One integer
/// tunes the grid: raising it thins the matrix.
const PRESCALE_MAX: u32 = 20;

/// CRT scanlines are not a darkening laid over the picture — the beam *emits*
/// each source row as a bright line with a soft vertical falloff, and the gaps
/// are simply where no beam lands. The output row is the sum of Gaussian beam
/// contributions from the two source rows bracketing it, each beam's width
/// growing with that row's luminance: bright lines bloom to nearly fill the
/// pitch, dark lines stay thin and leave a wide gap. Widths are in units of the
/// source-row pitch, so the look holds at any output resolution. Shared with the
/// fragment shader.
const BEAM_SIGMA_MIN: f32 = 0.18;
const BEAM_SIGMA_MAX: f32 = 0.52;
/// Emission normalization: a naive beam profile spreads a row's energy over less
/// than a full pitch and dims the image. `BEAM_NORM` is `1 / mean_beam_field`
/// evaluated at the mid-luminance width (`BEAM_SIGMA_MIN..MAX` midpoint), so a
/// flat mid-bright field keeps its average brightness across a pitch. Brighter
/// content blooms past unity (an intentional, authentic-reading contrast lift);
/// darker content stays dark. Recomputed and pinned by the CPU tests.
const BEAM_NORM: f32 = 1.1447;
/// Rec.601 luma weights driving a beam's width from its row's brightness.
const BEAM_LUMA: [f32; 3] = [0.299, 0.587, 0.114];

/// A cosmetic device-simulation overlay drawn over the sampled picture, keyed to
/// the display technology and toggleable in settings.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum ScreenOverlay {
    #[default]
    None,
    /// An LCD's inter-pixel grid: a subtle line at every source-pixel boundary.
    PixelGrid,
    /// A CRT's scanlines: subtle darkening at the native line pitch.
    Scanlines,
}

impl ScreenOverlay {
    /// The mode value the fragment shader switches on.
    fn shader_mode(self) -> f32 {
        match self {
            ScreenOverlay::None => 0.0,
            ScreenOverlay::PixelGrid => 1.0,
            ScreenOverlay::Scanlines => 2.0,
        }
    }
}

/// The integer prescale the LCD matrix composes at for a widget showing `scale`
/// screen pixels per console pixel, or `None` past [`PRESCALE_MAX`] where the
/// analytic aperture shader takes over.
fn prescale_factor(scale: f32) -> Option<u32> {
    let factor = scale.max(1.0).ceil() as u32;
    (factor <= PRESCALE_MAX).then_some(factor)
}

/// Reusable GPU texture renderer for pixel-based graphics
pub struct TextureRenderer {
    /// Filled on the first draw unless [`TextureRenderer::key`] named a slot, so
    /// a keyed owner never spends an id it discards.
    id: OnceLock<u64>,
    width: u32,
    height: u32,
    pixels: Arc<[u8]>,
    overlay: ScreenOverlay,
    panel_base: [f32; 3],
}

impl TextureRenderer {
    pub fn with_pixels(width: u32, height: u32, pixels: impl Into<Arc<[u8]>>) -> Self {
        let pixels = pixels.into();
        assert_eq!(pixels.len(), (width * height * 4) as usize);
        Self {
            id: OnceLock::new(),
            width,
            height,
            pixels,
            overlay: ScreenOverlay::None,
            panel_base: [1.0, 1.0, 1.0],
        }
    }

    /// Draw the given cosmetic overlay over the picture.
    pub fn overlay(mut self, overlay: ScreenOverlay) -> Self {
        self.overlay = overlay;
        self
    }

    /// The reflective panel base the LCD aperture grid shows between pixels, as
    /// linear RGB in 0..1. Ignored unless the overlay is the pixel grid.
    pub fn panel_base(mut self, base: [f32; 3]) -> Self {
        self.panel_base = base;
        self
    }

    /// Render into a caller-owned texture slot instead of a fresh one, so a
    /// long-lived owner re-uses one GPU texture across per-draw constructions.
    pub fn key(mut self, key: u64) -> Self {
        self.id = OnceLock::from(key);
        self
    }

    /// A key for [`TextureRenderer::key`], unique for the process's lifetime.
    pub fn allocate_key() -> u64 {
        NEXT_TEXTURE_ID.fetch_add(1, Ordering::Relaxed)
    }
}

impl<Message> shader::Program<Message> for TextureRenderer {
    type State = ();
    type Primitive = TexturePrimitive;

    fn draw(
        &self,
        _state: &Self::State,
        _cursor: iced::mouse::Cursor,
        _bounds: Rectangle,
    ) -> Self::Primitive {
        TexturePrimitive {
            id: *self.id.get_or_init(Self::allocate_key),
            overlay: self.overlay,
            panel_base: self.panel_base,
            state: Mutex::new(PrimitiveState::Pending {
                width: self.width,
                height: self.height,
                pixels: self.pixels.clone(),
            }),
            registry: Mutex::new(None),
        }
    }
}

pub struct TexturePrimitive {
    id: u64,
    overlay: ScreenOverlay,
    panel_base: [f32; 3],
    state: Mutex<PrimitiveState>,
    /// Set on first prepare; releases this primitive's claim on its texture
    /// entry when iced drops the primitive.
    registry: Mutex<Option<TextureRegistry>>,
}

impl std::fmt::Debug for TexturePrimitive {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TexturePrimitive")
            .field("id", &self.id)
            .field("overlay", &self.overlay)
            .field("state", &self.state)
            .finish_non_exhaustive()
    }
}

impl Drop for TexturePrimitive {
    fn drop(&mut self) {
        if let Some(map) = self.registry.lock().unwrap().take() {
            let mut textures = map.lock().unwrap();
            if let Some(data) = textures.get_mut(&self.id) {
                data.live = data.live.saturating_sub(1);
                if data.live == 0 {
                    textures.remove(&self.id);
                }
            }
        }
    }
}

#[derive(Debug)]
enum PrimitiveState {
    Pending {
        width: u32,
        height: u32,
        pixels: Arc<[u8]>,
    },
    Prepared {
        width: u32,
        height: u32,
        bounds: Rectangle,
    },
}

impl TexturePrimitive {
    /// The prescale this draw composes the matrix at: the widget's zoom rounded
    /// up, and only for the LCD grid — every other overlay renders single-pass.
    fn prescale(
        &self,
        width: u32,
        height: u32,
        bounds: &Rectangle,
        viewport: &shader::Viewport,
    ) -> Option<u32> {
        if self.overlay != ScreenOverlay::PixelGrid {
            return None;
        }
        let physical = viewport.scale_factor();
        let per_source = (bounds.width * physical / width.max(1) as f32)
            .max(bounds.height * physical / height.max(1) as f32);
        prescale_factor(per_source)
    }

    /// The overlay the final pass draws: none once the matrix is baked into the
    /// intermediate, since the grid is already in the picture it samples.
    fn final_overlay(&self, prescale: Option<u32>) -> f32 {
        match prescale {
            Some(_) => ScreenOverlay::None.shader_mode(),
            None => self.overlay.shader_mode(),
        }
    }

    /// Claim the texture entry on first prepare; `Drop` releases the claim.
    fn register(&self, pipeline: &TexturePipeline) {
        let mut registry = self.registry.lock().unwrap();
        if registry.is_none()
            && let Some(data) = pipeline.textures.lock().unwrap().get_mut(&self.id)
        {
            data.live += 1;
            *registry = Some(pipeline.textures.clone());
        }
    }
}

impl shader::Primitive for TexturePrimitive {
    type Pipeline = TexturePipeline;

    fn prepare(
        &self,
        pipeline: &mut Self::Pipeline,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        bounds: &Rectangle,
        viewport: &shader::Viewport,
    ) {
        let mut state = self.state.lock().unwrap();

        match std::mem::replace(
            &mut *state,
            PrimitiveState::Prepared {
                width: 0,
                height: 0,
                bounds: *bounds,
            },
        ) {
            PrimitiveState::Pending {
                width,
                height,
                pixels,
            } => {
                pipeline.ensure_texture(device, self.id, width, height);
                self.register(pipeline);

                let textures = pipeline.textures.lock().unwrap();
                let texture_data = textures.get(&self.id).unwrap();

                queue.write_texture(
                    wgpu::TexelCopyTextureInfo {
                        texture: &texture_data.texture,
                        mip_level: 0,
                        origin: wgpu::Origin3d::ZERO,
                        aspect: wgpu::TextureAspect::All,
                    },
                    &pixels,
                    wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(width * 4),
                        rows_per_image: Some(height),
                    },
                    wgpu::Extent3d {
                        width,
                        height,
                        depth_or_array_layers: 1,
                    },
                );

                drop(textures);
                let prescale = self.prescale(width, height, bounds, viewport);
                pipeline.ensure_intermediate(device, self.id, prescale);
                // Use prepare()'s bounds (screen-space) not draw()'s bounds
                // (content-space), so the texture renders at the correct
                // position when inside a scrollable.
                pipeline.update_vertices(
                    queue,
                    self.id,
                    *bounds,
                    viewport,
                    self.final_overlay(prescale),
                    self.panel_base,
                );
                pipeline.update_compose_vertices(queue, self.id, self.panel_base);

                *state = PrimitiveState::Prepared {
                    width,
                    height,
                    bounds: *bounds,
                };
            }
            PrimitiveState::Prepared {
                width,
                height,
                bounds: old_bounds,
            } => {
                pipeline.ensure_texture(device, self.id, width, height);
                self.register(pipeline);

                let prescale = self.prescale(width, height, bounds, viewport);
                pipeline.ensure_intermediate(device, self.id, prescale);
                if &old_bounds != bounds {
                    pipeline.update_vertices(
                        queue,
                        self.id,
                        *bounds,
                        viewport,
                        self.final_overlay(prescale),
                        self.panel_base,
                    );
                }
                pipeline.update_compose_vertices(queue, self.id, self.panel_base);
                *state = PrimitiveState::Prepared {
                    width,
                    height,
                    bounds: *bounds,
                };
            }
        }
    }

    fn render(
        &self,
        pipeline: &Self::Pipeline,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        viewport: &Rectangle<u32>,
    ) {
        let textures = pipeline.textures.lock().unwrap();
        let Some(texture_data) = textures.get(&self.id) else {
            return;
        };

        // Compose the console frame and its inter-pixel matrix at the integer
        // prescale first; the final pass then resamples that to the widget.
        if let Some(intermediate) = &texture_data.intermediate {
            let mut compose_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("lcd_matrix_compose_pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &intermediate.view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            compose_pass.set_pipeline(&pipeline.compose_pipeline);
            compose_pass.set_bind_group(0, &texture_data.bind_group, &[]);
            compose_pass.set_vertex_buffer(0, intermediate.vertex_buffer.slice(..));
            compose_pass.draw(0..6, 0..1);
        }

        let sampled = match &texture_data.intermediate {
            Some(intermediate) => &intermediate.bind_group,
            None => &texture_data.bind_group,
        };

        let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("texture_renderer_pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
                depth_slice: None,
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });

        render_pass.set_scissor_rect(viewport.x, viewport.y, viewport.width, viewport.height);
        render_pass.set_pipeline(&pipeline.render_pipeline);
        render_pass.set_bind_group(0, sampled, &[]);
        render_pass.set_vertex_buffer(0, texture_data.vertex_buffer.slice(..));
        render_pass.draw(0..6, 0..1);
    }
}

type TextureRegistry = Arc<Mutex<HashMap<u64, TextureData>>>;

struct TextureData {
    texture: wgpu::Texture,
    bind_group: wgpu::BindGroup,
    vertex_buffer: wgpu::Buffer,
    width: u32,
    height: u32,
    /// Primitives currently registered against this entry; freed at zero.
    live: usize,
    /// The prescaled compose target, present only while the LCD grid path runs.
    intermediate: Option<Intermediate>,
}

/// The console frame at `prescale`× with the inter-pixel matrix baked in — what
/// the final pass resamples to the widget.
struct Intermediate {
    view: wgpu::TextureView,
    bind_group: wgpu::BindGroup,
    vertex_buffer: wgpu::Buffer,
    prescale: u32,
}

pub struct TexturePipeline {
    render_pipeline: wgpu::RenderPipeline,
    compose_pipeline: wgpu::RenderPipeline,
    bind_group_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    textures: Arc<Mutex<HashMap<u64, TextureData>>>,
}

/// The format the matrix is composed into: the same unorm space the console
/// frame arrives in, so the extra pass costs no colour conversion.
const INTERMEDIATE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

impl shader::Pipeline for TexturePipeline {
    fn new(device: &wgpu::Device, _queue: &wgpu::Queue, format: wgpu::TextureFormat) -> Self {
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("texture_renderer_sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("texture_renderer_bind_group_layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });

        let shader_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("texture_renderer_shader"),
            source: wgpu::ShaderSource::Wgsl(std::borrow::Cow::Owned(shader_source())),
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("texture_renderer_pipeline_layout"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });

        let render_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("texture_renderer_pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader_module,
                entry_point: Some("vs_main"),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<Vertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![
                        0 => Float32x2,
                        1 => Float32x2,
                        2 => Float32,
                        3 => Float32x3,
                    ],
                }],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader_module,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });

        let compose_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("lcd_matrix_compose_pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader_module,
                entry_point: Some("vs_compose"),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<ComposeVertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![
                        0 => Float32x2,
                        1 => Float32x3,
                        2 => Float32,
                    ],
                }],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader_module,
                entry_point: Some("fs_compose"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: INTERMEDIATE_FORMAT,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });

        Self {
            render_pipeline,
            compose_pipeline,
            bind_group_layout,
            sampler,
            textures: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

impl TexturePipeline {
    fn ensure_texture(&self, device: &wgpu::Device, id: u64, width: u32, height: u32) {
        let mut textures = self.textures.lock().unwrap();

        let needs_creation = textures
            .get(&id)
            .is_none_or(|data| data.width != width || data.height != height);

        if needs_creation {
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("texture_renderer_texture"),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });

            let texture_view = texture.create_view(&wgpu::TextureViewDescriptor::default());

            let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("texture_renderer_bind_group"),
                layout: &self.bind_group_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&texture_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&self.sampler),
                    },
                ],
            });

            let vertex_buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("texture_renderer_vertex_buffer"),
                size: std::mem::size_of::<[Vertex; 6]>() as u64,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });

            // A resize replaces the entry; the primitives already registered
            // against it keep their claim on the id.
            let live = textures.get(&id).map(|data| data.live).unwrap_or(0);
            textures.insert(
                id,
                TextureData {
                    texture,
                    bind_group,
                    vertex_buffer,
                    width,
                    height,
                    live,
                    intermediate: None,
                },
            );
        }
    }

    /// Hold an intermediate at the requested prescale, reusing the existing one
    /// where the factor is unchanged; `None` drops it and returns the entry to
    /// the single-pass path.
    fn ensure_intermediate(&self, device: &wgpu::Device, id: u64, prescale: Option<u32>) {
        let mut textures = self.textures.lock().unwrap();
        let Some(data) = textures.get_mut(&id) else {
            return;
        };
        let Some(prescale) = prescale else {
            data.intermediate = None;
            return;
        };
        if data
            .intermediate
            .as_ref()
            .is_some_and(|held| held.prescale == prescale)
        {
            return;
        }

        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("lcd_matrix_intermediate"),
            size: wgpu::Extent3d {
                width: data.width * prescale,
                height: data.height * prescale,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: INTERMEDIATE_FORMAT,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("lcd_matrix_intermediate_bind_group"),
            layout: &self.bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        });
        let vertex_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("lcd_matrix_compose_vertex_buffer"),
            size: std::mem::size_of::<[ComposeVertex; 6]>() as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        data.intermediate = Some(Intermediate {
            view,
            bind_group,
            vertex_buffer,
            prescale,
        });
    }

    /// The compose pass's full-target quad, carrying the matrix colour and the
    /// prescale the fragment rule needs.
    fn update_compose_vertices(&self, queue: &wgpu::Queue, id: u64, matrix_color: [f32; 3]) {
        let textures = self.textures.lock().unwrap();
        let Some(intermediate) = textures
            .get(&id)
            .and_then(|data| data.intermediate.as_ref())
        else {
            return;
        };

        let prescale = intermediate.prescale as f32;
        let corner = |position: [f32; 2]| ComposeVertex {
            position,
            matrix_color,
            prescale,
        };
        let vertices = [
            corner([-1.0, 1.0]),
            corner([1.0, 1.0]),
            corner([-1.0, -1.0]),
            corner([-1.0, -1.0]),
            corner([1.0, 1.0]),
            corner([1.0, -1.0]),
        ];
        queue.write_buffer(
            &intermediate.vertex_buffer,
            0,
            bytemuck::cast_slice(&vertices),
        );
    }

    fn update_vertices(
        &self,
        queue: &wgpu::Queue,
        id: u64,
        bounds: Rectangle,
        viewport: &shader::Viewport,
        overlay: f32,
        panel_base: [f32; 3],
    ) {
        // Transform bounds to NDC space based on viewport
        let scale = viewport.scale_factor();
        let viewport_width = viewport.physical_width() as f32 / scale;
        let viewport_height = viewport.physical_height() as f32 / scale;

        // Convert widget bounds to NDC coordinates
        // X: 0..width maps to -1..1
        // Y: 0..height maps to 1..-1 (inverted in NDC)
        let left = (bounds.x / viewport_width) * 2.0 - 1.0;
        let right = ((bounds.x + bounds.width) / viewport_width) * 2.0 - 1.0;
        let top = -((bounds.y / viewport_height) * 2.0 - 1.0);
        let bottom = -(((bounds.y + bounds.height) / viewport_height) * 2.0 - 1.0);

        let vertices = [
            Vertex {
                position: [left, top],
                tex_coords: [0.0, 0.0],
                overlay,
                panel_base,
            },
            Vertex {
                position: [right, top],
                tex_coords: [1.0, 0.0],
                overlay,
                panel_base,
            },
            Vertex {
                position: [left, bottom],
                tex_coords: [0.0, 1.0],
                overlay,
                panel_base,
            },
            Vertex {
                position: [left, bottom],
                tex_coords: [0.0, 1.0],
                overlay,
                panel_base,
            },
            Vertex {
                position: [right, top],
                tex_coords: [1.0, 0.0],
                overlay,
                panel_base,
            },
            Vertex {
                position: [right, bottom],
                tex_coords: [1.0, 1.0],
                overlay,
                panel_base,
            },
        ];

        let textures = self.textures.lock().unwrap();
        if let Some(texture_data) = textures.get(&id) {
            queue.write_buffer(
                &texture_data.vertex_buffer,
                0,
                bytemuck::cast_slice(&vertices),
            );
        }
    }
}

#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct Vertex {
    position: [f32; 2],
    tex_coords: [f32; 2],
    overlay: f32,
    panel_base: [f32; 3],
}

#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct ComposeVertex {
    position: [f32; 2],
    matrix_color: [f32; 3],
    prescale: f32,
}

/// A WGSL float literal for `value` — always carrying a decimal point so it
/// types as `f32` in the shader.
fn wgsl_f32(value: f32) -> String {
    let text = value.to_string();
    if text.contains('.') {
        text
    } else {
        format!("{text}.0")
    }
}

/// The fragment shader, with the overlay thresholds and strengths shared from
/// the Rust constants so the CPU-side ramp and the GPU-side ramp never drift.
fn shader_source() -> String {
    SHADER_TEMPLATE
        .replace("__OVERLAY_ONSET_PX__", &wgsl_f32(OVERLAY_ONSET_PX))
        .replace("__OVERLAY_FULL_PX__", &wgsl_f32(OVERLAY_FULL_PX))
        .replace("__APERTURE_FRACTION__", &wgsl_f32(APERTURE_FRACTION))
        .replace("__MATRIX_FRACTION__", &wgsl_f32(MATRIX_FRACTION))
        .replace("__MATRIX_CONTRAST__", &wgsl_f32(MATRIX_CONTRAST))
        .replace("__BEAM_SIGMA_MIN__", &wgsl_f32(BEAM_SIGMA_MIN))
        .replace("__BEAM_SIGMA_MAX__", &wgsl_f32(BEAM_SIGMA_MAX))
        .replace("__BEAM_NORM__", &wgsl_f32(BEAM_NORM))
        .replace("__BEAM_LUMA_R__", &wgsl_f32(BEAM_LUMA[0]))
        .replace("__BEAM_LUMA_G__", &wgsl_f32(BEAM_LUMA[1]))
        .replace("__BEAM_LUMA_B__", &wgsl_f32(BEAM_LUMA[2]))
}

const SHADER_TEMPLATE: &str = r#"
struct VertexInput {
    @location(0) position: vec2<f32>,
    @location(1) tex_coords: vec2<f32>,
    @location(2) overlay: f32,
    @location(3) panel_base: vec3<f32>,
}

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) tex_coords: vec2<f32>,
    @location(1) @interpolate(flat) overlay: f32,
    @location(2) @interpolate(flat) panel_base: vec3<f32>,
}

const OVERLAY_ONSET_PX: f32 = __OVERLAY_ONSET_PX__;
const OVERLAY_FULL_PX: f32 = __OVERLAY_FULL_PX__;
const APERTURE_FRACTION: f32 = __APERTURE_FRACTION__;
const MATRIX_FRACTION: f32 = __MATRIX_FRACTION__;
const MATRIX_CONTRAST: f32 = __MATRIX_CONTRAST__;
const BEAM_SIGMA_MIN: f32 = __BEAM_SIGMA_MIN__;
const BEAM_SIGMA_MAX: f32 = __BEAM_SIGMA_MAX__;
const BEAM_NORM: f32 = __BEAM_NORM__;
const BEAM_LUMA: vec3<f32> = vec3<f32>(__BEAM_LUMA_R__, __BEAM_LUMA_G__, __BEAM_LUMA_B__);

@vertex
fn vs_main(input: VertexInput) -> VertexOutput {
    var output: VertexOutput;
    output.position = vec4<f32>(input.position, 0.0, 1.0);
    output.tex_coords = input.tex_coords;
    output.overlay = input.overlay;
    output.panel_base = input.panel_base;
    return output;
}

@group(0) @binding(0)
var texture: texture_2d<f32>;

@group(0) @binding(1)
var texture_sampler: sampler;

struct ComposeInput {
    @location(0) position: vec2<f32>,
    @location(1) matrix_color: vec3<f32>,
    @location(2) prescale: f32,
}

struct ComposeOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) @interpolate(flat) matrix_color: vec3<f32>,
    @location(1) @interpolate(flat) prescale: f32,
}

@vertex
fn vs_compose(input: ComposeInput) -> ComposeOutput {
    var output: ComposeOutput;
    output.position = vec4<f32>(input.position, 0.0, 1.0);
    output.matrix_color = input.matrix_color;
    output.prescale = input.prescale;
    return output;
}

// One console pixel becomes an N×N block of texels whose last row and column
// carry the inter-pixel matrix. Their coverage is MATRIX_FRACTION * N up to the
// MATRIX_CONTRAST cap — below it the block's mean is the source pixel mixed
// with MATRIX_FRACTION of matrix per axis, the same energy the analytic
// aperture spends; past it the line holds at the cap instead of saturating.
// The boundary falls exactly between texels, so the edge is crisp with no
// snapping error, and the grid is perfectly periodic whatever the final
// resample does.
@fragment
fn fs_compose(input: ComposeOutput) -> @location(0) vec4<f32> {
    let prescale = input.prescale;
    let texel = floor(input.position.xy);
    let cell = floor(texel / prescale);
    let source = textureLoad(texture, vec2<i32>(cell), 0);

    let within = texel - cell * prescale;
    let coverage = min(MATRIX_CONTRAST, MATRIX_FRACTION * prescale);
    let edge = step(vec2(prescale - 1.0), within) * coverage;
    let matrix = edge.x + edge.y - edge.x * edge.y;
    return vec4<f32>(mix(source.rgb, input.matrix_color, matrix), source.a);
}

fn beam_luma(rgb: vec3<f32>) -> f32 {
    return dot(rgb, BEAM_LUMA);
}

// A source row's beam half-width, growing with its luminance so bright lines
// bloom and dark lines stay thin.
fn beam_sigma(luma: f32) -> f32 {
    return BEAM_SIGMA_MIN + (BEAM_SIGMA_MAX - BEAM_SIGMA_MIN) * clamp(luma, 0.0, 1.0);
}

// Gaussian vertical beam profile at distance `d` (in row-pitch units) for a
// beam of half-width `sigma`.
fn beam_weight(d: f32, sigma: f32) -> f32 {
    let z = d / sigma;
    return exp(-0.5 * z * z);
}

// Sharp bilinear filtering: each source texel maps to a uniform-sized
// block of screen pixels. Bilinear blending only occurs in a 1-pixel
// band at the boundary between texels, keeping interiors crisp.
@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    let tex_size = vec2<f32>(textureDimensions(texture));
    let texel = input.tex_coords * tex_size - vec2(0.5);

    let texel_floor = floor(texel);
    let frac = texel - texel_floor;

    // Source texels spanned by one screen pixel in each axis.
    let scale = tex_size * fwidth(input.tex_coords);

    // Remap the fractional part: flat across the pixel interior, ramping
    // linearly across one screen pixel centred on the source-pixel boundary —
    // frac 0.5, halfway between texel centres. Centring the ramp there keeps
    // every pixel at its true footprint; a ramp at the end of the interval
    // shifts the picture half a source pixel and halves the edge rows/columns.
    let sharp = clamp((frac - vec2(0.5)) / scale + vec2(0.5), vec2(0.0), vec2(1.0));

    let snapped = (texel_floor + vec2(0.5) + sharp) / tex_size;
    let plain = textureSample(texture, texture_sampler, snapped);

    // Cosmetic device-simulation overlay: 1 = reflective-LCD pixel aperture,
    // 2 = CRT beam emission. Both fade out below OVERLAY_ONSET_PX screen pixels
    // per source pixel where the structure would merge into flat dimming.
    let mode = input.overlay;
    if (mode < 0.5) {
        return plain;
    }

    let screen_px_per_source = vec2(1.0) / max(scale, vec2(0.0001));
    let vis = smoothstep(
        vec2(OVERLAY_ONSET_PX), vec2(OVERLAY_FULL_PX), screen_px_per_source);

    if (mode < 1.5) {
        // LCD pixel aperture: a reflective panel has no backlight, so the cell
        // is a flat-coloured lit pixel and the inter-pixel matrix is a border
        // that exposes the pale panel base — the grid is where pixels aren't.
        // Point-sample THIS cell's colour at its centre (a physical pixel is one
        // flat colour — no cross-cell bilinear inside the aperture), and blend
        // to the panel base across the border. The fragment's cell is the source
        // pixel under it; boundaries sit at frac 0.5 (halfway between texel
        // centres), cell centres at frac 0 and 1.
        let cell = floor(input.tex_coords * tex_size);
        let cell_centre = (cell + vec2(0.5)) / tex_size;
        let cell_color = textureSample(texture, texture_sampler, cell_centre).rgb;
        let aperture = vec2(APERTURE_FRACTION * 0.5);
        // The matrix band has half-width 0.5 - aperture each side of the cell
        // boundary. Blend to the base only in proportion to the band's true
        // area — any wider ramp washes the whole picture toward the base.
        let g = vec2(0.5) - aperture;
        let boundary_dist = abs(frac - vec2(0.5));
        let px = max(scale, vec2(0.0001));
        // Hairline regime (band thinner than a screen pixel): snap each line to
        // the nearest pixel row/column at constant strength. Free-phase coverage
        // would concentrate a line in one fragment or split it across two as its
        // sub-pixel phase drifts, beating against the pixel grid as a large
        // moiré pattern at non-integer scales.
        let hairline = min(2.0 * g / px, vec2(1.0))
            * vec2(f32(boundary_dist.x < px.x * 0.5), f32(boundary_dist.y < px.y * 0.5));
        // Wide regime (zoomed in): exact box-filter coverage anti-aliases the
        // band edges.
        let half_px = px * 0.5;
        let overlap = min(boundary_dist + half_px, g) - max(boundary_dist - half_px, -g);
        let box_cover = clamp(overlap / px, vec2(0.0), vec2(1.0));
        let matrix_cover = min(select(box_cover, hairline, 2.0 * g < px), vec2(MATRIX_CONTRAST));
        let inside = vec2(1.0) - matrix_cover;
        let lit = mix(input.panel_base, cell_color, inside.x * inside.y);
        // Below the onset the pixels are too few screen pixels apart to resolve
        // the matrix, so fall back to the plain sharp picture.
        let grid_vis = min(vis.x, vis.y);
        return vec4<f32>(mix(plain.rgb, lit, grid_vis), plain.a);
    }

    // CRT beam emission: keep the sharp-bilinear x treatment, but along y sum
    // the beam contributions of the two source rows bracketing this output
    // pixel. Each row is sampled at its centre and weighted by a Gaussian
    // profile whose width grows with that row's luminance, so a bright row
    // blooms to fill the pitch while a dark row stays a thin line with a wide
    // dark gap around it. Two rows suffice — the profile is narrow enough that
    // the next row out never contributes meaningfully.
    let row_below_y = (texel_floor.y + 0.5) / tex_size.y;
    let row_above_y = (texel_floor.y + 1.5) / tex_size.y;
    let c_below = textureSample(texture, texture_sampler, vec2(snapped.x, row_below_y)).rgb;
    let c_above = textureSample(texture, texture_sampler, vec2(snapped.x, row_above_y)).rgb;

    let w_below = beam_weight(frac.y, beam_sigma(beam_luma(c_below)));
    let w_above = beam_weight(1.0 - frac.y, beam_sigma(beam_luma(c_above)));
    let emitted = (c_below * w_below + c_above * w_above) * BEAM_NORM;

    // Below the visibility onset the rows are too few screen pixels apart to
    // resolve the beam structure, so fall back to the plain sharp picture.
    return vec4<f32>(mix(plain.rgb, emitted, vis.y), plain.a);
}
"#;

#[cfg(test)]
mod tests {
    use super::*;

    /// The CPU mirror of the shader's visibility ramp, exercising the shared
    /// overlay thresholds. The scanline emission and the LCD grid both scale
    /// their strength by this ramp.
    fn overlay_visibility(screen_px_per_source: f32) -> f32 {
        let t = ((screen_px_per_source - OVERLAY_ONSET_PX) / (OVERLAY_FULL_PX - OVERLAY_ONSET_PX))
            .clamp(0.0, 1.0);
        t * t * (3.0 - 2.0 * t)
    }

    // ── Sharp-bilinear mirror ────────────────────────────────────────────

    /// CPU mirror of the shader's sharp-bilinear remap: the blend weight toward
    /// the next texel at shader `frac`, for a `scale`-texel screen pixel.
    fn sharp_remap(frac: f32, scale: f32) -> f32 {
        ((frac - 0.5) / scale + 0.5).clamp(0.0, 1.0)
    }

    #[test]
    fn sharp_transition_is_centred_on_the_pixel_boundary() {
        // The 50/50 blend lands exactly on the source-pixel boundary (frac 0.5,
        // halfway between texel centres), so every pixel keeps its true screen
        // footprint. An end-of-interval ramp shifts the picture half a source
        // pixel and renders the edge rows and columns at half size.
        let scale = 0.1;
        assert_eq!(sharp_remap(0.5, scale), 0.5);
        // Flat interiors outside a one-screen-pixel band around the boundary.
        assert_eq!(sharp_remap(0.5 - scale * 0.51, scale), 0.0);
        assert_eq!(sharp_remap(0.5 + scale * 0.51, scale), 1.0);
        assert_eq!(sharp_remap(0.0, scale), 0.0);
        assert_eq!(sharp_remap(1.0, scale), 1.0);
    }

    // ── LCD aperture mirrors ─────────────────────────────────────────────

    /// CPU mirror of the shader's hard-edged aperture membership at shader
    /// `frac` (per axis): 1 inside the lit pixel, 0 in the matrix border. Pixel
    /// boundaries sit at `frac` 0.5, so the matrix band spans `0.5 ± g` and the
    /// aperture is everything nearer a cell centre (`frac` 0 or 1).
    fn aperture_inside(frac: f32) -> f32 {
        let g = 0.5 - APERTURE_FRACTION * 0.5;
        if (frac - 0.5).abs() < g { 0.0 } else { 1.0 }
    }

    /// The one-axis lit fraction of a cell — the share of intra-cell positions
    /// falling inside the aperture — by fine sampling.
    fn lit_fraction() -> f32 {
        let n = 100_000;
        let lit = (0..n)
            .filter(|&i| aperture_inside((i as f32 + 0.5) / n as f32) > 0.5)
            .count();
        lit as f32 / n as f32
    }

    /// CPU mirror of the aperture colour blend: the cell's own colour inside the
    /// aperture, the panel base in the border (hard-edged, one axis).
    fn aperture_color(frac: f32, cell: f32, base: f32) -> f32 {
        let inside = aperture_inside(frac);
        base * (1.0 - inside) + cell * inside
    }

    /// CPU mirror of the shader's matrix coverage at shader `frac` for a
    /// `scale`-texel screen pixel (one axis): pixel-snapped constant strength
    /// while the band is a hairline, box-filter coverage once zoomed wide
    /// enough to resolve its edges, both held under the contrast cap.
    fn matrix_coverage(frac: f32, scale: f32) -> f32 {
        let aperture = APERTURE_FRACTION * 0.5;
        let g = 0.5 - aperture;
        let boundary_dist = (frac - 0.5).abs();
        let px = scale.max(0.0001);
        let uncapped = if 2.0 * g < px {
            if boundary_dist < px * 0.5 {
                (2.0 * g / px).min(1.0)
            } else {
                0.0
            }
        } else {
            let half_px = px * 0.5;
            let overlap = (boundary_dist + half_px).min(g) - (boundary_dist - half_px).max(-g);
            (overlap / px).clamp(0.0, 1.0)
        };
        uncapped.min(MATRIX_CONTRAST)
    }

    // ── Prescaled compose mirrors ────────────────────────────────────────

    /// CPU mirror of the compose rule's per-axis matrix coverage for texel
    /// `within` of a cell composed at `prescale`.
    fn compose_edge_coverage(within: u32, prescale: u32) -> f32 {
        if within + 1 == prescale {
            (MATRIX_FRACTION * prescale as f32).min(MATRIX_CONTRAST)
        } else {
            0.0
        }
    }

    /// The mean matrix share across one axis of a composed cell.
    fn compose_axis_mean(prescale: u32) -> f32 {
        (0..prescale)
            .map(|within| compose_edge_coverage(within, prescale))
            .sum::<f32>()
            / prescale as f32
    }

    /// The mean matrix share across a whole composed cell, with the corner
    /// texels' two bands unioned rather than double-counted.
    fn compose_block_mean(prescale: u32) -> f32 {
        let mut total = 0.0;
        for i in 0..prescale {
            for j in 0..prescale {
                let (x, y) = (
                    compose_edge_coverage(i, prescale),
                    compose_edge_coverage(j, prescale),
                );
                total += x + y - x * y;
            }
        }
        total / (prescale * prescale) as f32
    }

    /// Screen-space centre of cell `cell`'s matrix line under the two-stage
    /// path: the last texel of the cell in the intermediate, resampled to a
    /// widget showing `scale` screen pixels per console pixel.
    fn composed_line_centre(cell: u32, scale: f32, prescale: u32) -> f32 {
        let texel_centre = (cell * prescale + prescale - 1) as f32 + 0.5;
        texel_centre * scale / prescale as f32
    }

    /// The single-pass hairline placement: the cell boundary snaps to whichever
    /// screen pixel contains it.
    fn snapped_line_centre(cell: u32, scale: f32) -> f32 {
        (cell as f32 * scale).floor() + 0.5
    }

    /// The prescale where a line's blend reaches the contrast cap; below it the
    /// physical fraction conserves exactly.
    fn contrast_knee() -> u32 {
        (MATRIX_CONTRAST / MATRIX_FRACTION) as u32
    }

    #[test]
    fn composed_cells_spend_the_matrix_share_per_axis() {
        // Every cell gives the matrix MATRIX_FRACTION of its pitch per axis —
        // the same energy the analytic aperture spends — until the line's blend
        // reaches the contrast cap; past the knee it holds at the cap instead
        // of saturating, deliberately under-spending the physical fraction.
        for prescale in 1..=PRESCALE_MAX {
            let mean = compose_axis_mean(prescale);
            let expect = (MATRIX_FRACTION * prescale as f32).min(MATRIX_CONTRAST) / prescale as f32;
            assert!(
                (mean - expect).abs() < 1e-6,
                "axis mean {mean} at prescale {prescale} vs {expect}"
            );
            if prescale <= contrast_knee() {
                assert!(
                    (mean - MATRIX_FRACTION).abs() < 1e-6,
                    "axis mean {mean} at prescale {prescale} vs {MATRIX_FRACTION}"
                );
            }
        }
    }

    #[test]
    fn the_prescale_cap_is_where_the_band_fills_its_texel() {
        // The compose rule holds the band's full energy only while it fits the
        // one texel it lands on; one step past the cap it would saturate and
        // thin, which is exactly where the analytic band reaches a full screen
        // pixel and takes over at the same strength.
        assert!(MATRIX_FRACTION * PRESCALE_MAX as f32 <= 1.0 + 1e-6);
        assert!(MATRIX_FRACTION * (PRESCALE_MAX + 1) as f32 > 1.0);
    }

    #[test]
    fn composed_cells_match_the_analytic_matrix_area() {
        // In two dimensions the union of the two bands is what the analytic
        // path's lit area leaves over, so the two paths agree in the mean below
        // the contrast knee.
        let analytic = 1.0 - APERTURE_FRACTION * APERTURE_FRACTION;
        for prescale in 1..=contrast_knee() {
            let mean = compose_block_mean(prescale);
            assert!(
                (mean - analytic).abs() < 1e-6,
                "block mean {mean} at prescale {prescale} vs {analytic}"
            );
        }
    }

    #[test]
    fn the_capped_handoff_matches_across_the_prescale_seam() {
        // At the prescale cap a composed line is one texel at the contrast cap;
        // just past it the analytic band is one screen pixel at the same cap —
        // per-axis matrix means agree, so a resize across the seam doesn't
        // visibly re-weight the grid.
        let composed = compose_axis_mean(PRESCALE_MAX);
        let analytic = MATRIX_FRACTION * MATRIX_CONTRAST;
        assert!(
            (composed - analytic).abs() < 1e-6,
            "composed {composed} vs analytic {analytic}"
        );
    }

    #[test]
    fn composed_lines_are_evenly_spaced_where_snapping_alternated() {
        // At a fractional zoom the composed grid is perfectly periodic — the
        // intermediate is periodic and a uniform resample cannot quantise it —
        // where the single-pass hairline snapped to alternating 4 and 5 pixel
        // gaps.
        let scale = 4.4;
        let prescale = prescale_factor(scale).unwrap();
        assert_eq!(prescale, 5);

        for cell in 0..20 {
            let gap = composed_line_centre(cell + 1, scale, prescale)
                - composed_line_centre(cell, scale, prescale);
            assert!(
                (gap - scale).abs() < 0.2,
                "gap {gap} at cell {cell} vs {scale}"
            );
        }

        let snapped: Vec<f32> = (0..20)
            .map(|cell| snapped_line_centre(cell + 1, scale) - snapped_line_centre(cell, scale))
            .collect();
        assert!(snapped.iter().any(|gap| (gap - 4.0).abs() < 1e-6));
        assert!(snapped.iter().any(|gap| (gap - 5.0).abs() < 1e-6));
    }

    #[test]
    fn prescale_covers_every_zoom_up_to_its_cap() {
        // A window smaller than the console still composes, at one texel per
        // pixel; past the cap the analytic path takes over.
        assert_eq!(prescale_factor(0.5), Some(1));
        assert_eq!(prescale_factor(1.0), Some(1));
        assert_eq!(prescale_factor(2.0), Some(2));
        assert_eq!(prescale_factor(2.1), Some(3));
        assert_eq!(prescale_factor(PRESCALE_MAX as f32), Some(PRESCALE_MAX));
        assert_eq!(prescale_factor(PRESCALE_MAX as f32 + 0.1), None);
    }

    // ── CRT beam-emission mirrors ────────────────────────────────────────

    /// CPU mirror of the shader's luminance-driven beam half-width.
    fn beam_sigma(luma: f32) -> f32 {
        BEAM_SIGMA_MIN + (BEAM_SIGMA_MAX - BEAM_SIGMA_MIN) * luma.clamp(0.0, 1.0)
    }

    /// CPU mirror of the Gaussian beam profile at distance `d` (row-pitch units).
    fn beam_weight(d: f32, sigma: f32) -> f32 {
        let z = d / sigma;
        (-0.5 * z * z).exp()
    }

    /// Normalized emitted brightness of a uniform grayscale field of the given
    /// `luminance`, sampled at fractional row position `frac`, as a multiple of
    /// the field's flat brightness. 1.0 = brightness preserved. Both bracketing
    /// rows share the field's luminance, so this is the beam sum times the norm.
    fn field_brightness_ratio(frac: f32, luminance: f32) -> f32 {
        let sigma = beam_sigma(luminance);
        BEAM_NORM * (beam_weight(frac, sigma) + beam_weight(1.0 - frac, sigma))
    }

    /// Mean of the two-row beam sum across a full pitch, `2 ∫₀¹ beam(f) df`,
    /// by fine numeric integration — the factor `BEAM_NORM` cancels against.
    fn mean_beam_field(sigma: f32) -> f32 {
        let n = 100_000;
        let mut sum = 0.0f32;
        for i in 0..n {
            let f = (i as f32 + 0.5) / n as f32;
            sum += beam_weight(f, sigma) + beam_weight(1.0 - f, sigma);
        }
        sum / n as f32
    }

    #[test]
    fn beam_width_grows_monotonically_with_luminance() {
        let widths: Vec<f32> = [0.0, 0.25, 0.5, 0.75, 1.0]
            .iter()
            .map(|&l| beam_sigma(l))
            .collect();
        for pair in widths.windows(2) {
            assert!(pair[1] > pair[0], "widths not monotonic: {widths:?}");
        }
        // Bounded by the stated range.
        assert_eq!(beam_sigma(0.0), BEAM_SIGMA_MIN);
        assert_eq!(beam_sigma(1.0), BEAM_SIGMA_MAX);
        const { assert!(BEAM_SIGMA_MIN < BEAM_SIGMA_MAX) };
    }

    #[test]
    fn mid_content_brightness_is_preserved() {
        // BEAM_NORM is fixed as 1/mean_field at the mid-luminance width; verify
        // the pinned constant reproduces unit average brightness there, and that
        // the image never reads dim (average ≥ ~1) for mid content.
        let mean = BEAM_NORM * mean_beam_field(beam_sigma(0.5));
        assert!(
            (mean - 1.0).abs() < 0.01,
            "mid-field mean brightness: {mean}"
        );
        assert!(mean >= 0.99, "mid content must not go dim: {mean}");
    }

    #[test]
    fn bright_line_blooms_and_dark_line_stays_thin() {
        // At the mid-gap (half a pitch from a row centre) a full-white beam is
        // still strong — it nearly fills the pitch — while a black beam has all
        // but vanished, leaving a wide dark gap.
        let white_mid_gap = beam_weight(0.5, beam_sigma(1.0));
        let dark_mid_gap = beam_weight(0.5, beam_sigma(0.0));
        assert!(
            white_mid_gap > 0.5,
            "white beam should fill: {white_mid_gap}"
        );
        assert!(
            dark_mid_gap < 0.1,
            "dark beam should stay thin: {dark_mid_gap}"
        );
        assert!(white_mid_gap > dark_mid_gap * 5.0);
    }

    #[test]
    fn beam_leaves_a_valley_between_rows() {
        // For mid content the row centre reads brighter than the gap between
        // rows — the scanline structure, emergent from emission rather than an
        // applied darkening.
        let at_row_centre = field_brightness_ratio(0.0, 0.5);
        let at_mid_gap = field_brightness_ratio(0.5, 0.5);
        assert!(
            at_row_centre > at_mid_gap,
            "no valley: centre {at_row_centre}, gap {at_mid_gap}"
        );
        // The two-row sum brackets the normalization target: at least full
        // brightness on the row, at most it in the gap.
        assert!(
            at_row_centre >= 1.0,
            "row centre below target: {at_row_centre}"
        );
        assert!(at_mid_gap <= 1.0, "gap above target: {at_mid_gap}");
    }

    #[test]
    fn third_row_contribution_is_negligible() {
        // A neighbour-of-neighbour row is one full pitch beyond the bracketing
        // pair; even at the widest beam it contributes far below perceptible,
        // so two rows suffice.
        assert!(beam_weight(1.5, BEAM_SIGMA_MAX) < 0.02);
    }

    #[test]
    fn emission_degrades_to_plain_below_threshold() {
        // The shader mixes plain→emission by the visibility ramp, so below the
        // onset (weight 0) the CRT path is the plain sharp picture, and at full
        // threshold it is entirely the beam emission.
        assert_eq!(overlay_visibility(OVERLAY_ONSET_PX), 0.0);
        assert_eq!(overlay_visibility(2.9), 0.0);
        assert_eq!(overlay_visibility(OVERLAY_FULL_PX), 1.0);
        let partial = overlay_visibility((OVERLAY_ONSET_PX + OVERLAY_FULL_PX) / 2.0);
        assert!(partial > 0.0 && partial < 1.0);
    }

    #[test]
    fn aperture_lit_fraction_matches_the_constant() {
        // The lit pixel covers APERTURE_FRACTION of the cell per axis, leaving
        // MATRIX_FRACTION as matrix border — so the 2D lit area is that squared.
        let lit = lit_fraction();
        assert!(
            (lit - APERTURE_FRACTION).abs() < 1e-3,
            "one-axis lit fraction {lit} vs {APERTURE_FRACTION}"
        );
        assert_eq!(APERTURE_FRACTION, 1.0 - MATRIX_FRACTION);
        // The border is a minority of the cell — pixels still dominate it.
        const { assert!(MATRIX_FRACTION > 0.0 && MATRIX_FRACTION < 0.5) };
        let area = APERTURE_FRACTION * APERTURE_FRACTION;
        assert!(
            (area - (1.0 - MATRIX_FRACTION).powi(2)).abs() < 1e-6,
            "lit area coverage {area}"
        );
    }

    #[test]
    fn aperture_blend_conserves_energy_at_every_scale() {
        // The anti-aliased blend must show the base in exact proportion to the
        // matrix's true area — the mean lit fraction across a cell stays
        // APERTURE_FRACTION wherever the contrast cap doesn't bind. (The former
        // smoothstep ramp spanned a full screen pixel inward from the aperture
        // edge, blending far more base than the matrix's area and washing black
        // toward grey.)
        for scale in [0.1, 0.15, 0.33] {
            let n = 100_000;
            let lit: f32 = (0..n)
                .map(|i| 1.0 - matrix_coverage((i as f32 + 0.5) / n as f32, scale))
                .sum::<f32>()
                / n as f32;
            assert!(
                (lit - APERTURE_FRACTION).abs() < 5e-3,
                "mean lit fraction {lit} at scale {scale} vs {APERTURE_FRACTION}"
            );
        }
    }

    #[test]
    fn past_the_cap_the_matrix_deliberately_under_spends() {
        // Where a line would exceed the contrast cap — deep zoom, box interior
        // or strong hairline — coverage holds at the cap and the matrix spends
        // less than its physical fraction: large screens keep hairlines rather
        // than saturated lines.
        for scale in [0.0002, 0.02, 0.067] {
            let n = 100_000;
            let mut matrix = 0.0f32;
            for i in 0..n {
                let cover = matrix_coverage((i as f32 + 0.5) / n as f32, scale);
                assert!(cover <= MATRIX_CONTRAST + 1e-6, "cover {cover} over cap");
                matrix += cover;
            }
            let mean = matrix / n as f32;
            assert!(
                mean < MATRIX_FRACTION,
                "mean {mean} at scale {scale} should under-spend {MATRIX_FRACTION}"
            );
        }
    }

    #[test]
    fn hairline_lines_have_phase_invariant_strength() {
        // In the hairline regime every rendered line carries the same coverage
        // wherever the cell boundary falls relative to the pixel grid — a
        // phase-modulated peak beats against the pixel grid as visible moiré
        // at non-integer scales.
        let g = 0.5 - APERTURE_FRACTION * 0.5;
        for scale in [2.5 * g, 3.0 * g, 4.0 * g, 6.0 * g] {
            assert!(2.0 * g < scale, "not hairline at scale {scale}");
            let expect = (2.0 * g / scale).min(MATRIX_CONTRAST);
            let n = 10_000;
            for i in 0..n {
                let cover = matrix_coverage((i as f32 + 0.5) / n as f32, scale);
                assert!(
                    cover == 0.0 || (cover - expect).abs() < 1e-5,
                    "phase-dependent strength {cover} vs {expect} at scale {scale}"
                );
            }
        }
    }

    #[test]
    fn aperture_coverage_is_crisp_at_high_zoom_and_proportional_when_thin() {
        // Deep zoom (tiny footprint): capped base on the boundary, zero at the
        // cell centre, half-covered exactly at the aperture edge.
        assert_eq!(matrix_coverage(0.5, 0.001), MATRIX_CONTRAST);
        assert_eq!(matrix_coverage(0.0, 0.001), 0.0);
        let edge = APERTURE_FRACTION * 0.5;
        assert!((matrix_coverage(edge, 0.001) - 0.5f32.min(MATRIX_CONTRAST)).abs() < 0.01);
        // Matrix thinner than a screen pixel: the boundary fragment shows the
        // band's share of its footprint, up to the cap.
        let g = 0.5 - APERTURE_FRACTION * 0.5;
        let scale = 3.0 * g;
        let expect = ((2.0 * g) / scale).min(MATRIX_CONTRAST);
        assert!((matrix_coverage(0.5, scale) - expect).abs() < 0.01);
    }

    #[test]
    fn aperture_centre_is_the_cell_colour_border_is_the_base() {
        // A dark pixel (0.1) on a pale base (0.9): the cell centre reads the
        // pixel's own flat colour, the matrix border reads the panel base — not
        // a darkened sample of the picture.
        let cell = 0.1;
        let base = 0.9;
        assert_eq!(aperture_color(0.0, cell, base), cell); // cell centre
        assert_eq!(aperture_color(1.0, cell, base), cell);
        assert_eq!(aperture_color(0.5, cell, base), base); // pixel boundary = gap
        // The border is the light base, brighter than the darkened pixel — the
        // inverse of the old dark-line overlay.
        assert!(aperture_color(0.5, cell, base) > aperture_color(0.0, cell, base));
    }

    #[test]
    fn the_grid_never_fades_out() {
        // Every zoom the visibility ramp used to fade the grid through is now
        // composed instead, so the grid stays legible right down to 2× rather
        // than vanishing — energy conservation, not a threshold.
        assert!(prescale_factor(1.0).is_some());
        assert!(prescale_factor(OVERLAY_ONSET_PX).is_some());
        assert!(prescale_factor(OVERLAY_FULL_PX).is_some());
        // Where the analytic aperture does run it is far past the ramp, so its
        // fade is saturated wherever it is selected.
        assert!(PRESCALE_MAX as f32 > OVERLAY_FULL_PX);
        assert_eq!(overlay_visibility(PRESCALE_MAX as f32), 1.0);
    }

    #[test]
    fn the_analytic_aperture_only_runs_in_the_box_regime() {
        // A matrix band of MATRIX_FRACTION spans at least one screen pixel
        // wherever the analytic path is selected, so its hairline branch — kept
        // for the invariant it pins — is unreachable on the grid path.
        let g = 0.5 - APERTURE_FRACTION * 0.5;
        let widest_analytic_footprint = 1.0 / PRESCALE_MAX as f32;
        assert!(2.0 * g >= widest_analytic_footprint);
    }

    #[test]
    fn overlay_hidden_below_onset() {
        // Below ~3 screen pixels per source pixel the overlay is fully faded out.
        assert_eq!(overlay_visibility(1.0), 0.0);
        assert_eq!(overlay_visibility(OVERLAY_ONSET_PX), 0.0);
        assert_eq!(overlay_visibility(2.9), 0.0);
    }

    #[test]
    fn overlay_full_above_full_threshold() {
        assert_eq!(overlay_visibility(OVERLAY_FULL_PX), 1.0);
        assert_eq!(overlay_visibility(10.0), 1.0);
    }

    #[test]
    fn overlay_ramps_between_thresholds() {
        let mid = overlay_visibility((OVERLAY_ONSET_PX + OVERLAY_FULL_PX) / 2.0);
        assert!(mid > 0.0 && mid < 1.0);
    }

    #[test]
    fn shader_source_resolves_all_placeholders() {
        // The shader the pipeline loads is this injected string — assert the
        // beam and aperture constants reach it, ruling out a stale-source path.
        let source = shader_source();
        assert!(!source.contains("__"));
        assert!(source.contains("const OVERLAY_ONSET_PX: f32 = 3.0;"));
        assert!(source.contains("const OVERLAY_FULL_PX: f32 = 6.0;"));
        assert!(source.contains("const BEAM_SIGMA_MIN: f32 = 0.18;"));
        assert!(source.contains("const BEAM_SIGMA_MAX: f32 = 0.52;"));
        assert!(source.contains(&format!("const BEAM_NORM: f32 = {BEAM_NORM};")));
        assert!(source.contains(&format!(
            "const APERTURE_FRACTION: f32 = {APERTURE_FRACTION};"
        )));
        assert!(source.contains(&format!("const MATRIX_FRACTION: f32 = {MATRIX_FRACTION};")));
        assert!(source.contains(&format!("const MATRIX_CONTRAST: f32 = {MATRIX_CONTRAST};")));
    }
}
