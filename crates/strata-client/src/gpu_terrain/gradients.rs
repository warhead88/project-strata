use bevy::{
    prelude::default,
    render::{
        render_resource::{
            Buffer, BufferBindingType, BufferDescriptor, BufferUsages, CommandEncoderDescriptor,
            ComputePassDescriptor, PipelineLayoutDescriptor, RawComputePipelineDescriptor,
            ShaderStages,
        },
        renderer::{RenderDevice, RenderQueue},
    },
};

use super::{
    pipeline::{shader, storage_group},
    source::initialized,
};

const GRADIENTS: &str = include_str!("gradients.wgsl");
const WORKGROUP_SIZE: u32 = 64;

pub(super) struct GradientLattices {
    pub(super) buffer: Buffer,
    pub(super) second_octave_offset: u32,
}

pub(super) fn generate(
    device: &RenderDevice,
    queue: &RenderQueue,
    octaves: [(u32, u64); 2],
) -> GradientLattices {
    let [first_count, second_count] = octaves.map(|(frequency, _)| (2 * frequency + 1).pow(3));
    let second_octave_offset = first_count.div_ceil(8);
    let words = second_octave_offset + second_count.div_ceil(8);
    let parameters = std::array::from_fn::<_, 2, _>(|i| {
        let (frequency, seed) = octaves[i];
        let seed = seed.to_le_bytes();
        [
            u32::from_le_bytes([seed[0], seed[1], seed[2], seed[3]]),
            u32::from_le_bytes([seed[4], seed[5], seed[6], seed[7]]),
            frequency,
            frequency * 2 + 1,
            [first_count, second_count][i],
            [0, second_octave_offset][i],
            0,
            0,
        ]
    });
    let parameters = initialized(
        device,
        "Terrain octave seeds",
        BufferUsages::UNIFORM,
        &parameters,
    );
    let buffer = device.create_buffer(&BufferDescriptor {
        label: Some("Terrain noise gradients"),
        size: u64::from(words) * 4,
        usage: BufferUsages::STORAGE | BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let (layout, bindings) = storage_group(
        device,
        "Terrain gradient generation",
        ShaderStages::COMPUTE,
        [
            (&parameters, BufferBindingType::Uniform),
            (&buffer, BufferBindingType::Storage { read_only: false }),
        ],
    );
    let layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
        label: Some("Terrain gradient generation"),
        bind_group_layouts: &[Some(&layout)],
        immediate_size: 0,
    });
    let module = shader(device, "Terrain lattice generation WGSL", &[GRADIENTS]);
    let pipeline = device.create_compute_pipeline(&RawComputePipelineDescriptor {
        label: Some("Generate terrain gradients"),
        layout: Some(&layout),
        module: &module,
        entry_point: Some("generate_gradients"),
        compilation_options: default(),
        cache: None,
    });
    let mut encoder = device.create_command_encoder(&CommandEncoderDescriptor {
        label: Some("Generate terrain gradients"),
    });
    let groups = words.div_ceil(WORKGROUP_SIZE);
    let groups_x = groups.min(device.limits().max_compute_workgroups_per_dimension);
    {
        let mut pass = encoder.begin_compute_pass(&ComputePassDescriptor::default());
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &*bindings, &[]);
        pass.dispatch_workgroups(groups_x, groups.div_ceil(groups_x), 1);
    }
    // Enqueue before any draw can read this source. Queue ordering covers the dependency.
    queue.submit([encoder.finish()]);
    GradientLattices {
        buffer,
        second_octave_offset,
    }
}
