use std::time::Duration;

use strata_globe::WorldEpoch;

use crate::SimulationError;

const TICKS_PER_SECOND: u64 = 20;
const MILLIS_PER_TICK: u64 = 1000 / TICKS_PER_SECOND;

/// Wall duration of one game tick at the normal pace.
pub const TICK_DURATION: Duration = Duration::from_millis(MILLIS_PER_TICK);

/// Elapsed game time in whole ticks since the start of the world.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct GameTime(pub(crate) u64);

impl GameTime {
    /// Elapsed whole game ticks.
    #[must_use]
    pub const fn ticks(self) -> u64 {
        self.0
    }

    /// Elapsed game duration at the tick boundary.
    #[must_use]
    pub fn elapsed(self) -> Duration {
        Duration::from_secs(self.0 / TICKS_PER_SECOND)
            + Duration::from_millis(self.0 % TICKS_PER_SECOND * MILLIS_PER_TICK)
    }
}

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
            let target = u128::from(self.target.0) + total_ns / tick_ns;
            self.target =
                GameTime(u64::try_from(target).map_err(|_| SimulationError::TickOverflow)?);
            self.remainder_ns = total_ns % tick_ns;
        }
        self.last_wall = wall_now;
        Ok(())
    }
}
