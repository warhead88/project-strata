use strata_globe::EcefMeters;

use crate::climate::Biome;
use crate::noise::noise3;
use crate::strata::{GeologyProvince, MineralPotential};

/// Evaluates deterministic, seam-continuous 3D geological variation in `[0.0, 1.0]`.
#[inline]
pub(crate) fn geo_var(unit_dir: EcefMeters, freq: f64, seed: u64, salt: u64) -> f64 {
    let n = noise3(
        unit_dir.x * freq,
        unit_dir.y * freq,
        unit_dir.z * freq,
        seed.wrapping_add(salt),
    );
    (0.5 + 0.5 * n).clamp(0.0, 1.0)
}

#[expect(
    clippy::cast_possible_truncation,
    reason = "mineral potentials are normalized [0, 1] f32 values"
)]
pub(crate) fn derive_mineral_potential(
    unit_dir: EcefMeters,
    province: GeologyProvince,
    biome: Biome,
    v_lith: f64,
    seed: u64,
) -> MineralPotential {
    let n1 = geo_var(unit_dir, 11.0, seed, 4_111);
    let n2 = geo_var(unit_dir, 17.0, seed, 5_227);
    let n3 = geo_var(unit_dir, 23.0, seed, 6_337);
    let n4 = geo_var(unit_dir, 13.0, seed, 7_451);

    let (iron, coal, hc, cu, li, ree, u, lime, geo) =
        province_mineral_tuple(province, n1, n2, n3, n4, v_lith);

    let desert_li_boost = if matches!(biome, Biome::HotDesert | Biome::ColdDesert) {
        0.16
    } else {
        0.0
    };

    MineralPotential {
        iron: iron.clamp(0.0, 1.0) as f32,
        coal: coal.clamp(0.0, 1.0) as f32,
        hydrocarbons: hc.clamp(0.0, 1.0) as f32,
        copper: cu.clamp(0.0, 1.0) as f32,
        lithium: (li + desert_li_boost).clamp(0.0, 1.0) as f32,
        rare_earths: ree.clamp(0.0, 1.0) as f32,
        uranium: u.clamp(0.0, 1.0) as f32,
        limestone: lime.clamp(0.0, 1.0) as f32,
        geothermal: geo.clamp(0.0, 1.0) as f32,
    }
}

fn province_mineral_tuple(
    province: GeologyProvince,
    n1: f64,
    n2: f64,
    n3: f64,
    n4: f64,
    v_lith: f64,
) -> (f64, f64, f64, f64, f64, f64, f64, f64, f64) {
    match province {
        GeologyProvince::CratonShield => (
            0.55 + 0.42 * n1,
            0.05 + 0.12 * n2,
            0.04 + 0.10 * n3,
            0.25 + 0.35 * n4,
            0.18 + 0.30 * v_lith,
            0.42 + 0.45 * n2,
            0.45 + 0.45 * n3,
            0.12 + 0.18 * n1,
            0.10 + 0.15 * n4,
        ),
        GeologyProvince::SedimentaryBasin => (
            0.18 + 0.25 * n1,
            0.48 + 0.46 * n2,
            0.52 + 0.44 * n3,
            0.10 + 0.18 * n4,
            0.25 + 0.38 * v_lith,
            0.10 + 0.18 * n2,
            0.18 + 0.28 * n3,
            0.52 + 0.42 * n1,
            0.18 + 0.22 * n4,
        ),
        GeologyProvince::YoungOrogen => (
            0.28 + 0.35 * n1,
            0.12 + 0.20 * n2,
            0.10 + 0.18 * n3,
            0.52 + 0.44 * n4,
            0.42 + 0.46 * v_lith,
            0.35 + 0.42 * n2,
            0.22 + 0.30 * n3,
            0.36 + 0.36 * n1,
            0.35 + 0.40 * n4,
        ),
        GeologyProvince::OldOrogenRift => (
            0.44 + 0.42 * n1,
            0.36 + 0.38 * n2,
            0.16 + 0.24 * n3,
            0.38 + 0.38 * n4,
            0.28 + 0.34 * v_lith,
            0.30 + 0.36 * n2,
            0.30 + 0.34 * n3,
            0.28 + 0.30 * n1,
            0.44 + 0.42 * n4,
        ),
        GeologyProvince::SubductionArc => (
            0.26 + 0.32 * n1,
            0.04 + 0.10 * n2,
            0.06 + 0.12 * n3,
            0.58 + 0.38 * n4,
            0.26 + 0.32 * v_lith,
            0.22 + 0.32 * n2,
            0.14 + 0.20 * n3,
            0.16 + 0.22 * n1,
            0.66 + 0.32 * n4,
        ),
        GeologyProvince::ContinentalShelf => (
            0.10 + 0.18 * n1,
            0.22 + 0.28 * n2,
            0.56 + 0.40 * n3,
            0.08 + 0.14 * n4,
            0.12 + 0.20 * v_lith,
            0.08 + 0.14 * n2,
            0.08 + 0.14 * n3,
            0.48 + 0.40 * n1,
            0.16 + 0.20 * n4,
        ),
        GeologyProvince::OceanicAbyss => (
            0.14 + 0.24 * n1,
            0.01 + 0.03 * n2,
            0.02 + 0.06 * n3,
            0.26 + 0.34 * n4,
            0.06 + 0.12 * v_lith,
            0.16 + 0.26 * n2,
            0.04 + 0.08 * n3,
            0.06 + 0.12 * n1,
            0.42 + 0.44 * n4,
        ),
        GeologyProvince::PolarShield => (
            0.45 + 0.40 * n1,
            0.18 + 0.24 * n2,
            0.18 + 0.26 * n3,
            0.24 + 0.30 * n4,
            0.16 + 0.26 * v_lith,
            0.38 + 0.42 * n2,
            0.36 + 0.40 * n3,
            0.16 + 0.22 * n1,
            0.12 + 0.18 * n4,
        ),
    }
}
