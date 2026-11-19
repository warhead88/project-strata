use crate::hydrology::{WaterBodyKind, WaterSample};
use crate::macro_cache::MacroCell;

/// Point-sampled climate and cryosphere state returned by [`crate::TerrainWorld::sample_climate`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ClimateSample {
    /// Mean annual surface air temperature in degrees Celsius.
    pub mean_temp_c: f32,
    /// Mean annual precipitation in millimeters.
    pub annual_precip_mm: f32,
    /// Dimensionless aridity index (`PET / precipitation`, higher = drier).
    pub aridity: f32,
    /// Mean fractional snow/ice cover in `[0.0, 1.0]`.
    pub snow_cover: f32,
    /// Permafrost probability in `[0.0, 1.0]`.
    pub permafrost_prob: f32,
}

/// Fourteen physical-geographic biomes covering oceans, polar ice, forests, grasslands, deserts, and alpine zones.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Biome {
    /// Permanent continental ice sheets and polar pack ice.
    PolarIce,
    /// Arctic or sub-polar treeless tundra.
    Tundra,
    /// High-latitude boreal conifer forest (taiga).
    BorealForest,
    /// Mid-latitude broadleaf or mixed forest.
    TemperateForest,
    /// Mid-latitude humid prairie and meadow.
    TemperateGrassland,
    /// Semi-arid continental steppe and shrubland.
    Steppe,
    /// Warm subtropical or tropical desert.
    HotDesert,
    /// Cold continental or high-plateau desert.
    ColdDesert,
    /// Tropical and subtropical savanna / dry woodland.
    Savanna,
    /// Humid equatorial rainforest.
    TropicalRainforest,
    /// High-altitude alpine tundra above treeline.
    AlpineTundra,
    /// High-altitude bare alpine rock, talus, and peak ridges.
    AlpineRock,
    /// Low-lying humid coastal marsh, delta, and wetland.
    CoastalWetland,
    /// Ice-free ocean, coastal shelf, lake, or river channel.
    OpenWater,
}

/// Derives [`ClimateSample`] from cached macro climate, refined elevation, and hydrology.
#[must_use]
#[expect(
    clippy::cast_possible_truncation,
    reason = "climate indicators are intentionally stored as f32"
)]
pub(crate) fn compute_climate_sample(
    cell: MacroCell,
    elevation_m: f64,
    water: WaterSample,
) -> ClimateSample {
    let macro_elev = f64::from(cell.macro_elevation_m).max(0.0);
    let fine_elev = elevation_m.max(0.0);
    let elev_delta_lapse = ((fine_elev - macro_elev) / 1000.0) * 6.5;
    let temp_c = f64::from(cell.mean_temp_c) - elev_delta_lapse;
    let precip_mm = f64::from(cell.annual_precip_mm);

    let pet_mm = (temp_c + 6.0).max(0.5) * 42.0;
    let aridity = (pet_mm / precip_mm.max(10.0)).clamp(0.0, 20.0);

    let snow_temp = ((2.0 - temp_c) / 12.0).clamp(0.0, 1.0);
    let moisture_factor = if temp_c < -12.0 {
        1.0
    } else {
        (precip_mm / 180.0).clamp(0.2, 1.0)
    };
    let snow_cover = (snow_temp * moisture_factor).clamp(0.0, 1.0);

    let permafrost_prob = if water.kind == WaterBodyKind::Dry {
        ((-temp_c) / 8.0).clamp(0.0, 1.0)
    } else {
        0.0
    };

    ClimateSample {
        mean_temp_c: temp_c as f32,
        annual_precip_mm: precip_mm as f32,
        aridity: aridity as f32,
        snow_cover: snow_cover as f32,
        permafrost_prob: permafrost_prob as f32,
    }
}

/// Classifies the physical-geographic [`Biome`] at `elevation_m` given `water` and `climate`.
#[must_use]
pub fn classify_biome(elevation_m: f64, water: WaterSample, climate: ClimateSample) -> Biome {
    let temp = climate.mean_temp_c;
    let precip = climate.annual_precip_mm;
    let aridity = climate.aridity;

    if water.kind != WaterBodyKind::Dry {
        if water.kind == WaterBodyKind::Ocean && temp < -8.0 {
            return Biome::PolarIce;
        }
        return Biome::OpenWater;
    }

    if temp < -9.0 || (climate.snow_cover >= 0.88 && temp < -3.0 && elevation_m < 3600.0) {
        return Biome::PolarIce;
    }

    if elevation_m >= 4400.0 || (elevation_m >= 3300.0 && temp < -3.5 && climate.snow_cover < 0.72)
    {
        return Biome::AlpineRock;
    }
    if elevation_m >= 2600.0 && temp < 5.0 {
        return Biome::AlpineTundra;
    }

    if temp < 2.0 {
        if precip < 160.0 && aridity >= 2.2 {
            return Biome::ColdDesert;
        }
        return Biome::Tundra;
    }

    if aridity >= 4.2 || precip < 190.0 {
        return if temp >= 14.0 {
            Biome::HotDesert
        } else {
            Biome::ColdDesert
        };
    }

    if elevation_m <= 14.0 && precip >= 950.0 && temp > 3.0 {
        return Biome::CoastalWetland;
    }

    if temp < 8.0 {
        return if precip >= 320.0 {
            Biome::BorealForest
        } else {
            Biome::Steppe
        };
    }

    if precip < 460.0 {
        return Biome::Steppe;
    }

    if temp >= 20.5 {
        return if precip >= 1600.0 {
            Biome::TropicalRainforest
        } else {
            Biome::Savanna
        };
    }

    if precip >= 650.0 {
        Biome::TemperateForest
    } else {
        Biome::TemperateGrassland
    }
}

