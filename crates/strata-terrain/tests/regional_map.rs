use std::collections::VecDeque;

use strata_globe::{ElevationSampler, ecef_to_tile_uv};
use strata_terrain::{TerrainWorld, WaterBodyKind, lat_lon_to_unit};

#[test]
fn sampled_sea_stays_connected_through_narrow_straits() {
    let world = TerrainWorld::earth_default();
    assert!(wet_path(
        &world,
        [45.03, 36.31],
        [100, 130],
        [4, 56],
        [94, 78],
        0.005,
        true
    ));
    assert!(wet_path(
        &world,
        [35.87, -5.79],
        [40, 140],
        [21, 8],
        [21, 130],
        0.005,
        true
    ));
}

#[test]
fn baked_canal_path_connects_the_red_sea_and_the_mediterranean() {
    let world = TerrainWorld::earth_default();
    assert!(wet_path(
        &world,
        [29.85, 32.17],
        [510, 210],
        [18, 125],
        [500, 56],
        0.003,
        false
    ));
}

#[test]
fn lake_shore_holes_leave_islands_above_water() {
    let world = TerrainWorld::earth_default();
    let olkhon = world.sample_water(lat_lon_to_unit(53.15, 107.32));
    let west_bank = world.sample_water(lat_lon_to_unit(53.17, 107.125));
    let east_bank = world.sample_water(lat_lon_to_unit(53.12, 107.63));
    assert_eq!(olkhon.kind, WaterBodyKind::Dry);
    assert!(matches!(west_bank.kind, WaterBodyKind::Lake(_)));
    assert_eq!(west_bank.kind, east_bank.kind);
    assert!((west_bank.surface_elevation_m - east_bank.surface_elevation_m).abs() < 0.001);
}

#[test]
fn bounds_cover_baked_channel_beds_and_regional_peaks() -> Result<(), strata_globe::GlobeError> {
    let world = TerrainWorld::earth_default();
    for [latitude, longitude] in [
        [30.05, 31.23],
        [53.20, 107.30],
        [46.54, 10.12],
        [45.29, 36.52],
    ] {
        for offset in -40..=40 {
            let direction = lat_lon_to_unit(latitude, longitude + f64::from(offset) * 0.002);
            let (tile, _, _) = ecef_to_tile_uv(world.planet(), direction, 10)?;
            let range = world.tile_range(world.planet(), tile);
            let elevation = world.sample_elevation(direction);
            assert!(elevation >= range.min_meters && elevation <= range.max_meters);
        }
    }
    Ok(())
}

// Four-connected search through actual point samples. The small grid around
// each strait excludes alternative routes between the two seas.
fn wet_path(
    world: &TerrainWorld,
    origin: [f64; 2],
    [rows, cols]: [u16; 2],
    start: [u16; 2],
    end: [u16; 2],
    step_deg: f64,
    ocean_only: bool,
) -> bool {
    let mut queue = VecDeque::from([start]);
    let mut seen = vec![false; usize::from(rows) * usize::from(cols)];
    while let Some([row, col]) = queue.pop_front() {
        if row >= rows || col >= cols {
            continue;
        }
        let index = usize::from(row) * usize::from(cols) + usize::from(col);
        if seen[index] {
            continue;
        }
        seen[index] = true;
        let unit = lat_lon_to_unit(
            origin[0] + f64::from(row) * step_deg,
            origin[1] + f64::from(col) * step_deg,
        );
        let kind = world.sample_water(unit).kind;
        if kind == WaterBodyKind::Dry || (ocean_only && kind != WaterBodyKind::Ocean) {
            continue;
        }
        if [row, col] == end {
            return true;
        }
        queue.extend([
            [row.wrapping_sub(1), col],
            [row + 1, col],
            [row, col.wrapping_sub(1)],
            [row, col + 1],
        ]);
    }
    false
}
