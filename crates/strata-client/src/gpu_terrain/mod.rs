mod cpu_selection;
mod earth_textures;
mod gradients;
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
    renderer::{RenderContext, RenderDevice, RenderQueue, ViewQuery},
    view::{ExtractedView, Msaa, ViewDepthStencilTexture, ViewTarget},
};
use bytemuck::{bytes_of, cast_slice};
use strata_globe::PlanetSpec;

use crate::{
    camera::GlobeCameraState,
    camera_sync::update_camera_and_origin,
    globe_debug::{TerrainDebug, draw_cube_face_seams},
    globe_render::{reconcile_cpu_terrain, sync_globe_tiles},
    terrain::TerrainElevation,
    terrain_backend::{TerrainBackend, TerrainRenderKey, choose_terrain_backend, use_cpu_terrain},
};

use self::pipeline::{DrawPipelines, SelectPipeline};
use self::source::{SourceGpu, grid_indices, initialized};
use self::view::GpuView;

pub(crate) struct TerrainRenderPlugin;

impl Plugin for TerrainRenderPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            (
                choose_terrain_backend,
                reconcile_cpu_terrain.run_if(resource_changed::<TerrainBackend>),
                sync_globe_tiles.run_if(use_cpu_terrain),
            )
                .chain()
                .after(update_camera_and_origin)
                .before(draw_cube_face_seams),
        );
        app.sub_app_mut(RenderApp)
            .init_resource::<PreparedTerrain>()
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
struct ExtractedTerrainFrame {
    revision: u64,
    cpu_lod: bool,
    gpu: Option<ExtractedTerrain>,
}

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
    backend: Extract<Res<TerrainBackend>>,
    camera: Extract<Res<GlobeCameraState>>,
    debug: Extract<Res<TerrainDebug>>,
    terrain: Extract<Res<TerrainElevation>>,
) {
    commands.insert_resource(ExtractedTerrainFrame {
        revision: terrain.revision(),
        cpu_lod: !backend.uses_gpu(),
        gpu: backend.uses_prepared_draw().then(|| ExtractedTerrain {
            camera: (**camera).clone(),
            debug: debug.0,
            terrain: (**terrain).clone(),
        }),
    });
}

// Conservative bound from the WGSL enumeration window. Through LOD 5 each dimension is at most 32;
// at finer levels asin(3*quarter_turn*sqrt(3)/parent_dim) gives a <30-child window.
const TILE_CAPACITY: u64 = 6 * 23 * 32 * 32;
const DRAW_BYTES: u64 = 20;

/// Cached raster data stays resident across geometry-only changes. Only an
/// active GPU view owns draw resources. A different terrain revision retires both.
#[derive(Default, Resource)]
struct PreparedTerrain {
    requested: Option<TerrainRenderKey>,
    resident: Option<(u64, SourceGpu)>,
    active: Option<TerrainGpu>,
}

struct TerrainGpu {
    source: SourceGpu,
    frame: Buffer,
    indirect: Buffer,
    indices: Buffer,
    land: Buffer,
    water: Buffer,
    index_count: u32,
    cpu_tiles: cpu_selection::CpuSelection,
    reset: Buffer,
    selection: SelectPipeline,
    drawing: DrawPipelines,
    pipelines: HashMap<(TextureFormat, u32), [RenderPipeline; 2]>,
}

impl TerrainGpu {
    fn new(device: &RenderDevice, planet: PlanetSpec, source: SourceGpu) -> Option<Self> {
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
            BufferUsages::STORAGE | BufferUsages::COPY_SRC | BufferUsages::COPY_DST,
        );
        let water = allocate(
            device,
            "Terrain water instance list",
            TILE_CAPACITY * 16,
            BufferUsages::STORAGE | BufferUsages::COPY_SRC | BufferUsages::COPY_DST,
        );
        let index_data = grid_indices(planet.grid_segments());
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
            source,
            frame,
            indirect,
            indices,
            land,
            water,
            index_count,
            cpu_tiles: cpu_selection::CpuSelection::default(),
            reset,
            selection,
            drawing,
            pipelines: HashMap::new(),
        })
    }
    fn upload_cpu_selection(
        &mut self,
        input: &ExtractedTerrain,
        frame: &GpuView,
        queue: &RenderQueue,
    ) {
        self.cpu_tiles.update(input, frame);
        let tiles = &self.cpu_tiles.instances;
        let count = u32::try_from(tiles.len()).unwrap_or(0);
        if count > 0 {
            let data = cast_slice(tiles);
            queue.write_buffer(&self.land, 0, data);
            queue.write_buffer(&self.water, 0, data);
        }
        let indirect = [[self.index_count, count, 0, 0, 0]; 2];
        queue.write_buffer(&self.indirect, 0, bytes_of(&indirect));
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
    frame: Option<Res<ExtractedTerrainFrame>>,
    mut prepared: ResMut<PreparedTerrain>,
    device: Res<RenderDevice>,
    queue: Res<RenderQueue>,
) {
    let Some(frame) = frame else { return };
    let requested = frame.gpu.as_ref().map(|input| TerrainRenderKey {
        planet: input.camera.planet,
        revision: frame.revision,
    });
    if prepared
        .resident
        .as_ref()
        .is_some_and(|(rev, _)| *rev != frame.revision)
    {
        prepared.resident = None;
    }
    if prepared.requested == requested {
        return;
    }

    prepared.active = None;
    prepared.requested = requested;
    if let (Some(key), Some(input)) = (requested, &frame.gpu) {
        if prepared.resident.is_none()
            && let Some(data) = input.terrain.world().and_then(|world| world.surface_data())
        {
            prepared.resident =
                SourceGpu::upload(&device, &queue, &data).map(|source| (key.revision, source));
        }
        prepared.active = prepared
            .resident
            .as_ref()
            .and_then(|(_, source)| TerrainGpu::new(&device, key.planet, source.clone()));
        if prepared.active.is_none() {
            error!(
                "Could not prepare GPU terrain for revision {}",
                key.revision
            );
        }
    }
}

#[expect(
    clippy::needless_pass_by_value,
    reason = "Bevy render resources are system parameters"
)]
fn render_terrain(
    frame: Option<Res<ExtractedTerrainFrame>>,
    mut prepared: ResMut<PreparedTerrain>,
    queue: Res<RenderQueue>,
    view: ViewQuery<(
        &ExtractedCamera,
        &ExtractedView,
        &ViewTarget,
        &ViewDepthStencilTexture,
        &Msaa,
    )>,
    mut context: RenderContext,
) {
    let Some(extracted) = frame else { return };
    let Some(input) = &extracted.gpu else { return };
    let Some(gpu) = prepared.active.as_mut() else {
        return;
    };
    let (camera, view, target, depth, msaa) = view.into_inner();
    let (format, samples) = (target.main_texture_format(), msaa.samples());
    let frame = input.view(camera, view, gpu.source.settings);
    queue.write_buffer(&gpu.frame, 0, bytes_of(&frame));
    let diagnostics = context.diagnostic_recorder();
    let diagnostics = diagnostics.as_deref();
    if extracted.cpu_lod {
        gpu.upload_cpu_selection(input, &frame, &queue);
    } else {
        gpu.encode_selection(context.command_encoder(), frame.settings[0], diagnostics);
    }
    let pipelines = gpu.pipelines.entry((format, samples)).or_insert_with(|| {
        let device = context.render_device();
        [false, true].map(|water| gpu.drawing.create(device, format, samples, water))
    });
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
