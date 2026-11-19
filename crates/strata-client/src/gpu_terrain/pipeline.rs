use bevy::core_pipeline::core_3d::CORE_3D_DEPTH_FORMAT;
use bevy::render::{
    render_resource::{
        BindGroup, BindGroupEntry, BindGroupLayout, BindGroupLayoutEntry, BindingResource,
        BindingType, Buffer, BufferBindingType, ColorTargetState, ColorWrites, CompareFunction,
        ComputePipeline, DepthBiasState, DepthStencilState, Face, MultisampleState,
        PipelineCompilationOptions, PipelineLayout, PipelineLayoutDescriptor, PrimitiveState,
        RawComputePipelineDescriptor, RawFragmentState, RawRenderPipelineDescriptor,
        RawVertexState, RenderPipeline, ShaderModule, ShaderModuleDescriptor, ShaderSource,
        ShaderStages, StencilState, TextureFormat, TextureSampleType, TextureViewDimension,
    },
    renderer::RenderDevice,
};

use super::source::SourceGpu;

pub(super) const CUBE: &str = include_str!("cube.wgsl");
const SELECT: &str = include_str!("select.wgsl");
pub(super) const SURFACE: &str = include_str!("surface.wgsl");
pub(super) const DRAW: &str = include_str!("draw.wgsl");

pub(super) struct SelectPipeline {
    pub(super) pipeline: ComputePipeline,
    pub(super) world: BindGroup,
    pub(super) output: BindGroup,
}

impl SelectPipeline {
    pub(super) fn new(
        device: &RenderDevice,
        frame: &Buffer,
        source: &SourceGpu,
        indirect: &Buffer,
        land: &Buffer,
        water: &Buffer,
    ) -> Self {
        let stage = ShaderStages::COMPUTE;
        let world = storage_group(
            device,
            "Terrain selection inputs",
            stage,
            [
                (frame, BufferBindingType::Uniform),
                (&source.bounds, storage(true)),
            ],
        );
        let output = storage_group(
            device,
            "Terrain selection outputs",
            stage,
            [
                (indirect, storage(false)),
                (land, storage(false)),
                (water, storage(false)),
                (&source.buckets, storage(true)),
            ],
        );
        let module = shader(device, "Terrain selection WGSL", &[CUBE, SELECT]);
        let layout = pipeline_layout(device, &world.0, &output.0);
        let pipeline = device.create_compute_pipeline(&RawComputePipelineDescriptor {
            label: Some("Terrain LOD, culling and compaction"),
            layout: Some(&layout),
            module: &module,
            entry_point: Some("select_tiles"),
            compilation_options: PipelineCompilationOptions::default(),
            cache: None,
        });
        Self {
            pipeline,
            world: world.1,
            output: output.1,
        }
    }
}

pub(super) struct DrawPipelines {
    shader: ShaderModule,
    layout: PipelineLayout,
    pub(super) world: BindGroup,
    pub(super) land: BindGroup,
    pub(super) water: BindGroup,
}

impl DrawPipelines {
    pub(super) fn new(
        device: &RenderDevice,
        frame: &Buffer,
        source: &SourceGpu,
        land: &Buffer,
        water: &Buffer,
    ) -> Self {
        let (world_layout, world) =
            surface_group(device, frame, source, ShaderStages::VERTEX_FRAGMENT);
        let tile_layout = device.create_bind_group_layout(
            "Visible terrain tiles",
            &[buffer_layout(0, ShaderStages::VERTEX, storage(true))],
        );
        let land = device.create_bind_group(
            "Visible land tiles",
            &tile_layout,
            &[entry(0, land.as_entire_binding())],
        );
        let water = device.create_bind_group(
            "Visible water tiles",
            &tile_layout,
            &[entry(0, water.as_entire_binding())],
        );
        let shader = shader(device, "Terrain surface WGSL", &[CUBE, SURFACE, DRAW]);
        let layout = pipeline_layout(device, &world_layout, &tile_layout);
        Self {
            shader,
            layout,
            world,
            land,
            water,
        }
    }

