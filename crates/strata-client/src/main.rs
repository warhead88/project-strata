mod camera;
mod camera_controls;
mod camera_sync;
mod compass;
mod globe_render;
mod markers;
mod snapshot;

use std::sync::Arc;

use bevy::log::{Level, LogPlugin};
use bevy::prelude::*;
use strata_globe::{GlobeSnapshot, SnapshotChannel};

use crate::camera::{GlobeCameraState, OriginState};
use crate::camera_controls::{PointerGestureState, handle_camera_input};
use crate::camera_sync::{setup_camera_and_lighting, update_camera_and_origin};
use crate::compass::{setup_compass_ui, update_compass_ui};
use crate::globe_render::{draw_cube_face_seams, setup_globe_materials, sync_globe_tiles};
use crate::markers::{setup_markers_and_hud, sync_markers_and_hud};
use crate::snapshot::{ClientWorldView, SelectedPointState, latch_world_snapshot};

fn main() -> AppExit {
    let initial_snapshot = GlobeSnapshot::reference_earth();
    let planet = initial_snapshot.planet;
    let channel = Arc::new(SnapshotChannel::new(initial_snapshot));
    let world_view = ClientWorldView::new(channel);
    let selected = SelectedPointState::default();

    let focus_dir = selected
        .active_point(world_view.snapshot())
        .map_or(strata_globe::EcefMeters::from_raw(1.0, 1.0, 1.0), |p| {
            p.position
        });
    let Ok(cam_state) = GlobeCameraState::new(planet, focus_dir) else {
        return AppExit::error();
    };
    let (initial_eye, _, _) = cam_state.compute_pose_f64();
    let Ok(origin_state) = OriginState::new(initial_eye) else {
        return AppExit::error();
    };

    App::new()
        .add_plugins(
            DefaultPlugins
                .set(WindowPlugin {
                    primary_window: Some(Window {
                        title: "Project Strata — Globe".into(),
                        ..default()
                    }),
                    ..default()
                })
                .set(LogPlugin {
                    level: Level::ERROR,
                    filter: "strata_client=info".into(),
                    ..default()
                }),
        )
        .insert_resource(ClearColor(Color::srgb(0.02, 0.03, 0.05)))
        .insert_resource(world_view)
        .insert_resource(selected)
        .insert_resource(cam_state)
        .insert_resource(origin_state)
        .init_resource::<PointerGestureState>()
        .add_systems(
            Startup,
            (
                setup_camera_and_lighting,
                setup_globe_materials,
                setup_markers_and_hud,
                setup_compass_ui,
            ),
        )
        .add_systems(
            Update,
            (
                latch_world_snapshot,
                handle_camera_input,
                update_compass_ui,
                update_camera_and_origin,
                sync_globe_tiles,
                draw_cube_face_seams,
                sync_markers_and_hud,
            )
                .chain(),
        )
        .run()
}
