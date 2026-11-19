use std::time::Instant;

use strata_globe::{ElevationSampler, FaceUv, PlanetSpec, TileId, TileUv, face_uv_to_unit};
use strata_terrain::{
    BathymetryZone, DemGrid, Navigability, RiverDef, RiverId, RiverWaypoint, Salinity,
    TerrainConfig, TerrainError, TerrainWorld, WaterBodyKind, bathymetry_zone,
    is_deepwater_port_depth, lat_lon_to_unit,
};

#[test]
fn hydrology_invariants_rivers_lakes_and_bathymetry() {
    let world = TerrainWorld::earth_default();

    assert!(world.rivers().len() >= 15);
    assert!(world.lakes().len() >= 6);

    // Every Earth river waypoint sequence is monotonically non-increasing and samples as River(_).
    for river in world.rivers() {
        for pair in river.waypoints.windows(2) {
            assert!(
                pair[1].surface_elevation_m <= pair[0].surface_elevation_m,
                "river {} increased elevation downstream",
                river.name
            );
        }
        for wp in &river.waypoints {
            let water = world.sample_water(wp.unit_dir);
            let bed_elev = world.sample_elevation(wp.unit_dir);
            assert!(
                matches!(water.kind, WaterBodyKind::River(id) if id == river.id)
                    || matches!(water.kind, WaterBodyKind::Lake(_)),
                "river {} waypoint yielded {:?}",
                river.name,
                water.kind
            );
            assert!(
                water.depth_m > 0.0,
                "river {} depth must be > 0",
                river.name
            );
            assert!(
                bed_elev < water.surface_elevation_m,
                "river {} bed {bed_elev} must be below surface {}",
                river.name,
                water.surface_elevation_m
            );
        }
    }

    // Sampling lake centers yields Lake(_) with depth > 0 and accurate salinity.
    let mut saw_caspian = false;
    let mut saw_baikal = false;
    for lake in world.lakes() {
        let water = world.sample_water(lake.center_unit);
        let bed_elev = world.sample_elevation(lake.center_unit);
        assert_eq!(water.kind, WaterBodyKind::Lake(lake.id));
        assert!(water.depth_m > 0.0);
        assert!(bed_elev < water.surface_elevation_m);
        assert_eq!(water.is_freshwater, lake.salinity == Salinity::Freshwater);
        if lake.name == "Caspian Sea" {
            saw_caspian = true;
            assert_eq!(lake.salinity, Salinity::Saline);
            assert!(lake.surface_elevation_m < 0.0);
        }
        if lake.name == "Lake Baikal" {
            saw_baikal = true;
            assert_eq!(lake.salinity, Salinity::Freshwater);
            assert!(lake.max_depth_m > 1_500.0);
        }
    }
    assert!(saw_caspian && saw_baikal);

    // Bathymetry classification & port depth helper.
    assert_eq!(bathymetry_zone(-3_500.0), BathymetryZone::Abyssal);
    assert_eq!(bathymetry_zone(-800.0), BathymetryZone::ContinentalSlope);
    assert_eq!(bathymetry_zone(-45.0), BathymetryZone::ContinentalShelf);
    assert_eq!(bathymetry_zone(-4.0), BathymetryZone::Littoral);
    assert_eq!(bathymetry_zone(12.0), BathymetryZone::Land);
    assert!(is_deepwater_port_depth(-20.0));
    assert!(!is_deepwater_port_depth(-5.0));
    assert!(!is_deepwater_port_depth(-300.0));

    // RiverDef validation rejects non-monotonic rivers.
    let invalid = RiverDef::new(
        RiverId(99),
        "Broken",
        Navigability::Barge,
        vec![
            RiverWaypoint {
                unit_dir: lat_lon_to_unit(0.0, 0.0),
                surface_elevation_m: 100.0,
                half_width_m: 100.0,
                channel_depth_m: 5.0,
                valley_half_width_m: 1000.0,
                discharge_m3s: 100.0,
            },
            RiverWaypoint {
                unit_dir: lat_lon_to_unit(1.0, 0.0),
                surface_elevation_m: 120.0,
                half_width_m: 100.0,
                channel_depth_m: 5.0,
                valley_half_width_m: 1000.0,
                discharge_m3s: 100.0,
            },
        ],
    );
    assert!(matches!(
        invalid,
        Err(TerrainError::InvalidRiverWaypoints { .. })
    ));
}