    pub(super) fn create(
        &self,
        device: &RenderDevice,
        format: TextureFormat,
        samples: u32,
        water: bool,
    ) -> RenderPipeline {
        let specialization = [("water_pass", f64::from(u32::from(water)))];
        let options = || PipelineCompilationOptions {
            constants: &specialization,
            ..Default::default()
        };
        let label = if water {
            "Terrain water"
        } else {
            "Terrain land"
        };
        device.create_render_pipeline(&RawRenderPipelineDescriptor {
            label: Some(label),
            layout: Some(&self.layout),
            vertex: RawVertexState {
                module: &self.shader,
                entry_point: Some("vertex"),
                compilation_options: options(),
                buffers: &[],
            },
            fragment: Some(RawFragmentState {
                module: &self.shader,
                entry_point: Some("fragment"),
                compilation_options: options(),
                targets: &[Some(ColorTargetState {
                    format,
                    blend: None,
                    write_mask: ColorWrites::ALL,
                })],
            }),
            primitive: PrimitiveState {
                cull_mode: Some(Face::Back),
                ..Default::default()
            },
            depth_stencil: Some(DepthStencilState {
                format: CORE_3D_DEPTH_FORMAT,
                depth_write_enabled: Some(true),
                depth_compare: Some(CompareFunction::GreaterEqual),
                stencil: StencilState::default(),
                bias: DepthBiasState::default(),
            }),
            multisample: MultisampleState {
                count: samples,
                ..Default::default()
            },
            multiview_mask: None,
            cache: None,
        })
    }
}

pub(super) fn surface_group(
    device: &RenderDevice,
    frame: &Buffer,
    source: &SourceGpu,
    stage: ShaderStages,
) -> (BindGroupLayout, BindGroup) {
    let bindings = [
        buffer_layout(0, stage, BufferBindingType::Uniform),
        texture_layout(1, stage, TextureViewDimension::D2),
        texture_layout(2, stage, TextureViewDimension::D2Array),
        buffer_layout(3, stage, storage(true)),
        buffer_layout(4, stage, storage(true)),
        buffer_layout(5, stage, storage(true)),
    ];
    let world_layout = device.create_bind_group_layout("Terrain draw inputs", &bindings);
    let world = device.create_bind_group(
        "Terrain draw inputs",
        &world_layout,
        &[
            entry(0, frame.as_entire_binding()),
            entry(1, BindingResource::TextureView(&source.dem)),
            entry(2, BindingResource::TextureView(&source.climate)),
            entry(3, source.gradients.as_entire_binding()),
            entry(4, source.buckets.as_entire_binding()),
            entry(5, source.features.as_entire_binding()),
        ],
    );
    (world_layout, world)
}

pub(super) fn shader(device: &RenderDevice, label: &str, parts: &[&str]) -> ShaderModule {
    device.create_and_validate_shader_module(ShaderModuleDescriptor {
        label: Some(label),
        source: ShaderSource::Wgsl(parts.join("\n").into()),
    })
}

pub(super) fn pipeline_layout(
    device: &RenderDevice,
    first: &BindGroupLayout,
    second: &BindGroupLayout,
) -> PipelineLayout {
    device.create_pipeline_layout(&PipelineLayoutDescriptor {
        label: Some("Terrain layout"),
        bind_group_layouts: &[Some(first), Some(second)],
        immediate_size: 0,
    })
}

pub(super) fn storage_group<const N: usize>(
    device: &RenderDevice,
    label: &str,
    stage: ShaderStages,
    buffers: [(&Buffer, BufferBindingType); N],
) -> (BindGroupLayout, BindGroup) {
    let indices: [u32; N] = std::array::from_fn(|i| u32::try_from(i).unwrap_or(0));
    let layouts = indices.map(|i| buffer_layout(i, stage, buffers[i as usize].1));
    let entries = indices.map(|i| entry(i, buffers[i as usize].0.as_entire_binding()));
    let layout = device.create_bind_group_layout(label, &layouts);
    let group = device.create_bind_group(label, &layout, &entries);
    (layout, group)
}

fn entry(binding: u32, resource: BindingResource<'_>) -> BindGroupEntry<'_> {
    BindGroupEntry { binding, resource }
}

fn buffer_layout(
    binding: u32,
    visibility: ShaderStages,
    ty: BufferBindingType,
) -> BindGroupLayoutEntry {
    BindGroupLayoutEntry {
        binding,
        visibility,
        ty: BindingType::Buffer {
            ty,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

fn texture_layout(
    binding: u32,
    visibility: ShaderStages,
    view_dimension: TextureViewDimension,
) -> BindGroupLayoutEntry {
    BindGroupLayoutEntry {
        binding,
        visibility,
        ty: BindingType::Texture {
            view_dimension,
            multisampled: false,
            sample_type: TextureSampleType::Float { filterable: false },
        },
        count: None,
    }
}

fn storage(read_only: bool) -> BufferBindingType {
    BufferBindingType::Storage { read_only }
}
