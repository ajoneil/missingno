use iced::{Rectangle, wgpu, widget::shader};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

static NEXT_TEXTURE_ID: AtomicU64 = AtomicU64::new(0);

/// Screen pixels per source pixel below which an overlay's darkening lines would
/// merge into flat dimming rather than separate pixels, so they fade out; full
/// strength by [`OVERLAY_FULL_PX`]. Shared with the fragment shader, which reads
/// the same ramp.
pub const OVERLAY_ONSET_PX: f32 = 3.0;
pub const OVERLAY_FULL_PX: f32 = 6.0;
/// The LCD grid is a pixel *aperture*, not a darkening laid on top. Both Game
/// Boy panels are reflective (no backlight): the unlit state is the pale panel
/// base, pixels darken against it, and the inter-pixel matrix is a border that
/// exposes that light base — so a fragment either lands inside a lit pixel or in
/// the base-coloured gap. `APERTURE_FRACTION` is the fraction of the cell pitch,
/// per axis, that the lit pixel fills; the remaining border is the reflective
/// base. ~0.88 reads like a photographed DMG screen — pixels nearly touching
/// with a thin base gridline between (linear coverage 0.88/axis, 0.774 of area).
/// Shared with the fragment shader.
const APERTURE_FRACTION: f32 = 0.88;

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

/// Reusable GPU texture renderer for pixel-based graphics
pub struct TextureRenderer {
    id: u64,
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
            id: NEXT_TEXTURE_ID.fetch_add(1, Ordering::Relaxed),
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
            id: self.id,
            overlay: self.overlay.shader_mode(),
            panel_base: self.panel_base,
            state: Mutex::new(PrimitiveState::Pending {
                width: self.width,
                height: self.height,
                pixels: self.pixels.clone(),
            }),
        }
    }
}

#[derive(Debug)]
pub struct TexturePrimitive {
    id: u64,
    overlay: f32,
    panel_base: [f32; 3],
    state: Mutex<PrimitiveState>,
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
                // Use prepare()'s bounds (screen-space) not draw()'s bounds
                // (content-space), so the texture renders at the correct
                // position when inside a scrollable.
                pipeline.update_vertices(
                    queue,
                    self.id,
                    *bounds,
                    viewport,
                    self.overlay,
                    self.panel_base,
                );

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

                if &old_bounds != bounds {
                    pipeline.update_vertices(
                        queue,
                        self.id,
                        *bounds,
                        viewport,
                        self.overlay,
                        self.panel_base,
                    );
                }
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
        render_pass.set_bind_group(0, &texture_data.bind_group, &[]);
        render_pass.set_vertex_buffer(0, texture_data.vertex_buffer.slice(..));
        render_pass.draw(0..6, 0..1);
    }
}

struct TextureData {
    texture: wgpu::Texture,
    bind_group: wgpu::BindGroup,
    vertex_buffer: wgpu::Buffer,
    width: u32,
    height: u32,
}

