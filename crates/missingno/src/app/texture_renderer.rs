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
/// Peak darkening of a grid or scanline line, as a fraction of pixel brightness.
const GRID_DARKEN: f32 = 0.15;
const SCANLINE_DARKEN: f32 = 0.22;
/// Darkening-band geometry as a fraction of the source-pixel pitch, per effect.
/// `*_FRACTION` is the band's full width (where darkening reaches zero);
/// `*_CORE_FRACTION` is an inner core held at full darkness, with smoothstep
/// shoulders between core and band. The flat core keeps the gap genuinely dark
/// across most of its width instead of tapering to a sub-perceptual point, and
/// the whole band is proportional to the on-screen row height so it holds its
/// look from a small window up to a 4K panel. Shared with the fragment shader.
const GRID_LINE_FRACTION: f32 = 0.2;
const GRID_CORE_FRACTION: f32 = 0.1;
const SCANLINE_FRACTION: f32 = 0.5;
const SCANLINE_CORE_FRACTION: f32 = 0.3;

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
        }
    }

    /// Draw the given cosmetic overlay over the picture.
    pub fn overlay(mut self, overlay: ScreenOverlay) -> Self {
        self.overlay = overlay;
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
                pipeline.update_vertices(queue, self.id, *bounds, viewport, self.overlay);

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
                    pipeline.update_vertices(queue, self.id, *bounds, viewport, self.overlay);
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
            },
            Vertex {
                position: [right, top],
                tex_coords: [1.0, 0.0],
                overlay,
            },
            Vertex {
                position: [left, bottom],
                tex_coords: [0.0, 1.0],
                overlay,
            },
            Vertex {
                position: [left, bottom],
                tex_coords: [0.0, 1.0],
                overlay,
            },
            Vertex {
                position: [right, top],
                tex_coords: [1.0, 0.0],
                overlay,
            },
            Vertex {
                position: [right, bottom],
                tex_coords: [1.0, 1.0],
                overlay,
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
        .replace("__GRID_DARKEN__", &wgsl_f32(GRID_DARKEN))
        .replace("__SCANLINE_DARKEN__", &wgsl_f32(SCANLINE_DARKEN))
        .replace("__GRID_LINE_FRACTION__", &wgsl_f32(GRID_LINE_FRACTION))
        .replace("__GRID_CORE_FRACTION__", &wgsl_f32(GRID_CORE_FRACTION))
        .replace("__SCANLINE_FRACTION__", &wgsl_f32(SCANLINE_FRACTION))
        .replace("__SCANLINE_CORE_FRACTION__", &wgsl_f32(SCANLINE_CORE_FRACTION))
}

const SHADER_TEMPLATE: &str = r#"
struct VertexInput {
    @location(0) position: vec2<f32>,
    @location(1) tex_coords: vec2<f32>,
    @location(2) overlay: f32,
}

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) tex_coords: vec2<f32>,
    @location(1) @interpolate(flat) overlay: f32,
}

const OVERLAY_ONSET_PX: f32 = __OVERLAY_ONSET_PX__;
const OVERLAY_FULL_PX: f32 = __OVERLAY_FULL_PX__;
const GRID_DARKEN: f32 = __GRID_DARKEN__;
const SCANLINE_DARKEN: f32 = __SCANLINE_DARKEN__;
const GRID_LINE_FRACTION: f32 = __GRID_LINE_FRACTION__;
const GRID_CORE_FRACTION: f32 = __GRID_CORE_FRACTION__;
const SCANLINE_FRACTION: f32 = __SCANLINE_FRACTION__;
const SCANLINE_CORE_FRACTION: f32 = __SCANLINE_CORE_FRACTION__;

@vertex
fn vs_main(input: VertexInput) -> VertexOutput {
    var output: VertexOutput;
    output.position = vec4<f32>(input.position, 0.0, 1.0);
    output.tex_coords = input.tex_coords;
    output.overlay = input.overlay;
    return output;
}

@group(0) @binding(0)
var texture: texture_2d<f32>;

