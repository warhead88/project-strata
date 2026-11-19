use std::error::Error;
use std::time::Duration;

use strata_globe::{GlobeSnapshot, PointId};
use strata_simulation::{
    GameDuration, GameTime, GameTimeError, Simulation, SimulationError, TickInterval,
};

#[test]
fn duration_arithmetic_preserves_existing_ticks_and_physical_units() -> Result<(), GameTimeError> {
    let mut initial = GlobeSnapshot::reference_earth();
    initial.tick = 27;
    let now = Simulation::new(initial).confirmed_time();
    assert_eq!(now.elapsed(), Duration::from_millis(1350));
    let delay = GameDuration::try_from(Duration::from_millis(2250))?;
    let deadline = now.checked_add(delay)?;
    assert_eq!(deadline.ticks(), 72);
    assert_eq!(deadline.elapsed(), Duration::from_millis(3600));
    assert_eq!(deadline.checked_duration_since(now)?, delay);
    assert_eq!(delay.as_duration(), Duration::from_millis(2250));
    assert_eq!(
        now.checked_duration_since(deadline),
        Err(GameTimeError::ReversedInterval)
    );

    let zero = GameDuration::try_from(Duration::ZERO)?;
    assert_eq!(now.checked_add(zero)?, now);
    assert_eq!(now.checked_duration_since(now)?, zero);
    Ok(())
}

#[test]
fn fractional_and_out_of_range_durations_are_rejected() -> Result<(), GameTimeError> {
    for unaligned in [
        Duration::from_nanos(1),
        Duration::from_millis(75),
        Duration::from_nanos(100_000_001),
    ] {
        assert_eq!(
            GameDuration::try_from(unaligned),
            Err(GameTimeError::FractionalTick)
        );
    }

    let mut initial = GlobeSnapshot::reference_earth();
    let origin = Simulation::new(initial.clone()).confirmed_time();
    initial.tick = u64::MAX;
    let last = Simulation::new(initial).confirmed_time();
    let span = last.checked_duration_since(origin)?;
    let max = Duration::new(922_337_203_685_477_580, 750_000_000);
    assert_eq!(span.as_duration(), max);
    assert_eq!(last.elapsed(), max);
    assert_eq!(origin.checked_add(GameDuration::try_from(max)?)?, last);
    assert_eq!(
        GameDuration::try_from(max + Duration::from_millis(50)),
        Err(GameTimeError::TickOverflow)
    );
    assert_eq!(
        last.checked_add(GameDuration::try_from(Duration::from_millis(50))?),
        Err(GameTimeError::TickOverflow)
    );
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    Waiting,
    Inspected,
    Verified(GameTime),
}

/// Test world owner that inspects a point, records a reading, then verifies the inspection.
#[derive(Debug, PartialEq, Eq)]
struct PointInspection {
    phase: Phase,
    phase_due: Option<GameTime>,
    reading_due: Option<GameTime>,
    reading: Option<Phase>,
    verification_delay: GameDuration,
}

impl PointInspection {
    fn new(start: GameTime) -> Result<Self, GameTimeError> {
        let inspection_due =
            start.checked_add(GameDuration::try_from(Duration::from_millis(150))?)?;
        Ok(Self {
            phase: Phase::Waiting,
            phase_due: Some(inspection_due),
            reading_due: Some(inspection_due),
            reading: None,
            verification_delay: GameDuration::try_from(Duration::from_millis(100))?,
        })
    }

    fn apply_due(&mut self, interval: TickInterval) -> Result<bool, GameTimeError> {
        let mut phase_changed = false;
        loop {
            let phase_due = self.phase_due.filter(|&due| interval.contains(due));
            let reading_due = self.reading_due.filter(|&due| interval.contains(due));
            // At equal times an inspection transition precedes the reading.
            if let Some(due) = phase_due
                && reading_due.is_none_or(|reading| due <= reading)
            {
                if self.phase == Phase::Waiting {
                    self.phase_due = Some(due.checked_add(self.verification_delay)?);
                    self.phase = Phase::Inspected;
                } else {
                    self.phase_due = None;
                    self.phase = Phase::Verified(due);
                }
                phase_changed = true;
            } else if reading_due.is_some() {
                self.reading = Some(self.phase);
                self.reading_due = None;
            } else {
                return Ok(phase_changed);
            }
        }
    }

    fn confirm(
        &mut self,
        simulation: &mut Simulation,
        interval: TickInterval,
    ) -> Result<(), Box<dyn Error>> {
        let world = self.apply_due(interval)?.then(|| {
            let mut world = simulation.reader().latest().world().as_ref().clone();
            let label = match self.phase {
                Phase::Waiting => "Waiting",
                Phase::Inspected => "Inspected",
                Phase::Verified(_) => "Verified",
            };
            label.clone_into(&mut world.points[0].label);
            world
        });
        simulation.confirm(interval, world)?;
        Ok(())
    }
}

#[test]
fn interval_partition_preserves_the_order_and_effects_of_internal_deadlines()
-> Result<(), Box<dyn Error>> {
    let initial = GlobeSnapshot::reference_earth();
    let mut whole = Simulation::new(initial.clone());
    let mut split = Simulation::new(initial);
    let start = whole.confirmed_time();
    let mut whole_owner = PointInspection::new(start)?;
    let mut split_owner = PointInspection::new(start)?;
    let boundary = start.checked_add(GameDuration::try_from(Duration::from_millis(150))?)?;
    let verification = start.checked_add(GameDuration::try_from(Duration::from_millis(250))?)?;

    whole.advance_wall(Duration::from_millis(300))?;
    let interval = whole
        .pending_interval()
        .ok_or(SimulationError::InvalidConfirmation)?;
    assert!(!interval.contains(start));
    assert!(interval.contains(boundary));
    assert!(interval.contains(verification));
    assert!(interval.contains(interval.end()));
    whole_owner.confirm(&mut whole, interval)?;

    for millis in [50, 150, 200, 300] {
        split.advance_wall(Duration::from_millis(millis))?;
        let interval = split
            .pending_interval()
            .ok_or(SimulationError::InvalidConfirmation)?;
        assert_eq!(interval.contains(boundary), millis == 150);
        assert_eq!(interval.contains(verification), millis == 300);
        split_owner.confirm(&mut split, interval)?;
        if millis == 150 {
            assert_eq!(split_owner.phase, Phase::Inspected);
            assert_eq!(split_owner.reading, Some(Phase::Inspected));
        }
    }

    assert_eq!(whole_owner, split_owner);
    assert_eq!(whole_owner.reading, Some(Phase::Inspected));
    assert_eq!(whole_owner.phase, Phase::Verified(verification));
    let whole_view = whole.reader().latest();
    let split_view = split.reader().latest();
    assert_eq!(whole_view.confirmed_time(), split_view.confirmed_time());
    assert_eq!(whole_view.world(), split_view.world());
    assert_eq!(
        whole_view
            .world()
            .point(PointId(1))
            .map(|point| point.label.as_str()),
        Some("Verified")
    );
    Ok(())
}
