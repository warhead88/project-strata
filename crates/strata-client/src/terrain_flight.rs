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
// 285 m per physical pixel at the center of a 720-pixel, 60-degree nadir view.
const REGIONAL_ALTITUDE_M: f64 = 177_708.4;
const KERCH: [f64; 3] = [45.29, 36.52, REGIONAL_ALTITUDE_M];
const KERCH_WEST: [f64; 3] = [45.15, 35.4, 230_000.0];
const KERCH_EAST: [f64; 3] = [
    45.43,
    37.64,
    (REGIONAL_ALTITUDE_M * REGIONAL_ALTITUDE_M) / 230_000.0,
];
const REGIONAL: [Leg; 6] = [
    Leg {
        name: "kerch_out",
        seconds: 8.0,
        from: KERCH_WEST,
        to: KERCH_EAST,
        pitch_deg: 0.0,
    },
    Leg {
        name: "kerch_back",
        seconds: 8.0,
        from: KERCH_EAST,
        to: KERCH_WEST,
        pitch_deg: 0.0,
    },
    Leg {
        name: "baikal",
        seconds: 6.0,
        from: [53.05, 107.13, REGIONAL_ALTITUDE_M],
        to: [53.2, 107.7, REGIONAL_ALTITUDE_M],
        pitch_deg: 0.0,
    },
    Leg {
        name: "nile_suez",
        seconds: 6.0,
        from: [30.18, 31.55, REGIONAL_ALTITUDE_M],
        to: [30.98, 32.05, REGIONAL_ALTITUDE_M],
        pitch_deg: 0.0,
    },
    Leg {
        name: "alps_regional",
        seconds: 6.0,
        from: [46.2, 9.7, REGIONAL_ALTITUDE_M],
        to: [46.9, 10.54, REGIONAL_ALTITUDE_M],
        pitch_deg: 0.0,
    },
    Leg {
        name: "africa",
        seconds: 6.0,
        from: [-2.0, 26.0, 9_000_000.0],
        to: [3.0, 31.0, 9_000_000.0],
        pitch_deg: 0.0,
    },
];
const STATIONARY: [Leg; 1] = [Leg {
    name: "kerch_static",
    seconds: 16.0,
    from: KERCH,
    to: KERCH,
    pitch_deg: 0.0,
}];
const WARMUP_SECS: f64 = 5.0;

#[derive(Resource)]
struct Flight {
    route: &'static [Leg],
    measuring: bool,
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
        present_mode: PresentMode::AutoVsync,
        resizable: false,
        ..default()
    }
}

pub(crate) fn install(app: &mut App, gpu_terrain: bool, baseline: bool) {
    println!("terrain_flight pid {}", std::process::id());
    println!(
        "terrain_flight renderer={} resolution=1280x720 present=AutoVsync cap_hz=60",
        if baseline {
            "baseline"
        } else if gpu_terrain {
            "gpu"
        } else {
            "cpu_selection"
        },
    );
    let now = Instant::now();
    let route: &'static [Leg] = if std::env::args().any(|arg| arg == "--terrain-static") {
        &STATIONARY
    } else if std::env::args().any(|arg| arg == "--terrain-regional") {
        &REGIONAL
    } else {
        &FLIGHT
    };
    gpu_timing::install(app);
    app.insert_resource(WinitSettings::continuous())
        .add_plugins(RenderDiagnosticsPlugin)
        .insert_resource(Flight {
            route,
            measuring: false,
            begun: now,
            update_started: now,
            previous: None,
            leg: 0,
            samples: BTreeMap::new(),
            latest: BTreeMap::new(),
            captures: std::env::var_os("STRATA_FLIGHT_CAPTURES").map(PathBuf::from),
            captured: false,
        })
        .add_systems(First, start_update.after(crate::frame_pacing::pace))
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
    if let (Some(start), Some((index, _))) = (previous, leg_at(flight.route, elapsed))
        && index == flight.leg
        && elapsed >= WARMUP_SECS
    {
        flight.record("frame", (now - start).as_secs_f64() * 1000.0);
    }
}

fn leg_at(route: &[Leg], elapsed: f64) -> Option<(usize, f64)> {
    let mut position = (elapsed - WARMUP_SECS).max(0.0);
    for (index, leg) in route.iter().enumerate() {
        if position < leg.seconds {
            return Some((index, position / leg.seconds));
        }
        position -= leg.seconds;
    }
    None
}

fn pose(leg: &Leg, t: f64) -> [f64; 3] {
    let lerp = |axis: usize| (1.0 - t) * leg.from[axis] + leg.to[axis] * t;
    let altitude = ((1.0 - t) * leg.from[2].ln() + t * leg.to[2].ln()).exp();
    [lerp(0), lerp(1), altitude.max(20.0)]
}

fn fly(
    mut commands: Commands,
    mut camera: ResMut<GlobeCameraState>,
    mut flight: ResMut<Flight>,
    mut exit: MessageWriter<AppExit>,
) {
    let elapsed = (flight.update_started - flight.begun).as_secs_f64();
    let Some((index, t)) = leg_at(flight.route, elapsed) else {
        flight.report();
        exit.write(AppExit::Success);
        return;
    };
    if flight.leg != index {
        flight.report();
        flight.leg = index;
        flight.captured = false;
        flight.measuring = false;
    }
    let leg = &flight.route[index];
    if !flight.measuring && elapsed >= WARMUP_SECS {
        println!("terrain_flight begin {}", leg.name);
        flight.measuring = true;
    }
    let capture = t >= 0.5 && !flight.captured && flight.captures.is_some();
    let [lat, lon, altitude] = pose(leg, if capture { 0.5 } else { t });
    camera.focus_unit = lat_lon_to_unit(lat, lon);
    camera.heading_unit = initial_north_tangent(camera.focus_unit);
    camera.altitude_meters = altitude;
    camera.target_altitude_meters = altitude;
    camera.pitch_rad = leg.pitch_deg.to_radians();
    camera.zoom_anchor = None;
    if capture && let Some(dir) = &flight.captures {
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
            && let Some((leg, _)) = leg_at(flight.route, elapsed)
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
        let leg = self.route[self.leg].name;
        if self.measuring {
            println!("terrain_flight end {leg}");
        }
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
    fn regional_route_retraces_the_same_pose_at_map_scale() {
        let a = pose(&REGIONAL[0], 0.5);
        let b = pose(&REGIONAL[1], 0.5);
        for (forward, reverse) in a.into_iter().zip(b) {
            assert!((forward - reverse).abs() < 1e-9);
        }
        assert!((a[2] - REGIONAL_ALTITUDE_M).abs() < 1e-9);
    }

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
        assert!(leg_at(&FLIGHT, 100.0).is_none());
    }
}
