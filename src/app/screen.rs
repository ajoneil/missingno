use iced::{Color, Rectangle, wgpu, widget::shader};
use rgb::RGB8;
use std::sync::Mutex;

use crate::game_boy::video::{
    palette::Palette,
    screen::{self, Screen},
};

/// Shader-based screen renderer for efficient Game Boy display
impl<Message> shader::Program<Message> for Screen {
    type State = ();
    type Primitive = ScreenPrimitive;

    fn draw(
        &self,
        _state: &Self::State,
        _cursor: iced::mouse::Cursor,
        bounds: Rectangle,
    ) -> Self::Primitive {
        // Convert Game Boy pixel data to RGB8 buffer
        let mut pixels = Vec::with_capacity(
            screen::PIXELS_PER_LINE as usize * screen::NUM_SCANLINES as usize * 4,
        );

        for y in 0..screen::NUM_SCANLINES {
            for x in 0..screen::PIXELS_PER_LINE {
                let color = Palette::MONOCHROME_GREEN.color(self.pixel(x, y));
                pixels.push(color.r);
                pixels.push(color.g);
                pixels.push(color.b);
                pixels.push(255); // Alpha
            }
        }

        ScreenPrimitive {
            state: Mutex::new(PrimitiveState::Pending { pixels, bounds }),
        }
    }
}

#[derive(Debug)]
pub struct ScreenPrimitive {
    state: Mutex<PrimitiveState>,
}

#[derive(Debug)]
enum PrimitiveState {
    Pending { pixels: Vec<u8>, bounds: Rectangle },
    Prepared { bounds: Rectangle },
}

impl shader::Primitive for ScreenPrimitive {
    type Pipeline = ScreenPipeline;

    fn prepare(
        &self,
        pipeline: &mut Self::Pipeline,
        _device: &wgpu::Device,
        queue: &wgpu::Queue,
        bounds: &Rectangle,
        _viewport: &shader::Viewport,
    ) {
        let mut state = self.state.lock().unwrap();

        match &*state {
            PrimitiveState::Pending { pixels, bounds } => {
                // Update the texture with new pixel data
                queue.write_texture(
                    wgpu::TexelCopyTextureInfo {
                        texture: &pipeline.texture,
                        mip_level: 0,
                        origin: wgpu::Origin3d::ZERO,
                        aspect: wgpu::TextureAspect::All,
                    },
                    pixels,
                    wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(screen::PIXELS_PER_LINE as u32 * 4),
                        rows_per_image: Some(screen::NUM_SCANLINES as u32),
                    },
                    wgpu::Extent3d {
                        width: screen::PIXELS_PER_LINE as u32,
                        height: screen::NUM_SCANLINES as u32,
                        depth_or_array_layers: 1,
                    },
                );

                // Update vertex buffer for new bounds
                pipeline.update_vertices(queue, *bounds);

                *state = PrimitiveState::Prepared { bounds: *bounds };
            }
            PrimitiveState::Prepared { bounds: old_bounds } => {
                // Only update vertices if bounds changed
                if old_bounds != bounds {
                    pipeline.update_vertices(queue, *bounds);
                    *state = PrimitiveState::Prepared { bounds: *bounds };
                }
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
        let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("screen_shader_pass"),
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

        render_pass.set_viewport(
            viewport.x as f32,
            viewport.y as f32,
            viewport.width as f32,
            viewport.height as f32,
            0.0,
            1.0,
        );
        render_pass.set_scissor_rect(viewport.x, viewport.y, viewport.width, viewport.height);
        render_pass.set_pipeline(&pipeline.render_pipeline);
        render_pass.set_bind_group(0, &pipeline.bind_group, &[]);
        render_pass.set_vertex_buffer(0, pipeline.vertex_buffer.slice(..));
        render_pass.draw(0..6, 0..1);
    }
}

pub struct ScreenPipeline {
    render_pipeline: wgpu::RenderPipeline,
    texture: wgpu::Texture,
    bind_group: wgpu::BindGroup,
    vertex_buffer: wgpu::Buffer,
}

impl shader::Pipeline for ScreenPipeline {
    fn new(device: &wgpu::Device, _queue: &wgpu::Queue, format: wgpu::TextureFormat) -> Self {
        // Create texture for Game Boy screen (160x144 pixels)
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("screen_texture"),
            size: wgpu::Extent3d {
                width: screen::PIXELS_PER_LINE as u32,
                height: screen::NUM_SCANLINES as u32,
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

        // Create sampler with nearest-neighbor filtering for crisp pixels
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("screen_sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            mipmap_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });

        // Create bind group layout
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("screen_bind_group_layout"),
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

        // Create bind group
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("screen_bind_group"),
            layout: &bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&texture_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
            ],
        });

        // Create shader module
        let shader_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("screen_shader"),
            source: wgpu::ShaderSource::Wgsl(std::borrow::Cow::Borrowed(SHADER_SOURCE)),
        });

        // Create pipeline layout
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("screen_pipeline_layout"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });

        // Create render pipeline
        let render_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("screen_render_pipeline"),
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

        // Create initial vertex buffer
        let vertex_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("screen_vertex_buffer"),
            size: std::mem::size_of::<[Vertex; 6]>() as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        Self {
            render_pipeline,
            texture,
            bind_group,
            vertex_buffer,
        }
    }
}

impl ScreenPipeline {
    fn update_vertices(&self, queue: &wgpu::Queue, bounds: Rectangle) {
        let vertices = create_quad_vertices(bounds);
        queue.write_buffer(&self.vertex_buffer, 0, bytemuck::cast_slice(&vertices));
    }
}

#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct Vertex {
    position: [f32; 2],
    tex_coords: [f32; 2],
}

fn create_quad_vertices(_bounds: Rectangle) -> [Vertex; 6] {
    // Use normalized device coordinates (-1 to 1) to fill the entire viewport
    [
        // Triangle 1
        Vertex {
            position: [-1.0, -1.0],
            tex_coords: [0.0, 1.0],
        },
        Vertex {
            position: [1.0, -1.0],
            tex_coords: [1.0, 1.0],
        },
        Vertex {
            position: [-1.0, 1.0],
            tex_coords: [0.0, 0.0],
        },
        // Triangle 2
        Vertex {
            position: [-1.0, 1.0],
            tex_coords: [0.0, 0.0],
        },
        Vertex {
            position: [1.0, -1.0],
            tex_coords: [1.0, 1.0],
        },
        Vertex {
            position: [1.0, 1.0],
            tex_coords: [1.0, 0.0],
        },
    ]
}

const SHADER_SOURCE: &str = r#"
struct VertexInput {
    @location(0) position: vec2<f32>,
    @location(1) tex_coords: vec2<f32>,
}

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) tex_coords: vec2<f32>,
}

@vertex
fn vs_main(input: VertexInput) -> VertexOutput {
    var output: VertexOutput;
    output.position = vec4<f32>(input.position, 0.0, 1.0);
    output.tex_coords = input.tex_coords;
    return output;
}

@group(0) @binding(0)
var screen_texture: texture_2d<f32>;

@group(0) @binding(1)
var screen_sampler: sampler;

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    return textureSample(screen_texture, screen_sampler, input.tex_coords);
}
"#;

pub fn iced_color(color: RGB8) -> Color {
    Color::from_rgb8(color.r, color.g, color.b)
}
