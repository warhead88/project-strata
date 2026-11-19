use strata_globe::{EcefMeters, ElevationSampler, PlanetSpec, TileId, TileUv, face_uv_to_unit};
use strata_terrain::{
    BedrockKind, DemGrid, GeographicTags, GeologyProvince, MAX_STRATA_LAYERS, PatchId,
    SurfaceMaterial, TerrainConfig, TerrainError, TerrainPatchKind, TerrainWorld, lat_lon_to_unit,
};

#[test]
fn strata_column_determinism_coverage_and_province_contrast() {
    let world = TerrainWorld::earth_default();

    let craton_pt = lat_lon_to_unit(63.0, 16.5); // Scandinavia craton
    let basin_pt = lat_lon_to_unit(50.0, 13.0); // Central Europe sedimentary basin
    let polar_pt = lat_lon_to_unit(-83.0, 45.0); // East Antarctica polar shield
    let congo_pt = lat_lon_to_unit(0.0, 22.0); // Equatorial Congo

    let col_a = world.sample_strata_column(craton_pt);
    let col_b = world.sample_strata_column(craton_pt);
    assert_eq!(col_a, col_b);

    for pt in [craton_pt, basin_pt, polar_pt, congo_pt] {
        let col = world.sample_strata_column(pt);
        let layers = col.layers();
        assert!(!layers.is_empty() && layers.len() <= MAX_STRATA_LAYERS);
        assert!((layers[0].top_depth_m - 0.0).abs() < 1e-5);
        for pair in layers.windows(2) {
            assert!((pair[0].bottom_depth_m - pair[1].top_depth_m).abs() < 1e-4);
        }
        assert!((layers[layers.len() - 1].bottom_depth_m - 10_000.0).abs() < 1e-3);

        let suit = col.underground_suitability(80.0);
        let cost = col.excavation_cost_multiplier(80.0);
        assert!((0.0..=1.0).contains(&suit));
        assert!(cost.is_finite() && cost >= 0.7);
        assert!(col.ambient_rock_temp_c(10.0, 2_000.0) > col.ambient_rock_temp_c(10.0, 100.0));
    }

    let craton = world.sample_strata_column(craton_pt);
    let basin = world.sample_strata_column(basin_pt);
    assert_eq!(craton.province, GeologyProvince::CratonShield);
    assert_eq!(basin.province, GeologyProvince::SedimentaryBasin);
    assert!(craton.layers()[1].hardness > basin.layers()[1].hardness);
    assert!(craton.layers()[1].structural_stability > basin.layers()[1].structural_stability);
    assert!(basin.layers()[0].permeability > craton.layers()[1].permeability);
    assert!(basin.mineral_potential.hydrocarbons > craton.mineral_potential.hydrocarbons);
    assert!(craton.mineral_potential.iron > basin.mineral_potential.iron);

    let polar = world.sample_strata_column(polar_pt);
    let congo = world.sample_strata_column(congo_pt);
    assert_eq!(polar.province, GeologyProvince::PolarShield);
    assert_eq!(polar.surface_material, SurfaceMaterial::IceSheet);
    assert!(polar.permafrost_depth_m > 0.0);
    assert!((congo.permafrost_depth_m - 0.0).abs() < 1e-6);

    let surf_elev = world.sample_elevation(craton_pt);
    let stratum_500 = world.sample_stratum_at_elevation(craton_pt, surf_elev - 500.0);
    assert_eq!(stratum_500, *craton.layer_at_depth(500.0));
    assert!(matches!(
        stratum_500.bedrock,
        BedrockKind::Granite | BedrockKind::Metamorphic
    ));
}

