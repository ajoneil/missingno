use iced::{Rectangle, wgpu, widget::shader};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

static NEXT_TEXTURE_ID: AtomicU64 = AtomicU64::new(0);

/// Reusable GPU texture renderer for pixel-based graphics
pub struct TextureRenderer {
    id: u64,
    width: u32,
    height: u32,
    pixels: Vec<u8>,
}

impl TextureRenderer {
    pub fn with_pixels(width: u32, height: u32, pixels: Vec<u8>) -> Self {
        assert_eq!(pixels.len(), (width * height * 4) as usize);
        Self {
            id: NEXT_TEXTURE_ID.fetch_add(1, Ordering::Relaxed),
            width,
            height,
            pixels,
        }
    }
}

impl<Message> shader::Program<Message> for TextureRenderer {
    type State = ();
    type Primitive = TexturePrimitive;

    fn draw(
        &self,
        _state: &Self::State,
        _cursor: iced::mouse::Cursor,
        bounds: Rectangle,
    ) -> Self::Primitive {
        TexturePrimitive {
            id: self.id,
            state: Mutex::new(PrimitiveState::Pending {
                width: self.width,
                height: self.height,
                pixels: self.pixels.clone(),
                bounds,
            }),
        }
    }
}

#[derive(Debug)]
pub struct TexturePrimitive {
    id: u64,
    state: Mutex<PrimitiveState>,
}

#[derive(Debug)]
enum PrimitiveState {
    Pending {
        width: u32,
        height: u32,
        pixels: Vec<u8>,
        bounds: Rectangle,
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
        _viewport: &shader::Viewport,
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
                bounds,
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
                pipeline.update_vertices(queue, bounds);

                *state = PrimitiveState::Prepared {
                    width,
                    height,
                    bounds,
                };
            }
            PrimitiveState::Prepared {
                width,
                height,
                bounds: old_bounds,
            } => {
                pipeline.ensure_texture(device, self.id, width, height);

                if &old_bounds != bounds {
                    pipeline.update_vertices(queue, *bounds);
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
        render_pass.set_bind_group(0, &texture_data.bind_group, &[]);
        render_pass.set_vertex_buffer(0, pipeline.vertex_buffer.slice(..));
        render_pass.draw(0..6, 0..1);
    }
}

struct TextureData {
    texture: wgpu::Texture,
    bind_group: wgpu::BindGroup,
    width: u32,
    height: u32,
}

pub struct TexturePipeline {
    render_pipeline: wgpu::RenderPipeline,
    bind_group_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    vertex_buffer: wgpu::Buffer,
    textures: Arc<Mutex<HashMap<u64, TextureData>>>,
}

impl shader::Pipeline for TexturePipeline {
    fn new(device: &wgpu::Device, _queue: &wgpu::Queue, format: wgpu::TextureFormat) -> Self {
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("texture_renderer_sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            mipmap_filter: wgpu::FilterMode::Nearest,
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
            source: wgpu::ShaderSource::Wgsl(std::borrow::Cow::Borrowed(SHADER_SOURCE)),
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

        let vertex_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("texture_renderer_vertex_buffer"),
            size: std::mem::size_of::<[Vertex; 6]>() as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        Self {
            render_pipeline,
            bind_group_layout,
            sampler,
            vertex_buffer,
            textures: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

impl TexturePipeline {
    fn ensure_texture(&self, device: &wgpu::Device, id: u64, width: u32, height: u32) {
        let mut textures = self.textures.lock().unwrap();

        let needs_creation = textures
            .get(&id)
            .map_or(true, |data| data.width != width || data.height != height);

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

            textures.insert(
                id,
                TextureData {
                    texture,
                    bind_group,
                    width,
                    height,
                },
            );
        }
    }

    fn update_vertices(&self, queue: &wgpu::Queue, _bounds: Rectangle) {
        queue.write_buffer(&self.vertex_buffer, 0, bytemuck::cast_slice(&QUAD_VERTICES));
    }
}

#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct Vertex {
    position: [f32; 2],
    tex_coords: [f32; 2],
}

const QUAD_VERTICES: [Vertex; 6] = [
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
];

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
var texture: texture_2d<f32>;

@group(0) @binding(1)
var texture_sampler: sampler;

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    return textureSample(texture, texture_sampler, input.tex_coords);
}
"#;
