//! Offscreen wgpu renderer for the viewport (D-48). Draws into its own colour
//! (4× MSAA, resolved) and depth targets; the app shows the result as an egui
//! image. Independent of egui, so it can be tested headless.

use eframe::wgpu;
use glam::Mat4;
use wgpu::util::DeviceExt;

pub const COLOR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Vertex {
    pub pos: [f32; 3],
    pub normal: [f32; 3],
    pub color: [f32; 4],
}

#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Globals {
    view_proj: [[f32; 4]; 4],
    light_dir: [f32; 4],
    /// Camera position relative to the render origin.
    eye: [f32; 4],
}

/// Geometry for one frame. `scene_*` change rarely (keyed by `scene_version`);
/// `overlay_*` are rebuilt every frame (hover, previews, snap markers).
pub struct FrameData<'a> {
    pub view_proj: Mat4,
    /// Camera position relative to the render origin.
    pub eye: [f32; 3],
    pub scene_version: u64,
    pub scene_tris: &'a [Vertex],
    pub scene_lines: &'a [Vertex],
    pub overlay_tris: &'a [Vertex],
    /// Scene faces do not hide edges behind them.
    pub xray: bool,
    pub overlay_lines: &'a [Vertex],
    pub clear: [f64; 3],
}

struct Targets {
    size: [u32; 2],
    msaa: wgpu::TextureView,
    /// Kept alive alongside its view.
    _resolve: wgpu::Texture,
    resolve_view: wgpu::TextureView,
    depth: wgpu::TextureView,
}

pub struct Renderer {
    mesh_pipeline: wgpu::RenderPipeline,
    /// Faces that do not hide what is behind them (X-ray style).
    xray_pipeline: wgpu::RenderPipeline,
    line_pipeline: wgpu::RenderPipeline,
    globals: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    scene_version: Option<u64>,
    scene_tris: Option<(wgpu::Buffer, u32)>,
    scene_lines: Option<(wgpu::Buffer, u32)>,
    targets: Option<Targets>,
    samples: u32,
}