#[test]
fn patch_layer_surface_and_voids() -> Result<(), TerrainError> {
    let mut world = TerrainWorld::earth_default();
    assert_eq!(world.revision(), 0);

    // 1. FlattenPad on dry European plains.
    let pad_center = lat_lon_to_unit(49.0, 12.0);
    let orig_pad_elev = world.sample_elevation(pad_center);
    let target_elev = orig_pad_elev + 150.0;
    let pad_receipt = world.apply_patch(TerrainPatchKind::FlattenPad {
        center_unit: pad_center,
        radius_m: 1_500.0,
        blend_width_m: 1_000.0,
        target_elevation_m: target_elev,
    })?;
    assert_eq!(world.revision(), 1);
    assert!(!pad_receipt.affected_tiles.is_empty());
    assert!((world.sample_elevation(pad_center) - target_elev).abs() < 1e-5);

    // 2. CorridorGrade linear elevation interpolation.
    let corr_start = lat_lon_to_unit(47.0, 15.0);
    let corr_end = lat_lon_to_unit(47.08, 15.0);
    let corr_mid = (corr_start + corr_end).normalized()?;
    world.apply_patch(TerrainPatchKind::CorridorGrade {
        start_unit: corr_start,
        end_unit: corr_end,
        half_width_m: 600.0,
        blend_width_m: 400.0,
        start_elevation_m: 300.0,
        end_elevation_m: 500.0,
    })?;
    assert!((world.sample_elevation(corr_start) - 300.0).abs() < 1e-4);
    assert!((world.sample_elevation(corr_mid) - 400.0).abs() < 1.0);
    assert!((world.sample_elevation(corr_end) - 500.0).abs() < 1e-4);

    // 3. ExcavationPit depresses center by depth_m.
    let pit_center = lat_lon_to_unit(51.0, 20.0);
    let orig_pit_elev = world.sample_elevation(pit_center);
    let pit_receipt = world.apply_patch(TerrainPatchKind::ExcavationPit {
        center_unit: pit_center,
        radius_m: 800.0,
        blend_width_m: 500.0,
        depth_m: 120.0,
    })?;
    assert!((world.sample_elevation(pit_center) - (orig_pit_elev - 120.0)).abs() < 1e-5);

    // 4. UndergroundVoid without portal keeps surface bit-identical & queries depth/radius.
    let void_center = lat_lon_to_unit(53.0, 18.0);
    let before_void_elev = world.sample_elevation(void_center);
    let void_receipt = world.apply_patch(TerrainPatchKind::UndergroundVoid {
        center_unit: void_center,
        radius_m: 1_200.0,
        top_depth_m: 80.0,
        bottom_depth_m: 220.0,
        portal_surface_unit: None,
    })?;
    assert_eq!(
        world.sample_elevation(void_center).to_bits(),
        before_void_elev.to_bits()
    );
    assert_eq!(
        world.has_underground_void_at(void_center, 150.0),
        Some(void_receipt.id)
    );
    assert_eq!(world.has_underground_void_at(void_center, 40.0), None);
    assert_eq!(world.has_underground_void_at(void_center, 260.0), None);
    assert!(world.has_any_underground_void(void_center));
    assert!(world.sample_terrain(void_center).has_underground_void);
    assert_eq!(world.patches().len(), 4);

    // UndergroundVoid with portal grades a small ~25m portal pad.
    let portal_u = lat_lon_to_unit(53.05, 18.0);
    let portal_orig = world.sample_elevation(portal_u);
    let portal_void = world.apply_patch(TerrainPatchKind::UndergroundVoid {
        center_unit: lat_lon_to_unit(53.04, 18.0),
        radius_m: 600.0,
        top_depth_m: 50.0,
        bottom_depth_m: 150.0,
        portal_surface_unit: Some(portal_u),
    })?;
    assert!((world.sample_elevation(portal_u) - portal_orig).abs() < 1e-6);
    world.remove_patch(portal_void.id)?;

    // Removal restores exact bit-identical baseline and bumps revision.
    let rev_before = world.revision();
    let rm_pad = world.remove_patch(pad_receipt.id)?;
    assert_eq!(rm_pad.affected_tiles, pad_receipt.affected_tiles);
    assert_eq!(world.revision(), rev_before + 1);
    assert_eq!(
        world.sample_elevation(pad_center).to_bits(),
        orig_pad_elev.to_bits()
    );
    world.remove_patch(pit_receipt.id)?;
    assert_eq!(
        world.sample_elevation(pit_center).to_bits(),
        orig_pit_elev.to_bits()
    );
    world.remove_patch(void_receipt.id)?;
    assert_eq!(world.has_underground_void_at(void_center, 150.0), None);
    Ok(())
}