#[test]
fn custom_dem_small_planet_and_roundtrip() -> Result<(), TerrainError> {
    let small_planet = PlanetSpec::new(200_000.0, 14, 16)?;

    // DEM round-trip on a small planet: +750 m above equator, -1500 m below equator.
    let w = 64_usize;
    let h = 32_usize;
    let mut grid_data = Vec::with_capacity(w * h);
    for y in 0..h {
        let val = if y < h / 2 { 750.0_f32 } else { -1500.0_f32 };
        for _ in 0..w {
            grid_data.push(val);
        }
    }
    let dem = DemGrid::new(w, h, grid_data)?;
    let dem_world = TerrainWorld::new(
        TerrainConfig::from_dem(dem, 7)
            .with_planet(small_planet)
            .with_macro_resolution(32),
    )?;
    assert_eq!(dem_world.planet(), small_planet);
    assert_eq!(dem_world.macro_resolution(), 32);

    let clamped = dem_world.global_range().clamp_planet_radius(small_planet);
    assert!(small_planet.radius_meters() + clamped.min_meters > 0.0);

    for tile in TileId::roots() {
        let range = dem_world.tile_range(small_planet, tile);
        for sy in 0..=4 {
            for sx in 0..=4 {
                let uv = TileUv::clamped(f64::from(sx) * 0.25, f64::from(sy) * 0.25);
                let unit = face_uv_to_unit(tile.face, tile.tile_uv_to_face_uv(uv));
                let elev = dem_world.sample_elevation(unit);
                assert!(elev >= range.min_meters && elev <= range.max_meters);
            }
        }
    }

    let north_sample = dem_world.sample_elevation(lat_lon_to_unit(45.0, 20.0));
    let south_sample = dem_world.sample_elevation(lat_lon_to_unit(-45.0, 20.0));
    assert!(
        (north_sample - 750.0).abs() < 250.0,
        "north DEM sample {north_sample} out of expected range"
    );
    assert!(
        (south_sample - (-1500.0)).abs() < 150.0,
        "south DEM sample {south_sample} out of expected range"
    );

    // Invalid configurations fail cleanly.
    assert!(matches!(
        TerrainWorld::new(TerrainConfig::earth(1).with_macro_resolution(48)),
        Err(TerrainError::InvalidMacroResolution { resolution: 48 })
    ));
    assert!(matches!(
        DemGrid::new(4, 4, vec![0.0; 10]),
        Err(TerrainError::InvalidDemDimensions { .. })
    ));
    assert!(matches!(
        DemGrid::new(2, 2, vec![0.0, f32::NAN, 1.0, 2.0]),
        Err(TerrainError::NonFiniteDemValue { index: 1 })
    ));

    Ok(())
}

#[test]
fn performance_and_determinism_sanity() {
    let start_build = Instant::now();
    let world_a = TerrainWorld::earth_default();
    let world_b = TerrainWorld::earth_default();
    let build_elapsed = start_build.elapsed();
    assert!(
        build_elapsed.as_secs_f64() < 5.0,
        "macro cache bake took too long: {build_elapsed:?}"
    );

    let start_sample = Instant::now();
    let mut checksum = 0.0_f64;
    for i in 0..50_000_u32 {
        let fi = f64::from(i);
        let u = (fi * 0.618_033_988_75).fract();
        let v = (fi * 0.381_966_011_25).fract();
        let face = strata_globe::CubeFace::ALL[(i % 6) as usize];
        let dir = face_uv_to_unit(face, FaceUv::clamped(u, v));
        let e = world_a.sample_elevation(dir);
        checksum += e;
        if i < 500 {
            assert_eq!(e.to_bits(), world_b.sample_elevation(dir).to_bits());
            let _ = world_a.sample_water(dir);
            let _ = world_a.sample_biome(dir);
            let _ = world_a.geology_province_id(dir);
        }
    }
    let sample_elapsed = start_sample.elapsed();
    assert!(checksum.is_finite());
    assert!(
        sample_elapsed.as_secs_f64() < 5.0,
        "50k queries took too long: {sample_elapsed:?}"
    );

    let planet = world_a.planet();
    let cam = strata_globe::CameraViewF64::look_at(
        lat_lon_to_unit(27.85, 86.75).scale(planet.radius_meters() + 18_000.0),
        lat_lon_to_unit(28.0, 86.9).scale(planet.radius_meters() + 4_500.0),
        strata_globe::EcefMeters::from_raw(0.0, 1.0, 0.0),
        60.0_f64.to_radians(),
        16.0 / 9.0,
        1080.0,
    );
    assert!(cam.is_ok());
    if let Ok(view) = cam {
        let mut split = std::collections::HashSet::new();
        let t_lod = Instant::now();
        for _ in 0..50 {
            let sel = strata_globe::select_tiles_with_elevation(
                planet,
                &view,
                strata_globe::LodConfig::standard(),
                &mut split,
                &world_a,
            );
            assert!(sel.is_ok());
            if let Ok(s) = sel {
                assert!(!s.tiles.is_empty());
            }
        }
        let lod_elapsed = t_lod.elapsed();
        assert!(
            lod_elapsed.as_secs_f64() < 5.0,
            "50 LOD selections took too long: {lod_elapsed:?}"
        );
    }
}
