use std::sync::Arc;

use bevy::prelude::*;
use strata_globe::{AuthoritativePoint, GlobeSnapshot, PointId};
use strata_simulation::{SimulationSnapshot, SnapshotReader};

/// Read-only client view of the latest published simulation snapshot.
///
/// The client latches confirmed time and world together at the start of the frame
/// and never mutates authoritative `f64` coordinates from `f32` render transforms.
#[derive(Resource)]
pub struct ClientWorldView {
    reader: SnapshotReader,
    frame_snapshot: Arc<SimulationSnapshot>,
}

impl ClientWorldView {
    /// Creates a [`ClientWorldView`] subscribed to confirmed simulation state.
    #[must_use]
    pub fn new(reader: SnapshotReader) -> Self {
        let frame_snapshot = reader.latest();
        Self {
            reader,
            frame_snapshot,
        }
    }

    /// Refreshes the per-frame snapshot reference atomically from the channel.
    pub fn latch_latest(&mut self) {
        self.frame_snapshot = self.reader.latest();
    }

    /// World data latched for the current frame.
    #[must_use]
    pub fn snapshot(&self) -> &GlobeSnapshot {
        self.frame_snapshot.world()
    }

    /// Whole publication latched for the current frame.
    #[must_use]
    pub fn frame(&self) -> &SimulationSnapshot {
        &self.frame_snapshot
    }
}

/// Client-only presentation selection & zoom-sweep state.
#[derive(Resource)]
pub struct SelectedPointState {
    /// Stable ID of the currently focused authoritative control point.
    pub selected_id: PointId,
    /// When `true`, continuously sweeps altitude logarithmically between Planet and Local.
    pub auto_sweep: bool,
    /// Normalized sweep phase `[0, 2)` across a Planet -> Local -> Planet cycle.
    pub sweep_phase: f64,
}

impl Default for SelectedPointState {
    fn default() -> Self {
        Self {
            selected_id: PointId(4), // Start on 3-face cube corner (+X/+Y/+Z) for clear seam inspection.
            auto_sweep: false,
            sweep_phase: 0.0,
        }
    }
}

impl SelectedPointState {
    /// Returns the currently selected [`AuthoritativePoint`] in `snapshot`, falling back to the first point.
    #[must_use]
    pub fn active_point<'a>(&self, snapshot: &'a GlobeSnapshot) -> Option<&'a AuthoritativePoint> {
        snapshot
            .point(self.selected_id)
            .or_else(|| snapshot.points.first())
    }

    /// Cycles selection by `step` across the snapshot's authoritative points.
    pub fn cycle(&mut self, snapshot: &GlobeSnapshot, step: isize) {
        if snapshot.points.is_empty() {
            return;
        }
        let current_idx = snapshot
            .points
            .iter()
            .position(|p| p.id == self.selected_id)
            .unwrap_or(0);
        let len = snapshot.points.len().cast_signed();
        let next_idx = (current_idx.cast_signed() + step)
            .rem_euclid(len)
            .cast_unsigned();
        self.selected_id = snapshot.points[next_idx].id;
    }

    /// Selects a specific 1-based control point index (`1..=points.len()`).
    pub fn select_one_based(&mut self, snapshot: &GlobeSnapshot, one_based: usize) {
        if let Some(idx) = one_based.checked_sub(1)
            && let Some(point) = snapshot.points.get(idx)
        {
            self.selected_id = point.id;
        }
    }
}

/// System latching the latest published simulation snapshot once at the start of the frame.
pub fn latch_world_snapshot(mut world_view: ResMut<ClientWorldView>) {
    world_view.latch_latest();
}

#[cfg(test)]
mod tests {
    use super::*;
    use strata_globe::EcefMeters;
    use strata_simulation::{ClockStatus, Simulation, SimulationError, TICK_DURATION};

    #[test]
    fn client_view_latches_clock_and_points_together_and_can_skip_publications()
    -> Result<(), SimulationError> {
        let initial = GlobeSnapshot::reference_earth();
        let mut sim = Simulation::new(initial.clone());
        let mut view = ClientWorldView::new(sim.reader());
        let mut sel = SelectedPointState::default();

        let first_id = sel.selected_id;
        sel.cycle(view.snapshot(), 1);
        assert_ne!(sel.selected_id, first_id);

        sim.toggle_pause(TICK_DURATION * 5)?;
        let interval = sim
            .pending_interval()
            .ok_or(SimulationError::InvalidConfirmation)?;
        let mut next = initial.clone();
        let position = EcefMeters::from_raw(0.0, 0.0, next.planet.radius_meters());
        next.points[0].position = position;
        sim.confirm(interval, Some(next))?;

        // Both publications arrived while the client held its running frame.
        assert_eq!(view.frame().clock_status(), ClockStatus::Running);
        assert_eq!(view.frame().confirmed_time().ticks(), 0);
        assert_eq!(
            view.snapshot().points[0].position,
            initial.points[0].position
        );

        view.latch_latest();
        assert_eq!(view.frame().clock_status(), ClockStatus::Paused);
        assert_eq!(view.frame().confirmed_time().elapsed(), TICK_DURATION * 5);
        assert_eq!(view.snapshot().tick, 5);
        assert_eq!(view.snapshot().points[0].position, position);
        Ok(())
    }
}
