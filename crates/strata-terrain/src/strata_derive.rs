use strata_globe::EcefMeters;

use crate::climate::{Biome, ClimateSample};
use crate::hydrology::{WaterBodyKind, WaterSample};
use crate::strata::{GeologyProvince, StrataColumn, SurfaceMaterial};
use crate::strata_layers::build_province_layers;
use crate::strata_minerals::{derive_mineral_potential, geo_var};

#[must_use]
pub(crate) fn derive_surface_material(
    elevation_m: f64,
    slope_sin: f64,
    water: WaterSample,
    climate: ClimateSample,
    biome: Biome,
) -> SurfaceMaterial {
    match water.kind {
        WaterBodyKind::Ocean | WaterBodyKind::Lake(_) => SurfaceMaterial::MarineSediment,
        WaterBodyKind::River(_) => SurfaceMaterial::AlluvialSilt,
        WaterBodyKind::Dry => {
            if biome == Biome::PolarIce {
                SurfaceMaterial::IceSheet
            } else if slope_sin > 0.44 || biome == Biome::AlpineRock {
                SurfaceMaterial::ExposedRock
            } else if climate.permafrost_prob >= 0.40 || climate.mean_temp_c <= -2.5 {
                SurfaceMaterial::Permafrost
            } else if biome == Biome::CoastalWetland {
                SurfaceMaterial::PeatWetland
            } else if matches!(biome, Biome::HotDesert | Biome::ColdDesert) {
                SurfaceMaterial::Sand
            } else if matches!(biome, Biome::Tundra | Biome::AlpineTundra) {
                SurfaceMaterial::GlacialTill
            } else if elevation_m <= 35.0 && slope_sin < 0.06 && climate.annual_precip_mm >= 650.0 {
                SurfaceMaterial::AlluvialSilt
            } else {
                SurfaceMaterial::LoamSoil
            }
        }
    }
}

#[expect(
    clippy::too_many_arguments,
    clippy::cast_possible_truncation,
    reason = "geological derivation takes all physical-geographic context and narrows outputs to compact f32"
)]
#[must_use]
pub(crate) fn derive_strata_column(
    unit_dir: EcefMeters,
    elevation_m: f64,
    slope_sin: f64,
    water: WaterSample,
    climate: ClimateSample,
    biome: Biome,
    province: GeologyProvince,
    seed: u64,
) -> StrataColumn {
    let surface_material = derive_surface_material(elevation_m, slope_sin, water, climate, biome);

    let v_ob = geo_var(unit_dir, 18.0, seed, 1_103);
    let v_depth = geo_var(unit_dir, 9.5, seed, 2_207);
    let v_lith = geo_var(unit_dir, 14.0, seed, 3_313);

    let slope_factor = (1.0 - slope_sin * 1.6).clamp(0.08, 1.0);
    let base_ob = match province {
        GeologyProvince::SedimentaryBasin => 24.0 + 36.0 * v_ob,
        GeologyProvince::ContinentalShelf => 18.0 + 30.0 * v_ob,
        GeologyProvince::CratonShield => 4.0 + 12.0 * v_ob,
        GeologyProvince::YoungOrogen => 2.5 + 11.0 * v_ob,
        GeologyProvince::OldOrogenRift => 7.0 + 18.0 * v_ob,
        GeologyProvince::SubductionArc => 5.0 + 15.0 * v_ob,
        GeologyProvince::PolarShield => 10.0 + 24.0 * v_ob,
        GeologyProvince::OceanicAbyss => 14.0 + 28.0 * v_ob,
    };
    let overburden_thickness_m = if surface_material == SurfaceMaterial::ExposedRock {
        (0.4 + 1.4 * v_ob) as f32
    } else {
        (base_ob * slope_factor) as f32
    };

    let permafrost_depth_m = if water.kind != WaterBodyKind::Dry {
        0.0
    } else if climate.mean_temp_c < -1.0 || climate.permafrost_prob > 0.15 {
        let coldness = (-f64::from(climate.mean_temp_c)).max(0.0);
        let depth =
            coldness * 22.0 + f64::from(climate.permafrost_prob) * 90.0 * (0.75 + 0.5 * v_ob);
        depth.clamp(0.0, 900.0) as f32
    } else {
        0.0
    };

    let water_table_depth_m = if water.kind == WaterBodyKind::Dry {
        let arid = f64::from(climate.aridity).clamp(0.2, 12.0);
        let relief_drop = slope_sin * 38.0 + (elevation_m.max(0.0) / 2500.0) * 18.0;
        let depth = 3.0 + arid * 6.5 + relief_drop + v_depth * 14.0;
        depth.clamp(1.0, 260.0) as f32
    } else {
        0.0
    };

    let mineral_potential = derive_mineral_potential(unit_dir, province, biome, v_lith, seed);
    let (layers, layer_count) = build_province_layers(province, v_depth, v_lith);

    StrataColumn::new(
        province,
        surface_material,
        [
            overburden_thickness_m,
            permafrost_depth_m,
            water_table_depth_m,
        ],
        mineral_potential,
        layers,
        layer_count,
    )
}
