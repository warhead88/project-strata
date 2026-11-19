use std::io;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender};
use std::thread;
use std::time::Instant;

use bevy::log::{error, warn};
use bevy::prelude::*;
use strata_globe::GlobeSnapshot;
use strata_simulation::{ClockStatus, Simulation, SimulationError, SnapshotReader, TICK_DURATION};

#[derive(Resource)]
pub struct ClockControl(SyncSender<()>);

pub fn start_clock(initial: GlobeSnapshot) -> io::Result<(ClockControl, SnapshotReader)> {
    let simulation = Simulation::new(initial);
    let reader = simulation.reader();
    let (sender, receiver) = mpsc::sync_channel(1);
    thread::Builder::new()
        .name("strata-simulation".into())
        .spawn(move || {
            if let Err(error) = run_clock(simulation, &receiver) {
                error!("simulation clock stopped: {error}");
            }
        })?;
    Ok((ClockControl(sender), reader))
}

fn run_clock(mut simulation: Simulation, controls: &Receiver<()>) -> Result<(), SimulationError> {
    let origin = Instant::now();
    loop {
        let control = if simulation.clock_status() == ClockStatus::Paused {
            controls.recv().map_err(RecvTimeoutError::from)
        } else {
            controls.recv_timeout(TICK_DURATION)
        };
        match control {
            Ok(()) => simulation.toggle_pause(origin.elapsed())?,
            Err(RecvTimeoutError::Timeout) => simulation.advance_wall(origin.elapsed())?,
            Err(RecvTimeoutError::Disconnected) => return Ok(()),
        }

        // The globe is static, so every due interval is complete in one operation.
        if let Some(interval) = simulation.pending_interval() {
            simulation.confirm(interval, None)?;
        }
    }
}

#[expect(
    clippy::needless_pass_by_value,
    reason = "Bevy systems acquire shared resources through Res<T>"
)]
pub fn handle_pause_input(keys: Res<ButtonInput<KeyCode>>, control: Res<ClockControl>) {
    if keys.just_pressed(KeyCode::Space)
        && let Err(error) = control.0.try_send(())
    {
        warn!("could not submit pause/resume: {error}");
    }
}

#[cfg(test)]
mod tests {
    use bevy::input::mouse::{MouseMotion, MouseWheel};
    use strata_globe::EcefMeters;

    use super::*;
    use crate::camera::GlobeCameraState;
    use crate::camera_controls::{PointerGestureState, handle_camera_input};
    use crate::snapshot::{ClientWorldView, SelectedPointState, latch_world_snapshot};
    use crate::terrain::TerrainElevation;

    #[test]
    fn space_submits_one_toggle_and_camera_moves_while_confirmation_is_pending_and_paused()
    -> Result<(), Box<dyn std::error::Error>> {
        let initial = GlobeSnapshot::reference_earth();
        let focus = EcefMeters::from_raw(0.0, 0.0, 1.0);
        let camera = GlobeCameraState::new(initial.planet, focus)?;
        let mut simulation = Simulation::new(initial);
        let (sender, receiver) = mpsc::sync_channel(1);
        let mut keys = ButtonInput::default();
        keys.press(KeyCode::Space);
        keys.press(KeyCode::KeyD);
        let mut time = Time::<()>::default();
        time.advance_by(TICK_DURATION);

        let mut app = App::new();
        app.insert_resource(time)
            .insert_resource(keys)
            .insert_resource(ClockControl(sender))
            .insert_resource(camera)
            .insert_resource(ClientWorldView::new(simulation.reader()))
            .init_resource::<ButtonInput<MouseButton>>()
            .init_resource::<PointerGestureState>()
            .init_resource::<SelectedPointState>()
            .init_resource::<TerrainElevation>()
            .add_message::<MouseWheel>()
            .add_message::<MouseMotion>()
            .add_systems(
                Update,
                (
                    latch_world_snapshot,
                    handle_pause_input,
                    handle_camera_input,
                )
                    .chain(),
            );

        app.update();
        receiver.try_recv()?;
        simulation.toggle_pause(TICK_DURATION)?;
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .clear_just_pressed(KeyCode::Space);

        let before = camera_focus(&app);
        app.update();
        let during = camera_focus(&app);
        assert!(during.distance_to(before) > 1e-5);
        let frame = app.world().resource::<ClientWorldView>().frame();
        assert_eq!(frame.clock_status(), ClockStatus::Pausing);
        assert_eq!(frame.confirmed_time().ticks(), 0);
        assert_eq!(receiver.try_recv(), Err(mpsc::TryRecvError::Empty));

        let interval = simulation
            .pending_interval()
            .ok_or(SimulationError::InvalidConfirmation)?;
        simulation.confirm(interval, None)?;
        app.update();
        assert!(camera_focus(&app).distance_to(during) > 1e-5);
        let frame = app.world().resource::<ClientWorldView>().frame();
        assert_eq!(frame.clock_status(), ClockStatus::Paused);
        assert_eq!(frame.confirmed_time().ticks(), 1);
        Ok(())
    }

    fn camera_focus(app: &App) -> EcefMeters {
        app.world().resource::<GlobeCameraState>().focus_unit
    }
}
