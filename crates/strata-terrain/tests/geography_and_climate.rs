use strata_globe::ElevationSampler;
use strata_terrain::{
    BathymetryZone, Biome, TerrainWorld, WaterBodyKind, WaterSample, lat_lon_to_unit,
    surface_albedo_rgb,
};

#[test]
fn earth_geography_and_climate_sanity() {
    let world = TerrainWorld::earth_default();

    // Poles vs Equator.
    let north_pole = lat_lon_to_unit(90.0, 0.0);
    let south_pole = lat_lon_to_unit(-90.0, 0.0);
    let equator_atlantic = lat_lon_to_unit(0.0, -25.0);

    let np_clim = world.sample_climate(north_pole);
    let sp_clim = world.sample_climate(south_pole);
    let eq_clim = world.sample_climate(equator_atlantic);

    assert!(
        np_clim.mean_temp_c < eq_clim.mean_temp_c - 30.0,
        "north pole {} C should be much colder than equator {} C",
        np_clim.mean_temp_c,
        eq_clim.mean_temp_c
    );
    assert!(
        sp_clim.mean_temp_c < eq_clim.mean_temp_c - 40.0,
        "south pole {} C should be much colder than equator {} C",
        sp_clim.mean_temp_c,
        eq_clim.mean_temp_c
    );
    assert!(matches!(
        world.sample_biome(north_pole),
        Biome::PolarIce | Biome::Tundra
    ));
    assert!(matches!(
        world.sample_biome(south_pole),
        Biome::PolarIce | Biome::Tundra
    ));

    // Himalayas / Tibet highland (> 3000 m).
    let himalaya = lat_lon_to_unit(28.5, 86.9);
    let tibet = lat_lon_to_unit(32.5, 88.0);
    assert!(
        world.sample_elevation(himalaya) > 3_000.0,
        "Himalaya elevation {} m should be > 3000 m",
        world.sample_elevation(himalaya)
    );
    assert!(
        world.sample_elevation(tibet) > 3_000.0,
        "Tibet elevation {} m should be > 3000 m",
        world.sample_elevation(tibet)
    );

    // Sahara hot & arid desert.
    let sahara = lat_lon_to_unit(23.0, 13.0);
    let sahara_clim = world.sample_climate(sahara);
    assert!(world.sample_elevation(sahara) > 0.0);
    assert!(
        sahara_clim.mean_temp_c > 22.0,
        "Sahara temp {} C should be hot",
        sahara_clim.mean_temp_c
    );
    assert!(
        sahara_clim.annual_precip_mm < 180.0,
        "Sahara precip {} mm should be arid",
        sahara_clim.annual_precip_mm
    );
    assert_eq!(world.sample_biome(sahara), Biome::HotDesert);

    // Amazon & Congo warm & humid rainforest.
    let amazon = lat_lon_to_unit(-3.5, -61.0);
    let congo = lat_lon_to_unit(0.0, 22.0);
    for (name, pt) in [("Amazon", amazon), ("Congo", congo)] {
        let elev = world.sample_elevation(pt);
        let clim = world.sample_climate(pt);
        assert!(elev > 0.0, "{name} should be above sea level, got {elev}");
        assert!(clim.mean_temp_c > 21.0, "{name} temp {}", clim.mean_temp_c);
        assert!(
            clim.annual_precip_mm > 1_500.0,
            "{name} precip {} mm should be humid",
            clim.annual_precip_mm
        );
        assert_eq!(world.sample_biome(pt), Biome::TropicalRainforest);
    }

    // Pacific deep abyssal ocean (< -2000 m).
    let pacific = lat_lon_to_unit(0.0, -140.0);
    let pac_elev = world.sample_elevation(pacific);
    assert!(
        pac_elev < -2_000.0,
        "Pacific elevation {pac_elev} m should be < -2000 m"
    );
    assert_eq!(world.bathymetry_zone(pac_elev), BathymetryZone::Abyssal);
    assert_eq!(world.sample_biome(pacific), Biome::OpenWater);
}

