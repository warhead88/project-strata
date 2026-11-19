mod pipeline;
mod source;
#[cfg(test)]
mod tests;
mod view;

use std::collections::HashMap;

use bevy::core_pipeline::{Core3dSystems, core_3d::main_opaque_pass_3d, schedule::Core3d};
use bevy::prelude::*;
use bevy::render::{
    Extract, ExtractSchedule, Render, RenderApp, RenderSystems,
    camera::ExtractedCamera,
    diagnostic::RecordDiagnostics,
    render_resource::{
        Buffer, BufferDescriptor, BufferUsages, CommandEncoder, ComputePassDescriptor, IndexFormat,
        RenderPassDescriptor, RenderPipeline, StoreOp, TextureFormat,
    },
    renderer::{RenderAdapter, RenderContext, RenderDevice, RenderQueue, ViewQuery},
    view::{ExtractedView, Msaa, ViewDepthStencilTexture, ViewTarget},
};
use bytemuck::bytes_of;
use strata_globe::PlanetSpec;

use crate::{camera::GlobeCameraState, globe_debug::TerrainDebug, terrain::TerrainElevation};

use self::pipeline::{DrawPipelines, SelectPipeline};
use self::source::{SourceGpu, SourceSettings, grid_indices, initialized};
use self::view::GpuView;

#[derive(Resource)]
pub(crate) struct GpuTerrainEnabled(pub(crate) bool);

#[expect(
    clippy::needless_pass_by_value,
    reason = "Bevy run condition receives Res<T> by value"
)]
pub(crate) fn use_cpu_terrain(enabled: Res<GpuTerrainEnabled>) -> bool {
    !enabled.0
}

#[expect(
    clippy::needless_pass_by_value,
    reason = "Bevy startup system parameters"
)]
pub(crate) fn choose_terrain_backend(
    mut enabled: ResMut<GpuTerrainEnabled>,
    camera: Res<GlobeCameraState>,
    terrain: Res<TerrainElevation>,
    adapter: Res<RenderAdapter>,
    device: Res<RenderDevice>,
) {
    if !enabled.0 {
        return;
    }
    let limits = device.limits();
    let grid = camera.planet.grid_segments();
    let source_fits = terrain
        .world()
        .and_then(|world| world.surface_data())
        .is_some_and(|source| {
            let (width, height, _) = source.raster();
            width.max(height) <= limits.max_texture_dimension_2d as usize
        });
    let flags = adapter.get_downlevel_capabilities().flags;
    let dispatch = wgpu::DownlevelFlags::COMPUTE_SHADERS | wgpu::DownlevelFlags::INDIRECT_EXECUTION;
    enabled.0 = source_fits
        && flags.contains(dispatch)
        && limits.max_storage_buffers_per_shader_stage >= 5
        && limits.max_storage_buffer_binding_size >= 12 * 1024 * 1024
        && grid.is_power_of_two()
        && grid > 0
        && grid.ilog2() + u32::from(camera.planet.max_lod()) <= 24;
    if !enabled.0 {
        warn!("Terrain GPU path unavailable for this terrain source or adapter; using CPU terrain");
    }
}

pub(crate) struct GpuTerrainPlugin;

impl Plugin for GpuTerrainPlugin {
    fn build(&self, app: &mut App) {
        app.sub_app_mut(RenderApp)
            .add_systems(ExtractSchedule, extract_terrain)
            .add_systems(
                Render,
                prepare_terrain.in_set(RenderSystems::PrepareResources),
            )
            .add_systems(
                Core3d,
                render_terrain
                    .in_set(Core3dSystems::MainPass)
                    .after(main_opaque_pass_3d)
                    .before(bevy::core_pipeline::core_3d::main_transparent_pass_3d),
            );
    }
}

#[derive(Resource)]
struct ExtractedTerrain {
    camera: GlobeCameraState,
    debug: bool,
    terrain: TerrainElevation,
}

#[expect(
    clippy::needless_pass_by_value,
    reason = "Bevy extraction system parameters"
)]
fn extract_terrain(
    mut commands: Commands,
    enabled: Extract<Res<GpuTerrainEnabled>>,
    camera: Extract<Res<GlobeCameraState>>,
    debug: Extract<Res<TerrainDebug>>,
    terrain: Extract<Res<TerrainElevation>>,
) {
    if enabled.0 {
        commands.insert_resource(ExtractedTerrain {
            camera: (**camera).clone(),
            debug: debug.0,
            terrain: (**terrain).clone(),
        });
    }
}

// Conservative bound from the WGSL enumeration window. Through LOD 5 each dimension is at most 32;
// at finer levels asin(3*quarter_turn*sqrt(3)/parent_dim) gives a <30-child window.
const TILE_CAPACITY: u64 = 6 * 23 * 32 * 32;
const DRAW_BYTES: u64 = 20;

#[derive(Resource)]
struct TerrainGpu {
    source: SourceSettings,
    frame: Buffer,
    indirect: Buffer,
    indices: Buffer,
    #[cfg(test)]
    land: Buffer,
    reset: Buffer,
    selection: SelectPipeline,
    drawing: DrawPipelines,
    key: (PlanetSpec, u64),
}

