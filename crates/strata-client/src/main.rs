mod camera;
mod camera_controls;
mod camera_sync;
mod clock;
mod compass;
mod globe_debug;
mod globe_materials;
mod globe_render;
mod gpu_terrain;
mod hud;
mod markers;
mod snapshot;
mod terrain;
mod terrain_backend;
mod terrain_flight;
mod tile_async_queue;
mod tile_cache;
mod tile_mesh_build;
mod tile_mesh_colors;
mod tile_streaming;

#[cfg(test)]
mod terrain_render_tests;
#[cfg(test)]
mod terrain_streaming_tests;

use bevy::log::{Level, LogPlugin};
use bevy::prelude::*;
use strata_globe::GlobeSnapshot;

use crate::camera::{GlobeCameraState, OriginState};
use crate::camera_controls::{PointerGestureState, handle_camera_input};
use crate::camera_sync::{setup_camera_and_lighting, update_camera_and_origin};
use crate::clock::{handle_pause_input, start_clock};
use crate::compass::{setup_compass_ui, update_compass_ui};
use crate::globe_debug::{TerrainDebug, draw_cube_face_seams};
use crate::gpu_terrain::TerrainRenderPlugin;
use crate::markers::{setup_markers_and_hud, sync_markers_and_hud};
use crate::snapshot::{ClientWorldView, SelectedPointState, latch_world_snapshot};
use crate::terrain::TerrainElevation;
use crate::terrain_backend::{TerrainBackend, use_cpu_terrain};

fn main() -> AppExit {
    let initial_snapshot = GlobeSnapshot::reference_earth();
    let planet = initial_snapshot.planet;
    let (clock_control, reader) = match start_clock(initial_snapshot) {
        Ok(clock) => clock,
        Err(error) => {
            eprintln!("could not start the simulation clock: {error}");
            return AppExit::error();
        }
    };
    let world_view = ClientWorldView::new(reader);
    let selected = SelectedPointState::default();
    let terrain = TerrainElevation::earth_default();

    let focus_dir = selected
        .active_point(world_view.snapshot())
        .map_or(strata_globe::EcefMeters::from_raw(1.0, 1.0, 1.0), |p| {
            p.position
        });
    let Ok(mut cam_state) = GlobeCameraState::new(planet, focus_dir) else {
        return AppExit::error();
    };
    cam_state.sync_terrain_elevations(terrain.sampler());
    let (initial_eye, _, _) = cam_state.compute_pose_f64();
    let Ok(origin_state) = OriginState::new(initial_eye) else {
        return AppExit::error();
    };

    let gpu_terrain = !std::env::args().any(|arg| arg == "--cpu-terrain");
    let terrain_flight = std::env::args().any(|arg| arg == "--terrain-flight");
    let mut app = App::new();
    app.add_plugins(
        DefaultPlugins
            .set(WindowPlugin {
                primary_window: Some(if terrain_flight {
                    terrain_flight::window()
                } else {
                    Window {
                        title: "Project Strata — Globe".into(),
                        ..default()
                    }
                }),
                ..default()
            })
            .set(LogPlugin {
                level: Level::ERROR,
                filter: "strata_client=info".into(),
                ..default()
            }),
    )
    .add_plugins(TerrainRenderPlugin)
    .insert_resource(TerrainBackend::new(gpu_terrain))
    .insert_resource(ClearColor(Color::srgb(0.02, 0.03, 0.05)))
    .insert_resource(world_view)
    .insert_resource(clock_control)
    .insert_resource(selected)
    .insert_resource(cam_state)
    .insert_resource(origin_state)
    .insert_resource(terrain)
    .init_resource::<PointerGestureState>()
    .init_resource::<TerrainDebug>()
    .add_systems(
        Startup,
        (
            setup_camera_and_lighting,
            setup_markers_and_hud,
            setup_compass_ui,
        ),
    )
    .add_systems(
        Update,
        (
            latch_world_snapshot,
            handle_pause_input,
            handle_camera_input,
            update_compass_ui,
            update_camera_and_origin,
            draw_cube_face_seams.run_if(use_cpu_terrain),
            sync_markers_and_hud,
        )
            .chain(),
    );
    if terrain_flight {
        println!(
            "terrain_flight renderer={} resolution=1280x720",
            if gpu_terrain { "gpu" } else { "cpu" }
        );
        terrain_flight::install(&mut app);
    }
    app.run()
}
