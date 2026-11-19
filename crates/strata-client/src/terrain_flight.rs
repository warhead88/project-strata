mod gpu_timing;

use std::{collections::BTreeMap, path::PathBuf, time::Instant};

use bevy::{
    diagnostic::DiagnosticsStore,
    prelude::*,
    render::{
        diagnostic::RenderDiagnosticsPlugin,
        view::screenshot::{Screenshot, save_to_disk},
    },
    window::{PresentMode, WindowResolution},
    winit::WinitSettings,
};
use strata_terrain::lat_lon_to_unit;

use crate::{camera::GlobeCameraState, camera_sync::initial_north_tangent};

struct Leg {
    name: &'static str,
    seconds: f64,
    from: [f64; 3],
    to: [f64; 3],
    pitch_deg: f64,
}

// Waypoints are latitude, longitude (degrees) and altitude above the CPU terrain (meters).
const FLIGHT: [Leg; 6] = [
    Leg {
        name: "orbit",
        seconds: 6.0,
        from: [28.0, 25.0, 16e6],
        to: [46.54, 10.12, 16e6],
        pitch_deg: 0.0,
    },
    Leg {
        name: "descent",
        seconds: 10.0,
        from: [46.54, 10.12, 16e6],
        to: [46.54, 10.12, 20.0],
        pitch_deg: 45.0,
    },
    Leg {
        name: "alps_20m",
        seconds: 6.0,
        from: [46.54, 10.12, 20.0],
        to: [46.5412, 10.122, 20.0],
        pitch_deg: 55.0,
    },
    Leg {
        name: "cube_seam_20m",
        seconds: 6.0,
        from: [33.45, 44.998, 20.0],
        to: [33.451, 45.002, 20.0],
        pitch_deg: 55.0,
    },
    Leg {
        name: "nile_delta",
        seconds: 6.0,
        from: [30.75, 30.75, 90_000.0],
        to: [31.4, 31.3, 30_000.0],
        pitch_deg: 35.0,
    },
    Leg {
        name: "caspian",
        seconds: 6.0,
        from: [39.4, 49.2, 350_000.0],
        to: [42.0, 51.0, 350_000.0],
        pitch_deg: 25.0,
    },
];
const WARMUP_SECS: f64 = 4.0;

#[derive(Resource)]
struct Flight {
    begun: Instant,
    update_started: Instant,
    previous: Option<Instant>,
    leg: usize,
    samples: BTreeMap<String, Vec<f64>>,
    latest: BTreeMap<String, f64>,
    captures: Option<PathBuf>,
    captured: bool,
}

pub(crate) fn window() -> Window {
    Window {
        title: "Project Strata — Terrain Flight".into(),
        resolution: WindowResolution::new(1280, 720).with_scale_factor_override(1.0),
        present_mode: PresentMode::AutoNoVsync,
        resizable: false,
        ..default()
    }
}

pub(crate) fn install(app: &mut App) {
    let now = Instant::now();
    gpu_timing::install(app);
    app.insert_resource(WinitSettings::continuous())
        .add_plugins(RenderDiagnosticsPlugin)
        .insert_resource(Flight {
            begun: now,
            update_started: now,
            previous: None,
            leg: 0,
            samples: BTreeMap::new(),
            latest: BTreeMap::new(),
            captures: std::env::var_os("STRATA_FLIGHT_CAPTURES").map(PathBuf::from),
            captured: false,
        })
        .add_systems(First, start_update)
        .add_systems(
            Update,
            fly.before(crate::camera_sync::update_camera_and_origin),
        )
        .add_systems(Last, end_update);
}

fn start_update(mut flight: ResMut<Flight>) {
    let now = Instant::now();
    flight.update_started = now;
    let mut elapsed = (now - flight.begun).as_secs_f64();
    let first = flight.previous.is_none();
    if first {
        flight.begun = now;
        elapsed = 0.0;
    }
    let previous = flight.previous.replace(now);
    if let (Some(start), Some((index, _))) = (previous, leg_at(elapsed))
        && index == flight.leg
        && elapsed >= WARMUP_SECS
    {
        flight.record("frame", (now - start).as_secs_f64() * 1000.0);
    }
}

fn leg_at(elapsed: f64) -> Option<(usize, f64)> {
    let mut position = (elapsed - WARMUP_SECS).max(0.0);
    for (index, leg) in FLIGHT.iter().enumerate() {
        if position < leg.seconds {
            return Some((index, position / leg.seconds));
        }
        position -= leg.seconds;
    }
    None
}