impl TerrainGpu {
    fn new(device: &RenderDevice, queue: &RenderQueue, input: &ExtractedTerrain) -> Option<Self> {
        let source = SourceGpu::upload(device, queue, &input.terrain.world()?.surface_data()?)?;
        let frame = allocate(
            device,
            "Terrain camera",
            size_of::<GpuView>() as u64,
            BufferUsages::UNIFORM | BufferUsages::COPY_DST,
        );
        let indirect = allocate(
            device,
            "Terrain indirect arguments",
            2 * DRAW_BYTES,
            BufferUsages::INDIRECT
                | BufferUsages::STORAGE
                | BufferUsages::COPY_DST
                | BufferUsages::COPY_SRC,
        );
        let land = allocate(
            device,
            "Terrain land instance list",
            TILE_CAPACITY * 16,
            BufferUsages::STORAGE | BufferUsages::COPY_SRC,
        );
        let water = allocate(
            device,
            "Terrain water instance list",
            TILE_CAPACITY * 16,
            BufferUsages::STORAGE | BufferUsages::COPY_SRC,
        );
        let index_data = grid_indices(input.camera.planet.grid_segments());
        let index_count = u32::try_from(index_data.len()).ok()?;
        let indices = initialized(
            device,
            "Terrain reusable index grid",
            BufferUsages::INDEX,
            &index_data,
        );
        let reset = initialized(
            device,
            "Terrain initial indirect arguments",
            BufferUsages::COPY_SRC,
            &[[index_count, 0, 0, 0, 0]; 2],
        );
        let selection = SelectPipeline::new(device, &frame, &source, &indirect, &land, &water);
        let drawing = DrawPipelines::new(device, &frame, &source, &land, &water);
        Some(Self {
            source: source.settings,
            frame,
            indirect,
            indices,
            #[cfg(test)]
            land,
            reset,
            selection,
            drawing,
            key: (input.camera.planet, input.terrain.revision()),
        })
    }
    fn encode_selection(
        &self,
        encoder: &mut CommandEncoder,
        max_lod: u32,
        diagnostics: Option<&bevy::render::diagnostic::DiagnosticsRecorder>,
    ) {
        encoder.copy_buffer_to_buffer(&self.reset, 0, &self.indirect, 0, 2 * DRAW_BYTES);
        let mut pass = encoder.begin_compute_pass(&ComputePassDescriptor {
            label: Some("Terrain selection"),
            timestamp_writes: None,
        });
        let span = diagnostics.pass_span(&mut pass, "terrain_select");
        pass.set_pipeline(&self.selection.pipeline);
        pass.set_bind_group(0, &*self.selection.world, &[]);
        pass.set_bind_group(1, &*self.selection.output, &[]);
        pass.dispatch_workgroups(6, max_lod + 1, 1);
        span.end(&mut pass);
    }
}

fn allocate(device: &RenderDevice, label: &str, size: u64, usage: BufferUsages) -> Buffer {
    device.create_buffer(&BufferDescriptor {
        label: Some(label),
        size,
        usage,
        mapped_at_creation: false,
    })
}

#[expect(
    clippy::needless_pass_by_value,
    reason = "Bevy render resources are system parameters"
)]
fn prepare_terrain(
    mut commands: Commands,
    input: Option<Res<ExtractedTerrain>>,
    gpu: Option<Res<TerrainGpu>>,
    device: Res<RenderDevice>,
    queue: Res<RenderQueue>,
) {
    let Some(input) = input else { return };
    let key = (input.camera.planet, input.terrain.revision());
    if gpu.as_ref().is_some_and(|gpu| gpu.key == key) {
        return;
    }
    if let Some(gpu) = TerrainGpu::new(&device, &queue, &input) {
        commands.insert_resource(gpu);
    } else {
        error!("Could not upload GPU terrain source");
    }
}

#[expect(
    clippy::needless_pass_by_value,
    reason = "Bevy render resources are system parameters"
)]
fn render_terrain(
    input: Option<Res<ExtractedTerrain>>,
    gpu: Option<Res<TerrainGpu>>,
    queue: Res<RenderQueue>,
    view: ViewQuery<(
        &ExtractedCamera,
        &ExtractedView,
        &ViewTarget,
        &ViewDepthStencilTexture,
        &Msaa,
    )>,
    mut context: RenderContext,
    mut pipelines: Local<HashMap<(TextureFormat, u32), [RenderPipeline; 2]>>,
) {
    let (Some(input), Some(gpu)) = (input, gpu) else {
        return;
    };
    let (camera, view, target, depth, msaa) = view.into_inner();
    let (format, samples) = (target.main_texture_format(), msaa.samples());
    let pipelines = pipelines.entry((format, samples)).or_insert_with(|| {
        let device = context.render_device();
        [false, true].map(|water| gpu.drawing.create(device, format, samples, water))
    });
    let frame = input.view(camera, view, gpu.source);
    queue.write_buffer(&gpu.frame, 0, bytes_of(&frame));
    let diagnostics = context.diagnostic_recorder();
    let diagnostics = diagnostics.as_deref();
    gpu.encode_selection(context.command_encoder(), frame.settings[0], diagnostics);
    let color = [Some(target.get_color_attachment())];
    let mut pass = context.begin_tracked_render_pass(RenderPassDescriptor {
        label: Some("Terrain indirect drawing"),
        color_attachments: &color,
        depth_stencil_attachment: Some(depth.get_attachment(StoreOp::Store)),
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    });
    let span = diagnostics.pass_span(&mut pass, "terrain_draw");
    if let Some(viewport) = &camera.viewport {
        pass.set_camera_viewport(viewport);
    }
    pass.set_index_buffer(gpu.indices.slice(..), IndexFormat::Uint32);
    pass.set_bind_group(0, &gpu.drawing.world, &[]);
    pass.set_render_pipeline(&pipelines[0]);
    pass.set_bind_group(1, &gpu.drawing.land, &[]);
    pass.draw_indexed_indirect(&gpu.indirect, 0);
    if !input.debug {
        pass.set_render_pipeline(&pipelines[1]);
        pass.set_bind_group(1, &gpu.drawing.water, &[]);
        pass.draw_indexed_indirect(&gpu.indirect, DRAW_BYTES);
    }
    span.end(&mut pass);
}