@group(0) @binding(1)
var texture_sampler: sampler;

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
    var color = textureSample(texture, texture_sampler, snapped);

    // Cosmetic device-simulation overlay: 1 = LCD pixel grid, 2 = CRT
    // scanlines. Both darken a ~1-screen-pixel line at the source-pixel
    // boundary, fading out below OVERLAY_ONSET_PX screen pixels per source
    // pixel where the lines would merge into flat dimming.
    let mode = input.overlay;
    if (mode > 0.5) {
        let screen_px_per_source = vec2(1.0) / max(scale, vec2(0.0001));
        let vis = smoothstep(
            vec2(OVERLAY_ONSET_PX), vec2(OVERLAY_FULL_PX), screen_px_per_source);

        // Distance to the nearest cell boundary in the sampler's half-texel
        // frame: integer frac lands between rendered rows/columns, so the
        // darkening sits in the inter-cell gap, not through a cell's centre.
        let dist = min(frac, vec2(1.0) - frac);

        var darken = 0.0;
        if (mode < 1.5) {
            // LCD grid: a thin line at every cell boundary, both axes. A flat
            // full-dark core with smoothstep shoulders, its width a fixed
            // fraction of the cell pitch so it holds at any output resolution.
            let core = GRID_CORE_FRACTION * 0.5;
            let band = GRID_LINE_FRACTION * 0.5;
            let line = vec2(1.0) - smoothstep(vec2(core), vec2(band), dist);
            darken = GRID_DARKEN * max(line.x * vis.x, line.y * vis.y);
        } else {
            // CRT scanline: a flat-bottomed dark valley between rows — full
            // darkness across the core, smoothstep shoulders out to the band
            // edge — sized as a fixed fraction of the row pitch, so the gap
            // stays genuinely dark across its width at any output resolution.
            let core = SCANLINE_CORE_FRACTION * 0.5;
            let band = SCANLINE_FRACTION * 0.5;
            let line = 1.0 - smoothstep(core, band, dist.y);
            darken = SCANLINE_DARKEN * line * vis.y;
        }
        color = vec4<f32>(color.rgb * (1.0 - darken), color.a);
    }

    return color;
}
"#;

#[cfg(test)]
mod tests {
    use super::*;

    /// The CPU mirror of the shader's visibility ramp, exercising the shared
    /// overlay thresholds.
    fn overlay_visibility(screen_px_per_source: f32) -> f32 {
        let t = ((screen_px_per_source - OVERLAY_ONSET_PX) / (OVERLAY_FULL_PX - OVERLAY_ONSET_PX))
            .clamp(0.0, 1.0);
        t * t * (3.0 - 2.0 * t)
    }

    /// The sampler's texel-space coordinate for a normalized tex-coord —
    /// `tex_coord * tex_size - 0.5`, matching the shader's half-texel offset.
    /// Its integers fall on the boundaries the sharp sampler renders between
    /// rows; its half-integers on a rendered row's plateau centre.
    fn texel_coord(tex_coord: f32, tex_size: f32) -> f32 {
        tex_coord * tex_size - 0.5
    }

    /// CPU mirror of the shader's flat-bottomed line term as a function of the
    /// distance (in row units) to the nearest rendered-row boundary: full
    /// darkness within `core_half`, smoothstep shoulders out to `band_half`,
    /// zero beyond. Both are fixed fractions of the row pitch.
    fn line_at_dist(dist: f32, core_half: f32, band_half: f32) -> f32 {
        let t = ((dist - core_half) / (band_half - core_half)).clamp(0.0, 1.0);
        1.0 - t * t * (3.0 - 2.0 * t)
    }

    /// The line term at a position in the sampler's half-texel frame; the
    /// darkening peaks on the integer (between rendered rows) and vanishes at
    /// the half-integer (a rendered row's centre).
    fn overlay_line(texel_axis: f32, core_half: f32, band_half: f32) -> f32 {
        let frac = texel_axis - texel_axis.floor();
        let dist = frac.min(1.0 - frac);
        line_at_dist(dist, core_half, band_half)
    }

    /// Screen pixels within one row pitch whose darkening reaches at least
    /// `threshold` of the peak, at `screen_px_per_row` output scale — the
    /// perceived width of the dark line, which straddles the pitch boundary.
    fn pixels_at_least(
        core_half: f32,
        band_half: f32,
        screen_px_per_row: f32,
        threshold: f32,
    ) -> usize {
        let n = screen_px_per_row.round() as usize;
        (0..n)
            .filter(|&i| {
                let row_pos = (i as f32 + 0.5) / screen_px_per_row;
                let dist = row_pos.min(1.0 - row_pos);
                line_at_dist(dist, core_half, band_half) >= threshold
            })
            .count()
    }

