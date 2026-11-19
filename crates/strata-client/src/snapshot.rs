use std::sync::Arc;

use bevy::prelude::*;
use strata_globe::{AuthoritativePoint, GlobeSnapshot, PointId, SnapshotChannel};

/// Read-only client view of the latest published simulation [`GlobeSnapshot`].
///
/// The client reads an immutable `Arc<GlobeSnapshot>` at the start of the frame
/// and never mutates authoritative `f64` coordinates from `f32` render transforms.
#[derive(Resource)]
pub struct ClientWorldView {
    channel: Arc<SnapshotChannel>,
    frame_snapshot: Arc<GlobeSnapshot>,
}

impl ClientWorldView {
    /// Creates a [`ClientWorldView`] attached to a published [`SnapshotChannel`].
    #[must_use]
    pub fn new(channel: Arc<SnapshotChannel>) -> Self {
        let frame_snapshot = channel.latest();
        Self {
            channel,
            frame_snapshot,
        }
    }

    /// Refreshes the per-frame snapshot reference atomically from the channel.
    pub fn latch_latest(&mut self) {
        self.frame_snapshot = self.channel.latest();
    }

    /// Immutable snapshot latched for the current frame.
    #[must_use]
    pub fn snapshot(&self) -> &GlobeSnapshot {
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
    use strata_globe::{EcefMeters, PlanetSpec, WorldEpoch};

    #[test]
    fn client_view_latches_immutable_snapshot_and_cycles_selection() {
        let initial = GlobeSnapshot::reference_earth();
        let channel = Arc::new(SnapshotChannel::new(initial));
        let mut view = ClientWorldView::new(Arc::clone(&channel));
        let mut sel = SelectedPointState::default();

        assert_eq!(view.snapshot().tick, 0);
        let first_id = sel.selected_id;
        sel.cycle(view.snapshot(), 1);
        assert_ne!(sel.selected_id, first_id);

        // Publish a newer tick without latching yet: client frame view stays on tick 0.
        let mut next = GlobeSnapshot::reference_for_planet(PlanetSpec::earth());
        next.epoch = WorldEpoch(2);
        next.tick = 5;
        next.points[0].position =
            EcefMeters::from_raw(0.0, 0.0, PlanetSpec::earth().radius_meters());
        channel.publish(next);
        assert_eq!(view.snapshot().tick, 0);

        view.latch_latest();
        assert_eq!(view.snapshot().tick, 5);
        assert_eq!(view.snapshot().epoch, WorldEpoch(2));
    }
}