#[test]
fn patch_layer_seam_bounds_and_removal() -> Result<(), TerrainError> {
    let mut world = TerrainWorld::earth_default();
    let planet = world.planet();
    // 5. Cube-face seam patch: verify conservative tile_range on both sides of +X/+Z seam!
    let seam_unit = EcefMeters::from_raw(1.0, 0.2, 1.0).normalized()?;
    let seam_orig = world.sample_elevation(seam_unit);
    let seam_receipt = world.apply_patch(TerrainPatchKind::FlattenPad {
        center_unit: seam_unit,
        radius_m: 2_500.0,
        blend_width_m: 1_500.0,
        target_elevation_m: 9_600.0,
    })?;
    assert!(seam_receipt.affected_tiles.len() >= 2);
    for &tile5 in &seam_receipt.affected_tiles {
        let mut cur = Some(tile5);
        while let Some(tile) = cur {
            let range = world.tile_range(planet, tile);
            for sy in 0..=4 {
                for sx in 0..=4 {
                    let uv = TileUv::clamped(f64::from(sx) * 0.25, f64::from(sy) * 0.25);
                    let u = face_uv_to_unit(tile.face, tile.tile_uv_to_face_uv(uv));
                    let e = world.sample_elevation(u);
                    assert!(
                        e >= range.min_meters && e <= range.max_meters,
                        "tile {tile:?} range [{}, {}] failed for elev {e}",
                        range.min_meters,
                        range.max_meters
                    );
                }
            }
            cur = tile.parent();
        }
        // Check a child tile at LOD 8 inside tile5.
        let child = TileId {
            face: tile5.face,
            lod: 8,
            x: tile5.x << 3,
            y: tile5.y << 3,
        };
        let c_range = world.tile_range(planet, child);
        let c_unit = face_uv_to_unit(
            child.face,
            child.tile_uv_to_face_uv(TileUv::clamped(0.5, 0.5)),
        );
        let c_elev = world.sample_elevation(c_unit);
        assert!(c_elev >= c_range.min_meters && c_elev <= c_range.max_meters);
    }

    // Removal restores exact bit-identical baseline and bumps revision.
    let rev_before = world.revision();
    let rm_seam = world.remove_patch(seam_receipt.id)?;
    assert_eq!(rm_seam.affected_tiles, seam_receipt.affected_tiles);
    assert_eq!(world.revision(), rev_before + 1);
    assert_eq!(
        world.sample_elevation(seam_unit).to_bits(),
        seam_orig.to_bits()
    );
    Ok(())
}

#[test]
fn patch_and_site_validation_errors() {
    let mut world = TerrainWorld::earth_default();
    let valid_u = lat_lon_to_unit(10.0, 20.0);

    assert!(matches!(
        world.apply_patch(TerrainPatchKind::FlattenPad {
            center_unit: EcefMeters::ZERO,
            radius_m: 100.0,
            blend_width_m: 50.0,
            target_elevation_m: 0.0,
        }),
        Err(TerrainError::Globe(_))
    ));
    assert!(matches!(
        world.apply_patch(TerrainPatchKind::FlattenPad {
            center_unit: valid_u,
            radius_m: -5.0,
            blend_width_m: 50.0,
            target_elevation_m: 0.0,
        }),
        Err(TerrainError::InvalidPatch { .. })
    ));
    assert!(matches!(
        world.apply_patch(TerrainPatchKind::CorridorGrade {
            start_unit: valid_u,
            end_unit: valid_u,
            half_width_m: 50.0,
            blend_width_m: 20.0,
            start_elevation_m: 0.0,
            end_elevation_m: 10.0,
        }),
        Err(TerrainError::InvalidPatch { .. })
    ));
    assert!(matches!(
        world.apply_patch(TerrainPatchKind::ExcavationPit {
            center_unit: valid_u,
            radius_m: 50.0,
            blend_width_m: 20.0,
            depth_m: 0.0,
        }),
        Err(TerrainError::InvalidPatch { .. })
    ));
    assert!(matches!(
        world.apply_patch(TerrainPatchKind::UndergroundVoid {
            center_unit: valid_u,
            radius_m: 50.0,
            top_depth_m: 100.0,
            bottom_depth_m: 80.0,
            portal_surface_unit: None,
        }),
        Err(TerrainError::InvalidPatch { .. })
    ));
    assert!(matches!(
        world.remove_patch(PatchId(999_999)),
        Err(TerrainError::PatchNotFound {
            id: PatchId(999_999)
        })
    ));
    assert!(matches!(
        world.assess_site(valid_u, 0.0),
        Err(TerrainError::InvalidSiteRadius { .. })
    ));
    assert!(matches!(
        world.assess_site(valid_u, f64::NAN),
        Err(TerrainError::InvalidSiteRadius { .. })
    ));
}