    #[test]
    fn overlay_darkening_aligns_to_rendered_row_boundaries() {
        // A 228-line NTSC VCS field is the motivating case.
        let tex_size = 228.0;
        let core_half = SCANLINE_CORE_FRACTION / 2.0;
        let band_half = SCANLINE_FRACTION / 2.0;

        // The sharp sampler renders a row transition at an integer texel, which
        // is tex_coord = (k + 0.5)/tex_size. The darkening must be full there —
        // in the gap between rendered rows.
        let rendered_boundary = texel_coord(3.5 / tex_size, tex_size);
        assert_eq!(rendered_boundary, 3.0);
        assert!((overlay_line(rendered_boundary, core_half, band_half) - 1.0).abs() < 1e-6);

        // A rendered row's plateau centre is half a texel off — a half-integer
        // texel, tex_coord = (k + 1)/tex_size — where the darkening must vanish
        // so it never cuts through the row.
        let row_centre = texel_coord(4.0 / tex_size, tex_size);
        assert_eq!(row_centre, 3.5);
        assert_eq!(overlay_line(row_centre, core_half, band_half), 0.0);

        // The pre-fix placement dropped the half-texel offset, measuring from
        // `tex_coord * tex_size` whose integers are the row centres — peaking on
        // the row, not between rows. Pin that this frame differs by half a texel.
        let unshifted_at_boundary = 3.5 / tex_size * tex_size;
        assert_eq!(unshifted_at_boundary.fract(), 0.5);
    }

    #[test]
    fn overlay_band_width_is_a_fixed_fraction_of_the_row() {
        let core_half = SCANLINE_CORE_FRACTION / 2.0;
        let band_half = SCANLINE_FRACTION / 2.0;

        // The band reaches zero exactly band_half from the boundary, and holds
        // full darkness within core_half — in row units, the same whatever the
        // on-screen row height.
        assert_eq!(overlay_line(3.0 + band_half, core_half, band_half), 0.0);
        assert!(overlay_line(3.0 + band_half * 0.99, core_half, band_half) > 0.0);
        assert_eq!(overlay_line(3.0 + core_half * 0.99, core_half, band_half), 1.0);

        // So in screen pixels the band tracks the row height: 3x wider at 12
        // screen px/row than at 4, and the row-fraction is identical at both —
        // not a resolution-fixed hairline.
        let width_rows = 2.0 * band_half;
        let width_px_at_4 = width_rows * 4.0;
        let width_px_at_12 = width_rows * 12.0;
        assert!((width_px_at_12 / width_px_at_4 - 3.0).abs() < 1e-6);
        assert!(((width_px_at_4 / 4.0) - (width_px_at_12 / 12.0)).abs() < 1e-6);
    }

    #[test]
    fn scanline_valley_reads_as_dark_across_its_width_at_4k() {
        // A 4K-class zoom: ~12 screen pixels per source row.
        let core_half = SCANLINE_CORE_FRACTION / 2.0;
        let band_half = SCANLINE_FRACTION / 2.0;
        let full = pixels_at_least(core_half, band_half, 12.0, 0.999);
        let half = pixels_at_least(core_half, band_half, 12.0, 0.5);

        // The flat core holds several pixels at full darkness, and most of the
        // valley reaches at least half — not the sub-perceptual taper a pure
        // gradient leaves (~2 half-dark pixels, none at full).
        assert!(full >= 3, "full-dark pixels at 12px/row: {full}");
        assert!(half >= 4, "half-dark pixels at 12px/row: {half}");
    }

    #[test]
    fn grid_line_stays_visible_at_4k() {
        let core_half = GRID_CORE_FRACTION / 2.0;
        let band_half = GRID_LINE_FRACTION / 2.0;
        let full = pixels_at_least(core_half, band_half, 12.0, 0.999);
        let half = pixels_at_least(core_half, band_half, 12.0, 0.5);

        // Thinner than the scanline valley by design, but still a genuinely dark
        // core rather than a single-pixel taper.
        assert!(full >= 1, "full-dark pixels at 12px/row: {full}");
        assert!(half >= 2, "half-dark pixels at 12px/row: {half}");
    }

    #[test]
    fn grid_line_is_thinner_than_the_scanline_valley() {
        assert!(GRID_LINE_FRACTION < SCANLINE_FRACTION);
        assert!(GRID_CORE_FRACTION < SCANLINE_CORE_FRACTION);
        // The full-dark core sits inside the band, which stays within a source
        // pixel (half-band ≤ 0.5 row).
        assert!(GRID_CORE_FRACTION < GRID_LINE_FRACTION);
        assert!(SCANLINE_CORE_FRACTION < SCANLINE_FRACTION);
        assert!(SCANLINE_FRACTION / 2.0 <= 0.5);
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
        // reshaped band constants reach it, ruling out a stale-source path.
        let source = shader_source();
        assert!(!source.contains("__"));
        assert!(source.contains("const OVERLAY_ONSET_PX: f32 = 3.0;"));
        assert!(source.contains("const OVERLAY_FULL_PX: f32 = 6.0;"));
        assert!(source.contains("const SCANLINE_FRACTION: f32 = 0.5;"));
        assert!(source.contains("const SCANLINE_CORE_FRACTION: f32 = 0.3;"));
        assert!(source.contains("const GRID_LINE_FRACTION: f32 = 0.2;"));
        assert!(source.contains("const GRID_CORE_FRACTION: f32 = 0.1;"));
    }
}
