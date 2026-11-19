use std::sync::Arc;
use std::thread;
use std::time::Duration;

use strata_globe::{EcefMeters, GlobeSnapshot, WorldEpoch};
use strata_simulation::{ClockStatus, GameDuration, Simulation, SimulationError, TICK_DURATION};

#[test]
fn uneven_wall_samples_keep_every_nanosecond() -> Result<(), SimulationError> {
    let initial = GlobeSnapshot::reference_earth();
    let mut whole = Simulation::new(initial.clone());
    let mut partitioned = Simulation::new(initial);
    let reader = partitioned.reader();
    let origin = reader.latest();
    let mut total = Duration::ZERO;
    for elapsed_ns in [1, 19_000_000, 894_345_670, 1, 133_777_784] {
        total += Duration::from_nanos(elapsed_ns);
        partitioned.advance_wall(total)?;
        if let Some(interval) = partitioned.pending_interval() {
            partitioned.confirm(interval, None)?;
        }
    }
    assert_eq!(total, Duration::from_nanos(1_047_123_456));
    whole.advance_wall(total)?;
    assert_eq!(whole.target_time(), partitioned.confirmed_time());
    assert_eq!(reader.latest().confirmed_time().ticks(), 20);
    assert!(Arc::ptr_eq(origin.world(), reader.latest().world()));

    // Neither sampling pattern crosses the next boundary until its exact nanosecond.
    let boundary = Duration::from_millis(1050);
    for simulation in [&mut whole, &mut partitioned] {
        simulation.advance_wall(Duration::from_nanos(1_049_999_999))?;
        assert_eq!(simulation.target_time().ticks(), 20);
        simulation.advance_wall(boundary)?;
        assert_eq!(simulation.target_time().ticks(), 21);
        assert_eq!(simulation.target_time().elapsed(), boundary);
    }
    Ok(())
}

#[test]
fn paused_wall_time_is_excluded_but_partial_active_ticks_survive() -> Result<(), SimulationError> {
    let mut simulation = Simulation::new(GlobeSnapshot::reference_earth());
    let reader = simulation.reader();
    simulation.toggle_pause(Duration::from_nanos(49_999_999))?;
    let paused = reader.latest();
    assert_eq!(paused.clock_status(), ClockStatus::Paused);
    assert_eq!(paused.confirmed_time().ticks(), 0);

    simulation.advance_wall(Duration::from_secs(750))?;
    assert!(Arc::ptr_eq(&paused, &reader.latest()));
    assert_eq!(simulation.pending_interval(), None);

    let resume_at = Duration::from_mins(15);
    simulation.toggle_pause(resume_at)?;
    let resumed = reader.latest();
    assert_eq!(resumed.clock_status(), ClockStatus::Running);
    assert_eq!(resumed.confirmed_time(), paused.confirmed_time());
    simulation.advance_wall(resume_at + Duration::from_nanos(1))?;
    let interval = simulation
        .pending_interval()
        .ok_or(SimulationError::InvalidConfirmation)?;
    simulation.confirm(interval, None)?;
    assert_eq!(reader.latest().confirmed_time().ticks(), 1);
    assert_eq!(paused.clock_status(), ClockStatus::Paused);
    Ok(())
}

#[test]
fn delayed_confirmation_drains_through_pause_boundary_and_keeps_readers_coherent()
-> Result<(), SimulationError> {
    let mut simulation = Simulation::new(GlobeSnapshot::reference_earth());
    let reader = simulation.reader();
    let old = reader.latest();

    simulation.advance_wall(TICK_DURATION)?;
    let first = simulation
        .pending_interval()
        .ok_or(SimulationError::InvalidConfirmation)?;
    assert_eq!(first.start(), old.confirmed_time());
    assert_eq!(first.end().ticks(), 1);
    assert!(Arc::ptr_eq(&old, &reader.latest()));
    simulation.toggle_pause(TICK_DURATION * 3)?;
    simulation.advance_wall(Duration::from_secs(1))?;
    simulation.toggle_pause(Duration::from_secs(2))?;
    assert_eq!(simulation.target_time().ticks(), 3);
    thread::scope(|scope| {
        scope.spawn(|| {
            let view = reader.latest();
            assert_eq!(view.clock_status(), ClockStatus::Pausing);
            assert_eq!(view.confirmed_time().ticks(), 0);
            assert!(Arc::ptr_eq(view.world(), old.world()));
        });
    });

    let mut changed = old.world().as_ref().clone();
    let updated_position = EcefMeters::from_raw(100.0, 0.0, changed.planet.radius_meters());
    changed.points[0].position = updated_position;
    simulation.confirm(first, Some(changed))?;
    let middle = reader.latest();
    assert_eq!(middle.confirmed_time().ticks(), 1);
    assert_eq!(middle.clock_status(), ClockStatus::Pausing);
    assert_eq!(middle.world().tick, 1);
    assert_eq!(middle.world().points[0].position, updated_position);
    assert_eq!(
        simulation.confirm(first, None),
        Err(SimulationError::InvalidConfirmation)
    );
    assert!(Arc::ptr_eq(&middle, &reader.latest()));

    let rest = simulation
        .pending_interval()
        .ok_or(SimulationError::InvalidConfirmation)?;
    assert_eq!((rest.start().ticks(), rest.end().ticks()), (1, 3));
    simulation.confirm(rest, None)?;
    let latest = reader.latest();
    assert_eq!(latest.clock_status(), ClockStatus::Paused);
    assert_eq!(latest.confirmed_time(), simulation.target_time());
    assert_eq!(simulation.pending_interval(), None);
    assert!(Arc::ptr_eq(latest.world(), middle.world()));
    assert_eq!(old.confirmed_time().ticks(), 0);
    assert_ne!(
        old.world().points[0].position,
        latest.world().points[0].position
    );
    Ok(())
}

