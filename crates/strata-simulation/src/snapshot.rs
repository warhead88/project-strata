use std::mem;
use std::sync::{Arc, PoisonError, RwLock};

use strata_globe::GlobeSnapshot;

use crate::GameTime;

/// Published state of the game clock.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClockStatus {
    /// Wall time contributes to the target.
    Running,
    /// The target is frozen and due ticks are being confirmed.
    Pausing,
    /// All ticks through the pause boundary are confirmed.
    Paused,
}

/// One coherent publication of world data and confirmed game time.
#[derive(Debug)]
pub struct SimulationSnapshot {
    pub(crate) confirmed: GameTime,
    pub(crate) status: ClockStatus,
    pub(crate) world: Arc<GlobeSnapshot>,
}

impl SimulationSnapshot {
    /// Confirmed boundary represented by this publication.
    #[must_use]
    pub const fn confirmed_time(&self) -> GameTime {
        self.confirmed
    }

    /// State of the clock at publication.
    #[must_use]
    pub const fn clock_status(&self) -> ClockStatus {
        self.status
    }

    /// Immutable world data, shared by publications where the world is unchanged.
    ///
    /// The globe's own tick identifies when these world data were produced.
    #[must_use]
    pub const fn world(&self) -> &Arc<GlobeSnapshot> {
        &self.world
    }
}

/// Read-only subscription to the latest complete snapshot.
#[derive(Debug, Clone)]
pub struct SnapshotReader(Arc<RwLock<Arc<SimulationSnapshot>>>);

impl SnapshotReader {
    pub(crate) fn new(initial: SimulationSnapshot) -> Self {
        Self(Arc::new(RwLock::new(Arc::new(initial))))
    }

    /// Latches one immutable snapshot. A held snapshot remains valid after publication.
    #[must_use]
    pub fn latest(&self) -> Arc<SimulationSnapshot> {
        Arc::clone(&self.0.read().unwrap_or_else(PoisonError::into_inner))
    }

    pub(crate) fn publish(&self, snapshot: SimulationSnapshot) {
        let next = Arc::new(snapshot);
        let old = {
            let mut latest = self.0.write().unwrap_or_else(PoisonError::into_inner);
            mem::replace(&mut *latest, next)
        };
        drop(old);
    }
}
