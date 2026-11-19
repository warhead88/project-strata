use crate::strata::{
    BedrockKind, GeologyProvince, MAX_STRATA_DEPTH_M, MAX_STRATA_LAYERS, StratumLayer,
};

#[expect(
    clippy::cast_possible_truncation,
    reason = "layer depths and normalized rock attributes intentionally use f32"
)]
fn layer(
    top_m: f32,
    bottom_m: f32,
    bedrock: BedrockKind,
    hardness: f64,
    stability: f64,
    permeability: f64,
    thermal_c_km: f64,
) -> StratumLayer {
    StratumLayer {
        top_depth_m: top_m,
        bottom_depth_m: bottom_m,
        bedrock,
        hardness: hardness.clamp(0.05, 1.0) as f32,
        structural_stability: stability.clamp(0.05, 1.0) as f32,
        permeability: permeability.clamp(0.01, 1.0) as f32,
        thermal_gradient_c_per_km: thermal_c_km.clamp(12.0, 90.0) as f32,
    }
}

#[expect(
    clippy::cast_possible_truncation,
    reason = "stratum depth boundaries intentionally use f32 meters"
)]
pub(crate) fn build_province_layers(
    province: GeologyProvince,
    v_depth: f64,
    v_lith: f64,
) -> ([StratumLayer; MAX_STRATA_LAYERS], u8) {
    let shift = (v_depth - 0.5) * 0.22;
    let d1 = (160.0 * (1.0 + shift)) as f32;
    let d2 = (900.0 * (1.0 + shift)) as f32;
    let d3 = (2_800.0 * (1.0 + shift)) as f32;
    let d4 = (6_000.0 * (1.0 + shift)) as f32;

    match province {
        GeologyProvince::CratonShield => craton_layers(d1, d2, d4, v_lith),
        GeologyProvince::SedimentaryBasin => basin_layers(d1, d2, d3, d4, v_lith),
        GeologyProvince::YoungOrogen => young_orogen_layers(d1, d2, d4),
        GeologyProvince::OldOrogenRift => old_orogen_layers(d1, d2, d4),
        GeologyProvince::SubductionArc => arc_layers(d1, d2, d4),
        GeologyProvince::ContinentalShelf => shelf_layers(d1, d2, d3),
        GeologyProvince::OceanicAbyss => abyss_layers(d1, d3),
        GeologyProvince::PolarShield => polar_shield_layers(d1, d2, d4),
    }
}

fn young_orogen_layers(d1: f32, d2: f32, d4: f32) -> ([StratumLayer; MAX_STRATA_LAYERS], u8) {
    (
        [
            layer(0.0, d1, BedrockKind::LimestoneKarst, 0.62, 0.58, 0.44, 31.0),
            layer(d1, d2, BedrockKind::Metamorphic, 0.78, 0.70, 0.22, 33.0),
            layer(d2, d4, BedrockKind::Granite, 0.86, 0.76, 0.14, 35.0),
            layer(
                d4,
                MAX_STRATA_DEPTH_M,
                BedrockKind::Metamorphic,
                0.90,
                0.78,
                0.09,
                34.0,
            ),
            dummy_layer(BedrockKind::Granite),
        ],
        4,
    )
}

fn old_orogen_layers(d1: f32, d2: f32, d4: f32) -> ([StratumLayer; MAX_STRATA_LAYERS], u8) {
    (
        [
            layer(0.0, d1, BedrockKind::Sandstone, 0.56, 0.68, 0.40, 34.0),
            layer(d1, d2, BedrockKind::Metamorphic, 0.76, 0.80, 0.18, 38.0),
            layer(d2, d4, BedrockKind::Basalt, 0.82, 0.82, 0.14, 42.0),
            layer(
                d4,
                MAX_STRATA_DEPTH_M,
                BedrockKind::Granite,
                0.88,
                0.86,
                0.08,
                40.0,
            ),
            dummy_layer(BedrockKind::Granite),
        ],
        4,
    )
}

fn arc_layers(d1: f32, d2: f32, d4: f32) -> ([StratumLayer; MAX_STRATA_LAYERS], u8) {
    (
        [
            layer(0.0, d1, BedrockKind::Basalt, 0.70, 0.64, 0.34, 48.0),
            layer(d1, d2, BedrockKind::Basalt, 0.76, 0.68, 0.26, 54.0),
            layer(d2, d4, BedrockKind::Granite, 0.82, 0.72, 0.16, 58.0),
            layer(
                d4,
                MAX_STRATA_DEPTH_M,
                BedrockKind::Metamorphic,
                0.88,
                0.74,
                0.10,
                52.0,
            ),
            dummy_layer(BedrockKind::Granite),
        ],
        4,
    )
}

