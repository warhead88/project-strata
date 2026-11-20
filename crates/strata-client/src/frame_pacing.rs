use std::time::{Duration, Instant};

use bevy::prelude::*;

/// Present at up to 60 Hz on displays with higher refresh rates.
pub(crate) fn pace(mut previous: Local<Option<Instant>>) {
    const PERIOD: Duration = Duration::from_nanos(16_666_667);
    if let Some(last) = *previous {
        std::thread::sleep(PERIOD.saturating_sub(last.elapsed()));
    }
    *previous = Some(Instant::now());
}
