use std::fmt::Write as _;

use strata_globe::{AuthoritativePoint, ecef_to_tile_uv};
use strata_simulation::{ClockStatus, SimulationSnapshot};
use strata_terrain::{Biome, TerrainWorld, WaterBodyKind, WaterSample};

use crate::camera::{GlobeCameraState, OriginState};
use crate::globe_render::GlobeRenderState;

#[expect(
    clippy::too_many_arguments,
    reason = "HUD formatter reads snapshot, camera, origin, render stats, and optional terrain"
)]
pub(crate) fn format_hud_into(
    out: &mut String,
    debug: bool,
    frame: &SimulationSnapshot,
    active: Option<&AuthoritativePoint>,
    cam_state: &GlobeCameraState,
    origin_state: &OriginState,
    render_state: Option<&GlobeRenderState>,
    terrain_world: Option<&TerrainWorld>,
    auto_sweep: bool,
) {
    let snapshot = frame.world();
    let time = frame.confirmed_time();
    let elapsed = time.elapsed();
    let status = match frame.clock_status() {
        ClockStatus::Running => "Running",
        ClockStatus::Pausing => "Pausing",
        ClockStatus::Paused => "Paused",
    };
    let _ = writeln!(
        out,
        "Game time {:02}:{:02}:{:02}.{:03} | {status} | Space Pause/Resume",
        elapsed.as_secs() / 3600,
        elapsed.as_secs() / 60 % 60,
        elapsed.as_secs() % 60,
        elapsed.subsec_millis(),
    );
    write_renderer(
        out,
        snapshot.planet.radius_meters(),
        time.ticks(),
        render_state,
    );
    let blend = cam_state.orbital_blend();
    let view_mode = if blend >= 0.999 {
        "Planet (Center)"
    } else if blend <= 0.001 {
        "Surface"
    } else {
        "Transition"
    };
    let _ = writeln!(
        out,
        "Altitude: {:.1} m | View: {view_mode} (pitch {:.1} deg) | Rebases: {} | Sweep: {} | Debug (G): {}",
        cam_state.altitude_meters,
        cam_state.effective_pitch_rad().to_degrees(),
        origin_state.rebase_count,
        if auto_sweep { "ON" } else { "OFF" },
        if debug { "ON" } else { "OFF" },
    );
    if let Some(world) = terrain_world {
        let sample = world.sample_terrain(cam_state.focus_unit);
        let water_desc = format_water_status(world, &sample.water);
        let _ = writeln!(
            out,
            "Terrain: Elev {:+.1} m | Biome: {} | Water: {} | Province: {}",
            sample.elevation_m,
            biome_label(sample.biome),
            water_desc,
            sample.province.label(),
        );
    }
    if let Some(pt) = active {
        let tile_info = ecef_to_tile_uv(
            snapshot.planet,
            pt.position,
            render_state.map_or(snapshot.planet.max_lod(), |state| state.max_lod),
        )
        .map_or_else(
            |_| String::from("n/a"),
            |(tile, _, _)| {
                format!(
                    "{} L{} ({}, {})",
                    tile.face.label(),
                    tile.lod,
                    tile.x,
                    tile.y
                )
            },
        );
        let rel = origin_state.origin.to_render_vec3(pt.position);
        let _ = writeln!(
            out,
            "Selected [{}]: {} | Tile {}",
            pt.id.0, pt.label, tile_info
        );
        let _ = writeln!(
            out,
            "  ECEF f64: ({:.3}, {:.3}, {:.3}) m",
            pt.position.x, pt.position.y, pt.position.z
        );
        let _ = writeln!(
            out,
            "  Render f32: ({:.3}, {:.3}, {:.3}) m",
            rel.x, rel.y, rel.z
        );
    }
    let _ = write!(
        out,
        "Controls: Wheel/+-/PgUpDn Zoom-to-Cursor | MMB/WASD/Arrows Pan | RMB/Alt+MMB/Q-E Rotate+Tilt | N/Home/Compass North | 1-7/Tab Point | P/L/R | G Debug"
    );
}

fn format_water_status(world: &TerrainWorld, water: &WaterSample) -> String {
    match water.kind {
        WaterBodyKind::Dry => String::from("Dry"),
        WaterBodyKind::Ocean => format!("Ocean (depth {:.1} m)", water.depth_m),
        WaterBodyKind::Lake(id) => {
            let name = world
                .lakes()
                .iter()
                .find(|l| l.id == id)
                .map_or("Lake", |l| l.name);
            format!("Lake: {name} (depth {:.1} m)", water.depth_m)
        }
        WaterBodyKind::River(id) => {
            let name = world
                .rivers()
                .iter()
                .find(|r| r.id == id)
                .map_or("River", |r| r.name);
            format!("River: {name} (depth {:.1} m)", water.depth_m)
        }
    }
}

const fn biome_label(biome: Biome) -> &'static str {
    match biome {
        Biome::PolarIce => "Polar Ice",
        Biome::Tundra => "Tundra",
        Biome::BorealForest => "Boreal Forest",
        Biome::TemperateForest => "Temperate Forest",
        Biome::TemperateGrassland => "Temperate Grassland",
        Biome::Steppe => "Steppe",
        Biome::HotDesert => "Hot Desert",
        Biome::ColdDesert => "Cold Desert",
        Biome::Savanna => "Savanna",
        Biome::TropicalRainforest => "Tropical Rainforest",
        Biome::AlpineTundra => "Alpine Tundra",
        Biome::AlpineRock => "Alpine Rock",
        Biome::CoastalWetland => "Coastal Wetland",
        Biome::OpenWater => "Open Water",
    }
}

fn write_renderer(
    out: &mut String,
    radius_m: f64,
    tick: u64,
    render_state: Option<&GlobeRenderState>,
) {
    let _ = write!(
        out,
        "Project Strata | Planet R = {:.0} km | Tick {} | ",
        radius_m / 1_000.0,
        tick
    );
    if let Some(render_state) = render_state {
        let _ = writeln!(
            out,
            "CPU tiles {} / Cached {} / Pending {} (LOD {}..{})",
            render_state.active_tile_count,
            render_state.cached_tile_count,
            render_state.pending_task_count,
            render_state.min_lod,
            render_state.max_lod
        );
    } else {
        let _ = writeln!(out, "GPU terrain");
    }
}