fn pose(leg: &Leg, t: f64) -> [f64; 3] {
    let lerp = |axis: usize| leg.from[axis] + (leg.to[axis] - leg.from[axis]) * t;
    let altitude = (leg.from[2].ln() + (leg.to[2].ln() - leg.from[2].ln()) * t).exp();
    [lerp(0), lerp(1), altitude.max(20.0)]
}

fn fly(
    mut commands: Commands,
    mut camera: ResMut<GlobeCameraState>,
    mut flight: ResMut<Flight>,
    mut exit: MessageWriter<AppExit>,
) {
    let elapsed = (flight.update_started - flight.begun).as_secs_f64();
    let Some((index, t)) = leg_at(elapsed) else {
        flight.report();
        exit.write(AppExit::Success);
        return;
    };
    if flight.leg != index {
        flight.report();
        flight.leg = index;
        flight.captured = false;
    }
    let leg = &FLIGHT[index];
    let [lat, lon, altitude] = pose(leg, t);
    camera.focus_unit = lat_lon_to_unit(lat, lon);
    camera.heading_unit = initial_north_tangent(camera.focus_unit);
    camera.altitude_meters = altitude;
    camera.target_altitude_meters = altitude;
    camera.pitch_rad = leg.pitch_deg.to_radians();
    camera.zoom_anchor = None;
    if t >= 0.5
        && !flight.captured
        && let Some(dir) = &flight.captures
    {
        commands
            .spawn(Screenshot::primary_window())
            .observe(save_to_disk(dir.join(format!("{}.png", leg.name))));
        flight.captured = true;
    }
}

#[expect(
    clippy::needless_pass_by_value,
    reason = "Bevy diagnostics are a system parameter"
)]
fn end_update(
    mut flight: ResMut<Flight>,
    diagnostics: Res<DiagnosticsStore>,
    gpu: Res<gpu_timing::GpuTimings>,
) {
    for (started, gpu_ms) in gpu.take() {
        let elapsed = started
            .saturating_duration_since(flight.begun)
            .as_secs_f64();
        if elapsed >= WARMUP_SECS
            && let Some((leg, _)) = leg_at(elapsed)
            && leg == flight.leg
        {
            flight.record("camera_gpu", gpu_ms);
        }
    }
    if (flight.update_started - flight.begun).as_secs_f64() < WARMUP_SECS {
        return;
    }
    let cpu_ms = flight.update_started.elapsed().as_secs_f64() * 1000.0;
    flight.record("app_update_cpu", cpu_ms);
    let sample_names = [
        "terrain_select",
        "terrain_draw",
        "main_opaque_pass_3d",
        "main_transparent_pass_3d",
        "ui_pass",
    ];
    for entry in diagnostics.iter() {
        let name = entry.path().as_str();
        if sample_names.iter().any(|part| name.contains(part))
            && let Some(measurement) = entry.measurement()
        {
            let timestamp = measurement
                .time
                .saturating_duration_since(flight.begun)
                .as_secs_f64();
            if flight
                .latest
                .get(name)
                .is_none_or(|&previous| previous < timestamp)
            {
                flight.latest.insert(name.to_owned(), timestamp);
                flight.record(name, measurement.value);
            }
        }
    }
}

impl Flight {
    fn record(&mut self, key: &str, milliseconds: f64) {
        self.samples
            .entry(key.to_owned())
            .or_default()
            .push(milliseconds);
    }

    fn report(&mut self) {
        let leg = FLIGHT[self.leg].name;
        for (name, mut values) in std::mem::take(&mut self.samples) {
            values.sort_by(f64::total_cmp);
            let n = values.len();
            println!(
                "terrain_flight {leg} {name} n={n} p50_ms={:.3} p95_ms={:.3}",
                values[n / 2],
                values[(n - 1) * 95 / 100]
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn route_sweeps_orbit_to_ground_and_crosses_a_cube_seam() {
        assert!(pose(&FLIGHT[1], 0.0)[2] > 15e6);
        assert!((pose(&FLIGHT[1], 1.0)[2] - 20.0).abs() < 1e-6);
        let seam = &FLIGHT[3];
        let face = |t| {
            let [lat, lon, altitude] = pose(seam, t);
            assert!((altitude - 20.0).abs() < 1e-6);
            strata_globe::CubeFace::from_dominant_axis(lat_lon_to_unit(lat, lon))
        };
        assert_ne!(face(0.0), face(1.0));
        assert!(leg_at(100.0).is_none());
    }
}
