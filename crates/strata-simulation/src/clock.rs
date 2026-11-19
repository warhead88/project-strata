use std::time::Duration;

use strata_globe::WorldEpoch;

use crate::{GameDuration, GameTime, SimulationError, TICK_DURATION};

/// Contiguous due ticks in the range `(start, end]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TickInterval {
    pub(crate) epoch: WorldEpoch,
    pub(crate) start: GameTime,
    pub(crate) end: GameTime,
}

impl TickInterval {
    /// Already confirmed boundary, excluded from the interval.
    #[must_use]
    pub const fn start(self) -> GameTime {
        self.start
    }

    /// Last due tick, included in the interval.
    #[must_use]
    pub const fn end(self) -> GameTime {
        self.end
    }

    /// Whether a deadline on this timeline is after start and at or before end.
    #[must_use]
    pub fn contains(self, deadline: GameTime) -> bool {
        self.start < deadline && deadline <= self.end
    }
}

#[derive(Debug)]
pub(crate) struct WallPacer {
    pub(crate) target: GameTime,
    last_wall: Duration,
    remainder_ns: u128,
}

impl WallPacer {
    pub(crate) const fn new(initial: GameTime) -> Self {
        Self {
            target: initial,
            last_wall: Duration::ZERO,
            remainder_ns: 0,
        }
    }

    pub(crate) fn advance(
        &mut self,
        wall_now: Duration,
        running: bool,
    ) -> Result<(), SimulationError> {
        let delta = wall_now
            .checked_sub(self.last_wall)
            .ok_or(SimulationError::WallTimeWentBackwards)?;
        if running {
            let tick_ns = TICK_DURATION.as_nanos();
            let total_ns = delta.as_nanos() + self.remainder_ns;
            let ticks =
                u64::try_from(total_ns / tick_ns).map_err(|_| SimulationError::TickOverflow)?;
            self.target = self
                .target
                .checked_add(GameDuration(ticks))
                .map_err(|_| SimulationError::TickOverflow)?;
            self.remainder_ns = total_ns % tick_ns;
        }
        self.last_wall = wall_now;
        Ok(())
    }
}
