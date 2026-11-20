use bevy::{
    prelude::*,
    render::{
        render_resource::DownlevelFlags,
        renderer::{RenderAdapter, RenderDevice},
    },
};
use strata_globe::PlanetSpec;

use crate::{camera::GlobeCameraState, terrain::TerrainElevation};

/// The current terrain revision together with the planet geometry used by the client.
#[derive(Clone, Copy, PartialEq)]
pub(crate) struct TerrainRenderKey {
    pub(crate) planet: PlanetSpec,
    pub(crate) revision: u64,
}

#[derive(Resource)]
pub(crate) struct TerrainBackend {
    allow_gpu: bool,
    gpu_active: bool,
    cpu_prepared: bool,
    evaluated: Option<TerrainRenderKey>,
}

impl TerrainBackend {
    pub(crate) const fn new(allow_gpu: bool) -> Self {
        Self {
            allow_gpu,
            gpu_active: false,
            cpu_prepared: false,
            evaluated: None,
        }
    }

    pub(crate) const fn uses_gpu(&self) -> bool {
        self.gpu_active
    }
    pub(crate) const fn uses_prepared_draw(&self) -> bool {
        self.gpu_active || self.cpu_prepared
    }
}

#[expect(
    clippy::needless_pass_by_value,
    reason = "Bevy run condition receives Res<T> by value"
)]
pub(crate) fn use_cpu_terrain(backend: Res<TerrainBackend>) -> bool {
    !backend.uses_prepared_draw()
}

#[expect(clippy::needless_pass_by_value, reason = "Bevy system parameters")]
pub(crate) fn choose_terrain_backend(
    mut backend: ResMut<TerrainBackend>,
    camera: Res<GlobeCameraState>,
    terrain: Res<TerrainElevation>,
    adapter: Res<RenderAdapter>,
    device: Res<RenderDevice>,
) {
    let key = TerrainRenderKey {
        planet: camera.planet,
        revision: terrain.revision(),
    };
    if backend.evaluated == Some(key) {
        return;
    }

    let report_fallback = backend.evaluated.is_none() || backend.uses_gpu();
    backend.evaluated = Some(key);
    let supported = supports_gpu(key.planet, &terrain, &adapter, &device);
    backend.gpu_active = backend.allow_gpu && supported;
    backend.cpu_prepared = !backend.allow_gpu
        && supported
        && terrain
            .world()
            .and_then(|world| world.surface_data())
            .is_some_and(|data| data.earth_water().is_some());
    if backend.allow_gpu && !backend.uses_gpu() && report_fallback {
        info!("Using CPU terrain for the current source and planet");
    }
}

fn supports_gpu(
    planet: PlanetSpec,
    terrain: &TerrainElevation,
    adapter: &RenderAdapter,
    device: &RenderDevice,
) -> bool {
    let limits = device.limits();
    let grid = planet.grid_segments();
    let flags = adapter.get_downlevel_capabilities().flags;
    let required = DownlevelFlags::COMPUTE_SHADERS | DownlevelFlags::INDIRECT_EXECUTION;
    if !flags.contains(required)
        || limits.max_storage_buffers_per_shader_stage < 5
        || limits.max_storage_buffer_binding_size < 12 * 1024 * 1024
        || !grid.is_power_of_two()
        || grid.ilog2() + u32::from(planet.max_lod()) > 24
        || 24 * u64::from(grid).pow(2) > limits.max_buffer_size
        || 6 * u64::from(grid).pow(2) > u64::from(u32::MAX)
    {
        return false;
    }
    terrain
        .world()
        .and_then(|world| world.surface_data())
        .is_some_and(|source| {
            let (width, height, raster) = source.raster();
            let side = match raster {
                strata_terrain::SurfaceRaster::Earth(_) => 5400,
                strata_terrain::SurfaceRaster::Dem(_) => width.max(height),
            };
            let resolution = u64::from(source.macro_grid().0);
            let bounds_bytes = 6 * 8 * (4 * resolution.pow(2) - 1) / 3;
            side <= limits.max_texture_dimension_2d as usize
                && bounds_bytes <= limits.max_storage_buffer_binding_size
        })
}