#[test]
fn terrain_sample_and_site_assessment_across_environments() -> Result<(), TerrainError> {
    let world = TerrainWorld::earth_default();

    // Himalaya -> HIGHLAND.
    let himalaya = world.assess_site(lat_lon_to_unit(28.5, 86.9), 12_000.0)?;
    assert!(himalaya.geographic_tags.contains(GeographicTags::HIGHLAND));
    assert!(himalaya.mean_elevation_m > 2_500.0);

    // Sahara -> DESERT, no freshwater.
    let sahara = world.assess_site(lat_lon_to_unit(23.0, 13.0), 10_000.0)?;
    assert!(sahara.geographic_tags.contains(GeographicTags::DESERT));
    assert!(!sahara.has_freshwater);
    assert!(sahara.dry_land_fraction > 0.9);

    // Antarctica -> ANTARCTIC + GLACIATED + PERMAFROST.
    let antarctica = world.assess_site(lat_lon_to_unit(-83.0, 45.0), 10_000.0)?;
    assert!(
        antarctica
            .geographic_tags
            .contains(GeographicTags::ANTARCTIC)
    );
    assert!(
        antarctica
            .geographic_tags
            .contains(GeographicTags::GLACIATED)
    );
    assert!(
        antarctica
            .geographic_tags
            .contains(GeographicTags::PERMAFROST)
    );

    // River site (Amazon mid-course) -> RIVERINE + freshwater + navigable river.
    let amazon_river = world.assess_site(lat_lon_to_unit(-3.1, -60.0), 10_000.0)?;
    assert!(
        amazon_river
            .geographic_tags
            .contains(GeographicTags::RIVERINE)
    );
    assert!(amazon_river.has_freshwater);
    assert!(amazon_river.has_navigable_river);

    // Deep Pacific Ocean -> zero dry or buildable land.
    let pacific = world.assess_site(lat_lon_to_unit(0.0, -140.0), 10_000.0)?;
    assert!(pacific.dry_land_fraction < 1e-6);
    assert!(pacific.buildable_land_fraction < 1e-6);
    assert!(pacific.flat_airfield_fraction < 1e-6);

    // Coastal deepwater port check using a small smooth coastal DEM ramp across the equator.
    let (w, h) = (64_usize, 32_usize);
    let mut grid = Vec::with_capacity(w * h);
    for y in 0..h {
        let val = if y < h / 2 { 40.0_f32 } else { -80.0_f32 };
        grid.extend(std::iter::repeat_n(val, w));
    }
    let coast_world = TerrainWorld::new(
        TerrainConfig::from_dem(DemGrid::new(w, h, grid)?, 7)
            .with_planet(PlanetSpec::new(200_000.0, 14, 16)?)
            .with_macro_resolution(32),
    )?;
    let port_site = coast_world.assess_site(lat_lon_to_unit(0.0, 0.0), 12_000.0)?;
    assert!(port_site.dry_land_fraction > 0.0 && port_site.dry_land_fraction < 1.0);
    assert!(port_site.has_deepwater_port_access);
    assert!(port_site.geographic_tags.contains(GeographicTags::COASTAL));
    assert!(
        port_site
            .geographic_tags
            .contains(GeographicTags::DEEPWATER_PORT)
    );

    let rv = world.sample_render_vertex(lat_lon_to_unit(49.0, 10.0), 0.1);
    assert!(rv.elevation_m.is_finite());
    assert!(rv.albedo_rgb.iter().all(|c| (0.0..=1.0).contains(c)));
    Ok(())
}