/// Computes physical-geographic surface albedo RGB in `[0.0, 1.0]^3`.
#[must_use]
#[expect(
    clippy::cast_possible_truncation,
    reason = "render albedo colors intentionally use f32 channels"
)]
pub fn surface_albedo_rgb(
    elevation_m: f64,
    slope_sin: f64,
    water: WaterSample,
    climate: ClimateSample,
    biome: Biome,
) -> [f32; 3] {
    if biome == Biome::OpenWater {
        return match water.kind {
            WaterBodyKind::Ocean => {
                let d = water.depth_m;
                if d < 12.0 {
                    [0.09, 0.42, 0.52]
                } else if d < 200.0 {
                    let t = ((d - 12.0) / 188.0) as f32;
                    lerp_rgb([0.07, 0.34, 0.48], [0.03, 0.19, 0.36], t)
                } else {
                    let t = ((d - 200.0) / 4000.0).clamp(0.0, 1.0) as f32;
                    lerp_rgb([0.03, 0.19, 0.36], [0.012, 0.065, 0.17], t)
                }
            }
            WaterBodyKind::River(_) => [0.06, 0.28, 0.39],
            WaterBodyKind::Lake(_) => {
                if water.is_freshwater {
                    [0.04, 0.24, 0.42]
                } else {
                    [0.06, 0.29, 0.37]
                }
            }
            WaterBodyKind::Dry => [0.04, 0.24, 0.42],
        };
    }

    let mut rgb = match biome {
        Biome::PolarIce => [0.89, 0.93, 0.97],
        Biome::Tundra => [0.43, 0.46, 0.34],
        Biome::BorealForest => [0.14, 0.24, 0.15],
        Biome::TemperateForest => [0.18, 0.32, 0.16],
        Biome::TemperateGrassland => [0.36, 0.46, 0.22],
        Biome::Steppe => [0.55, 0.49, 0.31],
        Biome::HotDesert => [0.76, 0.62, 0.42],
        Biome::ColdDesert => [0.58, 0.54, 0.46],
        Biome::Savanna => [0.52, 0.48, 0.24],
        Biome::TropicalRainforest => [0.10, 0.27, 0.11],
        Biome::AlpineTundra => [0.40, 0.41, 0.32],
        Biome::AlpineRock => [0.38, 0.36, 0.34],
        Biome::CoastalWetland => [0.22, 0.33, 0.21],
        Biome::OpenWater => [0.03, 0.19, 0.36],
    };

    // Subtle sandy coast transition just above sea level.
    if (0.0..6.0).contains(&elevation_m) && slope_sin < 0.25 && biome != Biome::PolarIce {
        let beach_t = ((1.0 - elevation_m / 6.0) * 0.55) as f32;
        rgb = lerp_rgb(rgb, [0.74, 0.68, 0.52], beach_t);
    }

    // Steep slopes expose rock/scree and shed snow.
    let rock_blend = ((slope_sin - 0.30) / 0.45).clamp(0.0, 1.0) as f32;
    if rock_blend > 0.0 && biome != Biome::PolarIce {
        rgb = lerp_rgb(rgb, [0.36, 0.34, 0.32], rock_blend * 0.85);
    }

    let snow_retention = (1.0 - rock_blend * 0.75).max(0.15);
    let snow_blend = (climate.snow_cover * snow_retention).clamp(0.0, 1.0);
    if snow_blend > 0.01 && biome != Biome::PolarIce {
        rgb = lerp_rgb(rgb, [0.90, 0.93, 0.96], snow_blend * 0.88);
    }

    rgb
}

#[inline]
fn lerp_rgb(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [
        a[0] + (b[0] - a[0]) * t,
        a[1] + (b[1] - a[1]) * t,
        a[2] + (b[2] - a[2]) * t,
    ]
}
