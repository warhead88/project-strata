use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Instant,
};

use bevy::{
    core_pipeline::schedule::camera_driver,
    prelude::*,
    render::{
        RenderApp, RenderStartup,
        renderer::{
            PendingCommandBuffers, RenderDevice, RenderGraph, RenderGraphSystems, RenderQueue,
        },
    },
};
use wgpu::{
    Buffer, BufferDescriptor, BufferUsages, CommandEncoder, CommandEncoderDescriptor,
    ComputePassDescriptor, ComputePassTimestampWrites, ComputePipeline, ComputePipelineDescriptor,
    Features, MapMode, QuerySet, QuerySetDescriptor, QueryType, ShaderModuleDescriptor,
    ShaderSource,
};

#[derive(Default, Clone, Resource)]
pub(super) struct GpuTimings(Arc<Mutex<Vec<(Instant, f64)>>>);

impl GpuTimings {
    pub(super) fn take(&self) -> Vec<(Instant, f64)> {
        self.0
            .lock()
            .map_or_else(|_| Vec::new(), |mut samples| std::mem::take(&mut *samples))
    }
}

pub(super) fn install(app: &mut App) {
    let samples = GpuTimings::default();
    app.insert_resource(samples.clone());
    app.sub_app_mut(RenderApp)
        .insert_resource(samples)
        .add_systems(RenderStartup, initialize)
        .add_systems(
            RenderGraph,
            (
                begin
                    .before(camera_driver)
                    .in_set(RenderGraphSystems::Render),
                end.after(camera_driver).in_set(RenderGraphSystems::Render),
                read.in_set(RenderGraphSystems::Finish),
            ),
        );
}

const READBACK_BYTES: u64 = 16;

struct Slot {
    buffer: Buffer,
    busy: Arc<AtomicBool>,
}

#[derive(Resource)]
struct Timer {
    queries: QuerySet,
    marker: ComputePipeline,
    resolve: Buffer,
    slots: [Slot; 8],
    current: Option<usize>,
    started: Instant,
    ms_per_tick: f64,
}

#[expect(
    clippy::needless_pass_by_value,
    reason = "Bevy render startup system parameters"
)]
fn initialize(mut commands: Commands, device: Res<RenderDevice>, queue: Res<RenderQueue>) {
    if !device.features().contains(Features::TIMESTAMP_QUERY) {
        println!("terrain_flight GPU timestamp queries unavailable on this adapter");
        return;
    }
    let gpu = device.wgpu_device();
    let buffer = |label, usage| {
        gpu.create_buffer(&BufferDescriptor {
            label: Some(label),
            size: READBACK_BYTES,
            usage,
            mapped_at_creation: false,
        })
    };
    let marker_shader = gpu.create_shader_module(ShaderModuleDescriptor {
        label: Some("Terrain flight timestamp marker"),
        source: ShaderSource::Wgsl("@compute @workgroup_size(1) fn tick() {}".into()),
    });
    let marker = gpu.create_compute_pipeline(&ComputePipelineDescriptor {
        label: Some("Terrain flight timestamp marker"),
        layout: None,
        module: &marker_shader,
        entry_point: Some("tick"),
        compilation_options: wgpu::PipelineCompilationOptions::default(),
        cache: None,
    });
    commands.insert_resource(Timer {
        marker,
        queries: gpu.create_query_set(&QuerySetDescriptor {
            label: Some("Terrain flight camera timings"),
            ty: QueryType::Timestamp,
            count: 2,
        }),
        resolve: buffer(
            "Terrain flight timestamp resolve",
            BufferUsages::QUERY_RESOLVE | BufferUsages::COPY_SRC,
        ),
        slots: std::array::from_fn(|_| Slot {
            buffer: buffer(
                "Terrain flight timestamp readback",
                BufferUsages::COPY_DST | BufferUsages::MAP_READ,
            ),
            busy: Arc::new(AtomicBool::new(false)),
        }),
        current: None,
        started: Instant::now(),
        ms_per_tick: f64::from(queue.get_timestamp_period()) / 1e6,
    });
}

// Metal can timestamp pass boundaries even when it cannot timestamp Bevy's individual commands
// inside those passes. These two markers enclose the same camera work in both renderer modes.
fn marker(encoder: &mut CommandEncoder, timer: &Timer, index: u32) {
    let mut pass = encoder.begin_compute_pass(&ComputePassDescriptor {
        label: Some("Terrain flight timestamp marker"),
        timestamp_writes: Some(ComputePassTimestampWrites {
            query_set: &timer.queries,
            beginning_of_pass_write_index: Some(index),
            end_of_pass_write_index: None,
        }),
    });
    pass.set_pipeline(&timer.marker);
    pass.dispatch_workgroups(1, 1, 1);
}

#[expect(
    clippy::needless_pass_by_value,
    reason = "Bevy render system parameters"
)]
fn begin(
    timer: Option<ResMut<Timer>>,
    device: Res<RenderDevice>,
    mut pending: ResMut<PendingCommandBuffers>,
) {
    let Some(mut timer) = timer else { return };
    timer.current = timer
        .slots
        .iter()
        .position(|slot| !slot.busy.load(Ordering::Acquire));
    if timer.current.is_some() {
        timer.started = Instant::now();
        let mut encoder = device.create_command_encoder(&CommandEncoderDescriptor::default());
        marker(&mut encoder, &timer, 0);
        pending.push_encoder(encoder, "terrain_flight_gpu_start");
    }
}

#[expect(
    clippy::needless_pass_by_value,
    reason = "Bevy render system parameters"
)]
fn end(
    timer: Option<Res<Timer>>,
    device: Res<RenderDevice>,
    mut pending: ResMut<PendingCommandBuffers>,
) {
    let Some(timer) = timer else { return };
    let Some(slot) = timer.current else { return };
    let mut encoder = device.create_command_encoder(&CommandEncoderDescriptor::default());
    marker(&mut encoder, &timer, 1);
    encoder.resolve_query_set(&timer.queries, 0..2, &timer.resolve, 0);
    encoder.copy_buffer_to_buffer(
        &timer.resolve,
        0,
        &timer.slots[slot].buffer,
        0,
        READBACK_BYTES,
    );
    pending.push_encoder(encoder, "terrain_flight_gpu_end");
}

#[expect(
    clippy::needless_pass_by_value,
    reason = "Bevy render system parameters"
)]
fn read(timer: Option<Res<Timer>>, samples: Res<GpuTimings>) {
    let Some(timer) = timer else { return };
    let Some(index) = timer.current else { return };
    let slot = &timer.slots[index];
    slot.busy.store(true, Ordering::Release);
    let output = samples.0.clone();
    let buffer = slot.buffer.clone();
    let busy = slot.busy.clone();
    let started = timer.started;
    let period = timer.ms_per_tick;
    slot.buffer.slice(..).map_async(MapMode::Read, move |result| {
        if result.is_ok() {
            if let Ok(data) = buffer.slice(..).get_mapped_range() {
                let [start, end] = *bytemuck::from_bytes::<[u64; 2]>(&data);
                if start > 0 && end > start && end != u64::MAX
                    && let Ok(mut output) = output.lock() {
                    #[expect(clippy::cast_precision_loss, reason = "subtracted frame durations are far below f64 integer precision")]
                    output.push((started, (end - start) as f64 * period));
                }
            }
            buffer.unmap();
        }
        busy.store(false, Ordering::Release);
    });
}