impl Renderer {
    pub fn new(device: &wgpu::Device) -> Renderer {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("ferro shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shader.wgsl").into()),
        });
        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("globals"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let globals = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("globals"),
            size: std::mem::size_of::<Globals>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("globals"),
            layout: &bgl,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: globals.as_entire_binding(),
            }],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("ferro layout"),
            bind_group_layouts: &[Some(&bgl)],
            immediate_size: 0,
        });
        let samples = 4;
        let attrs = wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Float32x4];
        let pipeline = |vs: &str,
                        fs: &str,
                        topology: wgpu::PrimitiveTopology,
                        label: &str,
                        bias: wgpu::DepthBiasState,
                        depth_write: bool| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some(vs),
                    compilation_options: Default::default(),
                    buffers: &[Some(wgpu::VertexBufferLayout {
                        array_stride: std::mem::size_of::<Vertex>() as u64,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &attrs,
                    })],
                },
                primitive: wgpu::PrimitiveState {
                    topology,
                    cull_mode: None,
                    ..Default::default()
                },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: DEPTH_FORMAT,
                    depth_write_enabled: Some(depth_write),
                    depth_compare: Some(wgpu::CompareFunction::LessEqual),
                    stencil: Default::default(),
                    bias,
                }),
                multisample: wgpu::MultisampleState {
                    count: samples,
                    ..Default::default()
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(fs),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: COLOR_FORMAT,
                        blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        Renderer {
            mesh_pipeline: pipeline(
                "vs_mesh",
                "fs_mesh",
                wgpu::PrimitiveTopology::TriangleList,
                "mesh",
                Default::default(),
                true,
            ),
            xray_pipeline: pipeline(
                "vs_mesh",
                "fs_mesh",
                wgpu::PrimitiveTopology::TriangleList,
                "mesh xray",
                Default::default(),
                false,
            ),
            line_pipeline: pipeline(
                "vs_line",
                "fs_line",
                wgpu::PrimitiveTopology::LineList,
                "lines",
                Default::default(),
                true,
            ),
            globals,
            bind_group,
            scene_version: None,
            scene_tris: None,
            scene_lines: None,
            targets: None,
            samples,
        }
    }

    fn ensure_targets(&mut self, device: &wgpu::Device, size: [u32; 2]) -> bool {
        let size = [size[0].clamp(1, 8192), size[1].clamp(1, 8192)];
        if self.targets.as_ref().is_some_and(|t| t.size == size) {
            return false;
        }
        let extent = wgpu::Extent3d {
            width: size[0],
            height: size[1],
            depth_or_array_layers: 1,
        };
        let tex = |label, format, samples, usage| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: extent,
                mip_level_count: 1,
                sample_count: samples,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage,
                view_formats: &[],
            })
        };
        let msaa = tex(
            "msaa",
            COLOR_FORMAT,
            self.samples,
            wgpu::TextureUsages::RENDER_ATTACHMENT,
        );
        let resolve = tex(
            "viewport",
            COLOR_FORMAT,
            1,
            wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
        );
        let depth = tex(
            "depth",
            DEPTH_FORMAT,
            self.samples,
            wgpu::TextureUsages::RENDER_ATTACHMENT,
        );
        self.targets = Some(Targets {
            size,
            msaa: msaa.create_view(&Default::default()),
            resolve_view: resolve.create_view(&Default::default()),
            _resolve: resolve,
            depth: depth.create_view(&Default::default()),
        });
        true
    }

    /// Render a frame. Returns `true` if the output texture was (re)created,
    /// so the caller must re-register it with egui.
    pub fn render(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        size: [u32; 2],
        f: &FrameData,
    ) -> bool {
        let recreated = self.ensure_targets(device, size);
        if self.scene_version != Some(f.scene_version) {
            self.scene_tris = upload(device, f.scene_tris, "scene tris");
            self.scene_lines = upload(device, f.scene_lines, "scene lines");
            self.scene_version = Some(f.scene_version);
        }
        let overlay_tris = upload(device, f.overlay_tris, "overlay tris");
        let overlay_lines = upload(device, f.overlay_lines, "overlay lines");
        let g = Globals {
            view_proj: f.view_proj.to_cols_array_2d(),
            light_dir: [0.35, -0.55, 0.75, 0.0],
            eye: [f.eye[0], f.eye[1], f.eye[2], 1.0],
        };
        queue.write_buffer(&self.globals, 0, bytemuck::bytes_of(&g));

        let t = self.targets.as_ref().expect("ensured");
        let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("viewport"),
        });
        {
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("viewport"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &t.msaa,
                    depth_slice: None,
                    resolve_target: Some(&t.resolve_view),
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: f.clear[0],
                            g: f.clear[1],
                            b: f.clear[2],
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Discard,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &t.depth,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Discard,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_bind_group(0, &self.bind_group, &[]);
            pass.set_pipeline(if f.xray {
                &self.xray_pipeline
            } else {
                &self.mesh_pipeline
            });
            if let Some((buf, n)) = &self.scene_tris {
                pass.set_vertex_buffer(0, buf.slice(..));
                pass.draw(0..*n, 0..1);
            }
            pass.set_pipeline(&self.mesh_pipeline);
            if let Some((buf, n)) = &overlay_tris {
                pass.set_vertex_buffer(0, buf.slice(..));
                pass.draw(0..*n, 0..1);
            }
            pass.set_pipeline(&self.line_pipeline);
            for (buf, n) in [&self.scene_lines, &overlay_lines].into_iter().flatten() {
                pass.set_vertex_buffer(0, buf.slice(..));
                pass.draw(0..*n, 0..1);
            }
        }
        queue.submit(Some(enc.finish()));
        recreated
    }

    pub fn output_view(&self) -> Option<&wgpu::TextureView> {
        self.targets.as_ref().map(|t| &t.resolve_view)
    }
}

fn upload(device: &wgpu::Device, v: &[Vertex], label: &str) -> Option<(wgpu::Buffer, u32)> {
    if v.is_empty() {
        return None;
    }
    let buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some(label),
        contents: bytemuck::cast_slice(v),
        usage: wgpu::BufferUsages::VERTEX,
    });
    Some((buf, v.len() as u32))
}