pub struct TexturePipeline {
    render_pipeline: wgpu::RenderPipeline,
    bind_group_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    textures: Arc<Mutex<HashMap<u64, TextureData>>>,
}

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

        Self {
            render_pipeline,
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

            textures.insert(
                id,
                TextureData {
                    texture,
                    bind_group,
                    vertex_buffer,
                    width,
                    height,
                },
            );
        }
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

    // Remap the fractional part: hold at 0 for most of the texel,
    // then ramp linearly across one screen pixel at the boundary.
    let sharp = clamp((frac - (vec2(1.0) - scale)) / scale, vec2(0.0), vec2(1.0));

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
        // to the panel base across the border. The cell centre sits at frac 0.5;
        // its edges (frac 0 and 1) are the matrix gap.
        let cell_centre = (texel_floor + vec2(0.5)) / tex_size;
        let cell_color = textureSample(texture, texture_sampler, cell_centre).rgb;
        let d = abs(frac - vec2(0.5));
        let aperture = vec2(APERTURE_FRACTION * 0.5);
        // Soften the matrix edge over ~one screen pixel so it never aliases.
        let aa = clamp(scale, vec2(0.0001), aperture);
        let inside = (vec2(1.0) - smoothstep(aperture - aa, aperture, d));
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

    // ── LCD aperture mirrors ─────────────────────────────────────────────

    /// CPU mirror of the shader's hard-edged aperture membership at intra-cell
    /// position `frac` (per axis): 1 inside the lit pixel, 0 in the matrix
    /// border. The aperture is centred on the cell (`frac` 0.5), spanning
    /// `APERTURE_FRACTION` of the pitch, so its edges sit at `0.5 ± aperture`.
    fn aperture_inside(frac: f32) -> f32 {
        let aperture = APERTURE_FRACTION * 0.5;
        let d = (frac - 0.5).abs();
        if d <= aperture { 1.0 } else { 0.0 }
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
        assert!(BEAM_SIGMA_MIN < BEAM_SIGMA_MAX);
    }

    #[test]
    fn mid_content_brightness_is_preserved() {
        // BEAM_NORM is fixed as 1/mean_field at the mid-luminance width; verify
        // the pinned constant reproduces unit average brightness there, and that
        // the image never reads dim (average ≥ ~1) for mid content.
        let mean = BEAM_NORM * mean_beam_field(beam_sigma(0.5));
        assert!((mean - 1.0).abs() < 0.01, "mid-field mean brightness: {mean}");
        assert!(mean >= 0.99, "mid content must not go dim: {mean}");
    }

    #[test]
    fn bright_line_blooms_and_dark_line_stays_thin() {
        // At the mid-gap (half a pitch from a row centre) a full-white beam is
        // still strong — it nearly fills the pitch — while a black beam has all
        // but vanished, leaving a wide dark gap.
        let white_mid_gap = beam_weight(0.5, beam_sigma(1.0));
        let dark_mid_gap = beam_weight(0.5, beam_sigma(0.0));
        assert!(white_mid_gap > 0.5, "white beam should fill: {white_mid_gap}");
        assert!(dark_mid_gap < 0.1, "dark beam should stay thin: {dark_mid_gap}");
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
        assert!(at_row_centre >= 1.0, "row centre below target: {at_row_centre}");
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
        // the rest as matrix border — so the 2D lit area is that squared.
        let lit = lit_fraction();
        assert!(
            (lit - APERTURE_FRACTION).abs() < 1e-3,
            "one-axis lit fraction {lit} vs {APERTURE_FRACTION}"
        );
        // The border is a thin minority of the cell — pixels nearly touch.
        assert!(APERTURE_FRACTION > 0.8 && APERTURE_FRACTION < 1.0);
        let area = APERTURE_FRACTION * APERTURE_FRACTION;
        assert!(area > 0.75, "lit area coverage {area}");
    }

    #[test]
    fn aperture_centre_is_the_cell_colour_border_is_the_base() {
        // A dark pixel (0.1) on a pale base (0.9): the cell centre reads the
        // pixel's own flat colour, the matrix border reads the panel base — not
        // a darkened sample of the picture.
        let cell = 0.1;
        let base = 0.9;
        assert_eq!(aperture_color(0.5, cell, base), cell); // cell centre
        assert_eq!(aperture_color(0.0, cell, base), base); // cell edge = gap
        assert_eq!(aperture_color(1.0, cell, base), base);
        // The border is the light base, brighter than the darkened pixel — the
        // inverse of the old dark-line overlay.
        assert!(aperture_color(0.0, cell, base) > aperture_color(0.5, cell, base));
    }

    #[test]
    fn aperture_degrades_to_plain_below_threshold() {
        // The shader mixes plain→aperture by the visibility ramp, so below the
        // onset (weight 0) the grid path is the plain sharp picture, and at full
        // threshold it is entirely the aperture model.
        assert_eq!(overlay_visibility(OVERLAY_ONSET_PX), 0.0);
        assert_eq!(overlay_visibility(2.9), 0.0);
        assert_eq!(overlay_visibility(OVERLAY_FULL_PX), 1.0);
        let partial = overlay_visibility((OVERLAY_ONSET_PX + OVERLAY_FULL_PX) / 2.0);
        assert!(partial > 0.0 && partial < 1.0);
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
    }
}