fn shelf_layers(d1: f32, d2: f32, d3: f32) -> ([StratumLayer; MAX_STRATA_LAYERS], u8) {
    (
        [
            layer(0.0, d1, BedrockKind::Sandstone, 0.38, 0.52, 0.68, 25.0),
            layer(d1, d2, BedrockKind::LimestoneKarst, 0.48, 0.56, 0.54, 26.0),
            layer(d2, d3, BedrockKind::Shale, 0.42, 0.50, 0.22, 26.0),
            layer(
                d3,
                MAX_STRATA_DEPTH_M,
                BedrockKind::Granite,
                0.84,
                0.86,
                0.08,
                23.0,
            ),
            dummy_layer(BedrockKind::Granite),
        ],
        4,
    )
}

fn abyss_layers(d1: f32, d3: f32) -> ([StratumLayer; MAX_STRATA_LAYERS], u8) {
    (
        [
            layer(0.0, d1 * 0.6, BedrockKind::Shale, 0.26, 0.38, 0.45, 38.0),
            layer(d1 * 0.6, d3, BedrockKind::Basalt, 0.78, 0.74, 0.22, 42.0),
            layer(
                d3,
                MAX_STRATA_DEPTH_M,
                BedrockKind::Basalt,
                0.86,
                0.78,
                0.12,
                40.0,
            ),
            dummy_layer(BedrockKind::Basalt),
            dummy_layer(BedrockKind::Basalt),
        ],
        3,
    )
}

fn polar_shield_layers(d1: f32, d2: f32, d4: f32) -> ([StratumLayer; MAX_STRATA_LAYERS], u8) {
    (
        [
            layer(0.0, d1, BedrockKind::Metamorphic, 0.80, 0.85, 0.09, 20.0),
            layer(d1, d2, BedrockKind::Granite, 0.89, 0.91, 0.06, 19.5),
            layer(d2, d4, BedrockKind::Granite, 0.93, 0.93, 0.04, 19.0),
            layer(
                d4,
                MAX_STRATA_DEPTH_M,
                BedrockKind::Metamorphic,
                0.95,
                0.90,
                0.03,
                18.5,
            ),
            dummy_layer(BedrockKind::Granite),
        ],
        4,
    )
}

fn craton_layers(
    d1: f32,
    d2: f32,
    d4: f32,
    v_lith: f64,
) -> ([StratumLayer; MAX_STRATA_LAYERS], u8) {
    (
        [
            layer(
                0.0,
                d1,
                BedrockKind::Metamorphic,
                0.78 + 0.1 * v_lith,
                0.84,
                0.12,
                20.0,
            ),
            layer(
                d1,
                d2,
                BedrockKind::Granite,
                0.88 + 0.08 * v_lith,
                0.92,
                0.06,
                19.0,
            ),
            layer(d2, d4, BedrockKind::Granite, 0.92, 0.94, 0.04, 18.5),
            layer(
                d4,
                MAX_STRATA_DEPTH_M,
                BedrockKind::Metamorphic,
                0.95,
                0.91,
                0.03,
                18.0,
            ),
            dummy_layer(BedrockKind::Granite),
        ],
        4,
    )
}

fn basin_layers(
    d1: f32,
    d2: f32,
    d3: f32,
    d4: f32,
    v_lith: f64,
) -> ([StratumLayer; MAX_STRATA_LAYERS], u8) {
    let cap = if v_lith > 0.62 {
        BedrockKind::LimestoneKarst
    } else {
        BedrockKind::Sandstone
    };
    let mid = if v_lith < 0.25 {
        BedrockKind::Evaporite
    } else {
        BedrockKind::Shale
    };
    (
        [
            layer(0.0, d1, cap, 0.42, 0.60, 0.64, 26.0),
            layer(d1, d2, BedrockKind::Shale, 0.34, 0.46, 0.24, 27.0),
            layer(d2, d3, BedrockKind::Sandstone, 0.52, 0.64, 0.58, 28.0),
            layer(d3, d4, mid, 0.46, 0.55, 0.32, 27.0),
            layer(
                d4,
                MAX_STRATA_DEPTH_M,
                BedrockKind::Granite,
                0.86,
                0.88,
                0.08,
                24.0,
            ),
        ],
        5,
    )
}

fn dummy_layer(bedrock: BedrockKind) -> StratumLayer {
    layer(
        MAX_STRATA_DEPTH_M,
        MAX_STRATA_DEPTH_M,
        bedrock,
        0.9,
        0.9,
        0.05,
        20.0,
    )
}