#[test]
fn a_quadrillion_empty_ticks_use_one_shared_world_publication() -> Result<(), SimulationError> {
    let mut simulation = Simulation::new(GlobeSnapshot::reference_earth());
    let reader = simulation.reader();
    let initial = reader.latest();
    let wall = Duration::from_secs(50_000_000_000_000);

    simulation.advance_wall(wall)?;
    assert!(Arc::ptr_eq(&initial, &reader.latest()));
    let interval = simulation
        .pending_interval()
        .ok_or(SimulationError::InvalidConfirmation)?;
    assert_eq!(interval.end().ticks(), 1_000_000_000_000_000);
    assert!(!interval.contains(interval.start()));
    assert!(interval.contains(interval.end()));
    assert_eq!(
        interval
            .end()
            .checked_duration_since(interval.start())
            .map(GameDuration::as_duration),
        Ok(wall)
    );
    simulation.confirm(interval, None)?;
    let latest = reader.latest();
    assert_eq!(latest.confirmed_time().elapsed(), wall);
    assert!(Arc::ptr_eq(initial.world(), latest.world()));
    assert_eq!(simulation.pending_interval(), None);

    // Sampling the same offset again has nothing to confirm or publish.
    simulation.advance_wall(wall)?;
    assert!(Arc::ptr_eq(&latest, &reader.latest()));
    Ok(())
}

#[test]
fn rejected_wall_samples_leave_the_pacer_intact() -> Result<(), SimulationError> {
    let mut initial = GlobeSnapshot::reference_earth();
    initial.tick = u64::MAX - 1;
    let mut simulation = Simulation::new(initial);
    let partial = Duration::from_nanos(49_999_999);
    simulation.advance_wall(partial)?;
    assert_eq!(
        simulation.advance_wall(Duration::ZERO),
        Err(SimulationError::WallTimeWentBackwards)
    );
    assert_eq!(
        simulation.toggle_pause(TICK_DURATION * 2),
        Err(SimulationError::TickOverflow)
    );
    assert_eq!(simulation.target_time().ticks(), u64::MAX - 1);
    assert_eq!(simulation.clock_status(), ClockStatus::Running);
    simulation.advance_wall(TICK_DURATION)?;
    assert_eq!(simulation.target_time().ticks(), u64::MAX);
    Ok(())
}

#[test]
fn an_empty_interval_from_a_previous_epoch_cannot_confirm_the_new_world()
-> Result<(), SimulationError> {
    let mut world = GlobeSnapshot::reference_earth();
    let mut previous = Simulation::new(world.clone());
    world.epoch = WorldEpoch(world.epoch.0 + 1);
    let mut current = Simulation::new(world);
    previous.advance_wall(TICK_DURATION)?;
    current.advance_wall(TICK_DURATION)?;
    let stale = previous
        .pending_interval()
        .ok_or(SimulationError::InvalidConfirmation)?;
    let reader = current.reader();
    let held = reader.latest();
    assert_eq!(
        current.confirm(stale, None),
        Err(SimulationError::WrongWorldEpoch)
    );
    assert!(Arc::ptr_eq(&held, &reader.latest()));
    assert_eq!(current.confirmed_time().ticks(), 0);
    Ok(())
}