#[test]
fn earth_real_dem_landmarks_sanity() {
    let world = TerrainWorld::earth_default();

    // Recognizable continents, mountain belts, islands, seas, and ocean trenches (Real Earth DEM).
    for (name, lat, lon, min_m, max_m) in [
        ("Himalayas/Everest", 28.0, 86.9, 4_000.0, 9_000.0),
        ("Alps", 46.5, 8.0, 1_200.0, 5_000.0),
        ("Andes", -15.0, -70.0, 3_000.0, 7_500.0),
        ("Moscow", 55.75, 37.6, 50.0, 500.0),
        ("Sahara", 23.0, 10.0, 100.0, 3_500.0),
        ("Amazon", -3.0, -60.0, 0.0, 500.0),
        ("UK", 52.5, -1.5, 0.0, 1_500.0),
        ("Italy", 42.5, 12.5, 0.0, 3_000.0),
        ("Japan", 36.0, 138.0, 0.0, 4_000.0),
        ("Australia", -25.0, 134.0, 0.0, 2_000.0),
        ("Antarctica", -89.0, 0.0, 1_500.0, 5_000.0),
        ("Greenland", 72.5, -42.0, 1_500.0, 4_000.0),
        ("North America", 42.0, -98.0, 100.0, 2_000.0),
        ("India", 21.0, 78.5, 50.0, 2_000.0),
        ("Mariana Trench", 11.3, 142.2, -11_500.0, -6_000.0),
        ("Mediterranean", 35.5, 18.0, -6_000.0, -1_000.0),
        ("Black Sea", 43.3, 34.0, -3_000.0, -500.0),
        ("Red Sea", 20.5, 38.5, -3_500.0, -300.0),
        ("Pacific", 0.0, -140.0, -6_500.0, -3_000.0),
        ("Atlantic", 25.0, -45.0, -6_500.0, -2_000.0),
    ] {
        let unit = lat_lon_to_unit(lat, lon);
        let lat_rt = unit.y.clamp(-1.0, 1.0).asin().to_degrees();
        let lon_rt = unit.x.atan2(unit.z).to_degrees();
        assert!((lat_rt - lat).abs() < 1e-9, "{name} lat roundtrip failed");
        if lat.abs() < 89.9 {
            assert!((lon_rt - lon).abs() < 1e-9, "{name} lon roundtrip failed");
        }
        let elev = world.sample_elevation(unit);
        assert!(
            elev > min_m && elev < max_m,
            "{name} ({lat}, {lon}) expected in ({min_m}, {max_m}), got {elev} m"
        );
    }
}

#[test]
fn all_biomes_produce_valid_albedo_colors() {
    let world = TerrainWorld::earth_default();
    let dry_water = WaterSample {
        kind: WaterBodyKind::Dry,
        surface_elevation_m: 100.0,
        depth_m: 0.0,
        discharge_m3s: 0.0,
        navigability: strata_terrain::Navigability::Unnavigable,
        is_freshwater: true,
    };
    let ocean_water = WaterSample {
        kind: WaterBodyKind::Ocean,
        surface_elevation_m: 0.0,
        depth_m: 3500.0,
        discharge_m3s: 0.0,
        navigability: strata_terrain::Navigability::Unnavigable,
        is_freshwater: false,
    };
    let climate = world.sample_climate(lat_lon_to_unit(20.0, 0.0));

    let biomes = [
        Biome::PolarIce,
        Biome::Tundra,
        Biome::BorealForest,
        Biome::TemperateForest,
        Biome::TemperateGrassland,
        Biome::Steppe,
        Biome::HotDesert,
        Biome::ColdDesert,
        Biome::Savanna,
        Biome::TropicalRainforest,
        Biome::AlpineTundra,
        Biome::AlpineRock,
        Biome::CoastalWetland,
        Biome::OpenWater,
    ];
    for biome in biomes {
        let w = if biome == Biome::OpenWater {
            ocean_water
        } else {
            dry_water
        };
        let rgb = surface_albedo_rgb(100.0, 0.1, w, climate, biome);
        for c in rgb {
            assert!(c.is_finite() && (0.0..=1.0).contains(&c));
        }
    }

    let sample_rgb = world.sample_surface_rgb(lat_lon_to_unit(47.0, 10.0), 0.25);
    for c in sample_rgb {
        assert!(c.is_finite() && (0.0..=1.0).contains(&c));
    }
}
