//! Authoritative game time and coherent world publication, without a rendering dependency.

mod clock;
mod game_time;
mod snapshot;

use std::error::Error;
use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use strata_globe::GlobeSnapshot;

pub use crate::clock::TickInterval;
use crate::clock::WallPacer;
pub use crate::game_time::{GameDuration, GameTime, GameTimeError, TICK_DURATION};
pub use crate::snapshot::{ClockStatus, SimulationSnapshot, SnapshotReader};

/// Rejected clock advancement or world confirmation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SimulationError {
    /// The wall sample preceded the last accepted sample.
    WallTimeWentBackwards,
    /// The target exceeded the representable tick range.
    TickOverflow,
    /// The interval overlapped confirmed ticks, left a gap, or exceeded the target.
    InvalidConfirmation,
    /// The interval or replacement world data belong to another timeline.
    WrongWorldEpoch,
}

impl fmt::Display for SimulationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::WallTimeWentBackwards => "wall sample precedes the previous sample",
            Self::TickOverflow => "target game time exceeds the tick range",
            Self::InvalidConfirmation => "interval is not a contiguous prefix of due ticks",
            Self::WrongWorldEpoch => "confirmation belongs to a different world epoch",
        })
    }
}

impl Error for SimulationError {}

/// Single writer for the clock and confirmed world timeline.
#[derive(Debug)]
pub struct Simulation {
    wall: WallPacer,
    confirmed: GameTime,
    status: ClockStatus,
    world: Arc<GlobeSnapshot>,
    reader: SnapshotReader,
}

impl Simulation {
    /// Starts running from the supplied globe's tick with a wall origin of zero.
    #[must_use]
    pub fn new(initial: GlobeSnapshot) -> Self {
        let confirmed = GameTime(initial.tick);
        let status = ClockStatus::Running;
        let world = Arc::new(initial);
        Self {
            wall: WallPacer::new(confirmed),
            confirmed,
            status,
            reader: SnapshotReader::new(SimulationSnapshot {
                confirmed,
                status,
                world: Arc::clone(&world),
            }),
            world,
        }
    }

    /// Subscribes to confirmed publications independently of the writer.
    #[must_use]
    pub fn reader(&self) -> SnapshotReader {
        self.reader.clone()
    }

    /// Most recent whole tick due under wall pacing.
    #[must_use]
    pub const fn target_time(&self) -> GameTime {
        self.wall.target
    }

    /// Most recent tick whose world work is complete.
    #[must_use]
    pub const fn confirmed_time(&self) -> GameTime {
        self.confirmed
    }

    /// Current state of the clock.
    #[must_use]
    pub const fn clock_status(&self) -> ClockStatus {
        self.status
    }

    /// Advances the target from a monotonic offset since the session started.
    /// The sub-tick remainder carries over to the next active interval.
    ///
    /// # Errors
    ///
    /// Rejects a backwards sample or tick overflow without changing state.
    pub fn advance_wall(&mut self, wall_now: Duration) -> Result<(), SimulationError> {
        self.wall
            .advance(wall_now, self.status == ClockStatus::Running)
    }

    /// Requests pause while running, or resumes from a confirmed pause.
    ///
    /// Pause freezes the target at the last whole tick due at the supplied wall
    /// offset. While work through that boundary is pending, further toggles are
    /// ignored. Resume starts pacing at the supplied offset.
    ///
    /// # Errors
    ///
    /// Rejects a backwards sample or tick overflow without changing state.
    pub fn toggle_pause(&mut self, wall_now: Duration) -> Result<(), SimulationError> {
        self.advance_wall(wall_now)?;
        self.status = match self.status {
            ClockStatus::Running if self.pending_interval().is_some() => ClockStatus::Pausing,
            ClockStatus::Running => ClockStatus::Paused,
            ClockStatus::Paused => ClockStatus::Running,
            ClockStatus::Pausing => return Ok(()),
        };
        self.publish();
        Ok(())
    }

    /// Current due range. It can be taken for calculation while readers keep
    /// accessing the previously confirmed snapshot.
    #[must_use]
    pub fn pending_interval(&self) -> Option<TickInterval> {
        (self.confirmed < self.wall.target).then_some(TickInterval {
            epoch: self.world.epoch,
            start: self.confirmed,
            end: self.wall.target,
        })
    }

    /// Confirms a previously obtained interval once all its required work is complete.
    ///
    /// Supply new world data if they changed and `None` if the existing world
    /// remains valid through the entire interval. An interval taken before the
    /// target advanced can still be confirmed as a contiguous prefix.
    ///
    /// # Errors
    ///
    /// Rejects a stale or noncontiguous interval, an end beyond the target, or
    /// world data from another epoch without publishing.
    pub fn confirm(
        &mut self,
        interval: TickInterval,
        world: Option<GlobeSnapshot>,
    ) -> Result<(), SimulationError> {
        if interval.epoch != self.world.epoch {
            return Err(SimulationError::WrongWorldEpoch);
        }
        if !self.pending_interval().is_some_and(|pending| {
            interval.start == pending.start && pending.contains(interval.end)
        }) {
            return Err(SimulationError::InvalidConfirmation);
        }
        if let Some(mut world) = world {
            if world.epoch != self.world.epoch {
                return Err(SimulationError::WrongWorldEpoch);
            }
            world.tick = interval.end.ticks();
            self.world = Arc::new(world);
        }
        self.confirmed = interval.end;
        if self.status == ClockStatus::Pausing && self.confirmed == self.wall.target {
            self.status = ClockStatus::Paused;
        }
        self.publish();
        Ok(())
    }

    fn publish(&self) {
        self.reader.publish(SimulationSnapshot {
            confirmed: self.confirmed,
            status: self.status,
            world: Arc::clone(&self.world),
        });
    }
}
