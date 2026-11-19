use std::error::Error;
use std::fmt;
use std::time::Duration;

const TICKS_PER_SECOND: u64 = 20;
const MILLIS_PER_TICK: u64 = 1000 / TICKS_PER_SECOND;

/// Duration of one game tick, also its wall duration at the normal pace.
pub const TICK_DURATION: Duration = Duration::from_millis(MILLIS_PER_TICK);

/// An invalid operation on game time or a duration outside its exact range.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GameTimeError {
    /// The input duration has a fractional tick.
    FractionalTick,
    /// The duration or resulting moment exceeds the range of whole ticks.
    TickOverflow,
    /// The end of an elapsed interval precedes its start.
    ReversedInterval,
}

impl fmt::Display for GameTimeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::FractionalTick => "game duration must be a whole number of ticks",
            Self::TickOverflow => "game time exceeds the tick range",
            Self::ReversedInterval => "game interval ends before it starts",
        })
    }
}

impl Error for GameTimeError {}

/// A nonnegative game duration quantized to whole ticks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct GameDuration(pub(crate) u64);

impl GameDuration {
    /// Converts to an exact duration in physical time units.
    #[must_use]
    pub fn as_duration(self) -> Duration {
        // Even u64::MAX ticks fit: the seconds are divided by 20 and the
        // remaining milliseconds are less than a second.
        Duration::from_secs(self.0 / TICKS_PER_SECOND)
            + Duration::from_millis(self.0 % TICKS_PER_SECOND * MILLIS_PER_TICK)
    }
}

impl TryFrom<Duration> for GameDuration {
    type Error = GameTimeError;

    /// Converts a duration in physical time units to game duration.
    ///
    /// # Errors
    ///
    /// Rejects any fractional tick without rounding, then checks the whole-tick
    /// range. Zero is valid.
    fn try_from(duration: Duration) -> Result<Self, Self::Error> {
        let nanos = duration.as_nanos();
        let tick_nanos = TICK_DURATION.as_nanos();
        if !nanos.is_multiple_of(tick_nanos) {
            return Err(GameTimeError::FractionalTick);
        }
        let ticks = u64::try_from(nanos / tick_nanos).map_err(|_| GameTimeError::TickOverflow)?;
        Ok(Self(ticks))
    }
}

/// Elapsed game time in whole ticks since the start of the world.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct GameTime(pub(crate) u64);

impl GameTime {
    /// Elapsed whole game ticks.
    #[must_use]
    pub const fn ticks(self) -> u64 {
        self.0
    }

    /// Elapsed game duration at the tick boundary in physical time units.
    #[must_use]
    pub fn elapsed(self) -> Duration {
        GameDuration(self.0).as_duration()
    }

    /// Returns the moment after the given game duration. Zero returns this moment.
    ///
    /// # Errors
    ///
    /// Returns [`GameTimeError::TickOverflow`] if the result exceeds the tick range.
    pub fn checked_add(self, duration: GameDuration) -> Result<Self, GameTimeError> {
        self.0
            .checked_add(duration.0)
            .map(Self)
            .ok_or(GameTimeError::TickOverflow)
    }

    /// Returns the game duration from `start` to this moment. Equal moments give zero.
    ///
    /// # Errors
    ///
    /// Returns [`GameTimeError::ReversedInterval`] if this moment precedes `start`.
    pub fn checked_duration_since(self, start: Self) -> Result<GameDuration, GameTimeError> {
        self.0
            .checked_sub(start.0)
            .map(GameDuration)
            .ok_or(GameTimeError::ReversedInterval)
    }
}
