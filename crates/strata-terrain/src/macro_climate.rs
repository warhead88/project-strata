use std::f64::consts::FRAC_PI_2;

use strata_globe::EcefMeters;

use crate::dem::DemGrid;
use crate::earth_data::{
    CRATON_REGIONS, PROVINCE_CONTINENTAL_SHELF, PROVINCE_CRATON, PROVINCE_OCEANIC_ABYSS,
    PROVINCE_OLD_OROGEN_RIFT, PROVINCE_POLAR_SHIELD, PROVINCE_SEDIMENTARY_BASIN,
    PROVINCE_SUBDUCTION_ARC, PROVINCE_YOUNG_OROGEN, SUBDUCTION_ARC_REGIONS,
};
use crate::earth_dem::EarthDem;
use crate::earth_geography::MacroSample;
use crate::noise::noise3;

/// Evaluates macro terrain, roughness, continentality, climate, geology, and DEM quad `[min, max]`
/// directly from [`EarthDem`].
#[must_use]
#[expect(
    clippy::cast_possible_truncation,
    reason = "macro cache fields intentionally narrow f64 values into f32 storage"
)]
pub(crate) fn evaluate_earth_macro(
    dem: &EarthDem,
    unit_dir: EcefMeters,
    seed: u64,
) -> (MacroSample, f64, f64) {
    let lat_rad = unit_dir.y.clamp(-1.0, 1.0).asin();
    let lon_rad = unit_dir.x.atan2(unit_dir.z);

    let (elev, relief, qmin, qmax, land_frac) = dem.sample_earth_vertex_features(lat_rad, lon_rad);

    let continentality = if elev >= 0.0 {
        (0.72 * land_frac + 0.28 * smoothstep(100.0, 2_200.0, elev)).clamp(0.0, 1.0)
    } else {
        (0.35 * land_frac).clamp(0.0, 0.35)
    };

    let roughness = compute_earth_roughness(elev, relief);
    let lat_deg = lat_rad.to_degrees();
    let lon_deg = lon_rad.to_degrees();
    let province = classify_earth_province(elev, relief, lat_deg, lon_deg);
    let (mean_temp_c, annual_precip_mm) = earth_climate_lat_lon(
        unit_dir,
        lat_rad,
        lat_deg,
        lon_deg,
        elev,
        continentality,
        seed,
    );

    (
        MacroSample {
            macro_elevation_m: elev as f32,
            roughness: roughness as f32,
            continentality: continentality as f32,
            mean_temp_c: mean_temp_c as f32,
            annual_precip_mm: annual_precip_mm as f32,
            geology_province: province,
        },
        qmin,
        qmax,
    )
}

/// Evaluates macro cell attributes and DEM quad `[min, max]` from an imported [`DemGrid`].
#[must_use]
#[expect(
    clippy::cast_possible_truncation,
    reason = "macro cache fields intentionally narrow f64 values into f32 storage"
)]
pub(crate) fn evaluate_dem_macro(
    grid: &DemGrid,
    unit_dir: EcefMeters,
    seed: u64,
) -> (MacroSample, f64, f64) {
    let (elev, relief, qmin, qmax) = grid.sample_bilinear_relief_minmax(unit_dir);
    let continentality = if elev >= 0.0 {
        smoothstep(0.0, 900.0, elev).clamp(0.2, 1.0)
    } else {
        0.0
    };
    let roughness = compute_earth_roughness(elev, relief);
    let province = if elev < -2_000.0 {
        PROVINCE_OCEANIC_ABYSS
    } else if elev < 0.0 {
        PROVINCE_CONTINENTAL_SHELF
    } else if elev > 2_200.0 || relief > 300.0 {
        PROVINCE_YOUNG_OROGEN
    } else if elev > 700.0 || relief > 130.0 {
        PROVINCE_OLD_OROGEN_RIFT
    } else {
        PROVINCE_SEDIMENTARY_BASIN
    };
    let lat_rad = unit_dir.y.clamp(-1.0, 1.0).asin();
    let (mean_temp_c, annual_precip_mm) =
        generic_climate_lat(unit_dir, lat_rad, elev, continentality, seed);

    (
        MacroSample {
            macro_elevation_m: elev as f32,
            roughness: roughness as f32,
            continentality: continentality as f32,
            mean_temp_c: mean_temp_c as f32,
            annual_precip_mm: annual_precip_mm as f32,
            geology_province: province,
        },
        qmin,
        qmax,
    )
}

#[inline]
fn compute_earth_roughness(elev_m: f64, local_relief_m: f64) -> f64 {
    if elev_m <= -120.0 {
        return (0.02 + (local_relief_m / 4_000.0).clamp(0.0, 0.14)).clamp(0.02, 0.16);
    }
    if elev_m <= 0.0 {
        return 0.02;
    }
    let relief_t = ((local_relief_m - 15.0) / 550.0).clamp(0.0, 1.0);
    let alt_boost = ((elev_m - 1_200.0) / 5_000.0).clamp(0.0, 0.22);
    (0.02 + 0.78 * relief_t + alt_boost).clamp(0.02, 1.0)
}

#[inline]
fn classify_earth_province(elev_m: f64, local_relief_m: f64, lat_deg: f64, lon_deg: f64) -> u8 {
    if elev_m < -5_200.0 || (elev_m < -2_200.0 && local_relief_m > 650.0) {
        return PROVINCE_SUBDUCTION_ARC;
    }
    if elev_m < -2_000.0 {
        return PROVINCE_OCEANIC_ABYSS;
    }
    if elev_m < 0.0 {
        return PROVINCE_CONTINENTAL_SHELF;
    }
    if (lat_deg <= -65.0 && elev_m > 200.0)
        || (lat_deg >= 61.0 && (-60.0..=-20.0).contains(&lon_deg) && elev_m > 500.0)
    {
        return PROVINCE_POLAR_SHIELD;
    }
    if elev_m > 1_600.0 || local_relief_m > 280.0 {
        return PROVINCE_YOUNG_OROGEN;
    }
    if SUBDUCTION_ARC_REGIONS
        .iter()
        .any(|r| r.contains(lat_deg, lon_deg))
    {
        return PROVINCE_SUBDUCTION_ARC;
    }
    if elev_m > 650.0 || local_relief_m > 130.0 {
        return PROVINCE_OLD_OROGEN_RIFT;
    }
    if CRATON_REGIONS.iter().any(|r| r.contains(lat_deg, lon_deg)) {
        return PROVINCE_CRATON;
    }
    PROVINCE_SEDIMENTARY_BASIN
}

fn earth_climate_lat_lon(
    unit_dir: EcefMeters,
    lat_rad: f64,
    lat_deg: f64,
    lon_deg: f64,
    elev_m: f64,
    continentality: f64,
    seed: u64,
) -> (f64, f64) {
    let (mut temp_c, mut precip_mm) =
        generic_climate_lat(unit_dir, lat_rad, elev_m, continentality, seed);

    // Sahara + Arabia hyper-arid belt (~14..32 N, -17..58 E).
    if (2.0..=44.0).contains(&lat_deg) && (-48.0..=84.0).contains(&lon_deg) {
        let sahara = gaussian_band(lat_deg, 23.0, 8.5) * gaussian_band(lon_deg, 18.0, 26.0);
        if sahara > 0.05 {
            precip_mm *= 1.0 - 0.88 * sahara;
            temp_c += 5.8 * sahara;
        }
    }

    // Amazon, Congo & SE Asia monsoon humid belts.
    let amazon = if (-26.0..=19.0).contains(&lat_deg) && (-97.0..=-27.0).contains(&lon_deg) {
        gaussian_band(lat_deg, -3.5, 9.0) * gaussian_band(lon_deg, -62.0, 14.0)
    } else {
        0.0
    };
    let congo = if (-19.0..=19.0).contains(&lat_deg) && (-3.0..=47.0).contains(&lon_deg) {
        gaussian_band(lat_deg, 0.0, 7.5) * gaussian_band(lon_deg, 22.0, 10.0)
    } else {
        0.0
    };
    let monsoon_se = if (-14.0..=46.0).contains(&lat_deg) && (53.0..=143.0).contains(&lon_deg) {
        gaussian_band(lat_deg, 16.0, 12.0) * gaussian_band(lon_deg, 98.0, 18.0)
    } else {
        0.0
    };
    let humid_boost = (amazon + congo + 0.8 * monsoon_se).clamp(0.0, 1.0);
    precip_mm += humid_boost * 950.0;

    // Gulf Stream moderation for Western/Northern Europe.
    if (24.0..=84.0).contains(&lat_deg) && (-39.0..=51.0).contains(&lon_deg) {
        let europe_warm = gaussian_band(lat_deg, 54.0, 12.0) * gaussian_band(lon_deg, 6.0, 18.0);
        temp_c += europe_warm * 5.0;
    }

    (temp_c, precip_mm.clamp(10.0, 4_500.0))
}

fn generic_climate_lat(
    unit_dir: EcefMeters,
    lat_rad: f64,
    elev_m: f64,
    continentality: f64,
    seed: u64,
) -> (f64, f64) {
    let abs_lat = lat_rad.abs();
    let lat_norm = abs_lat / FRAC_PI_2;
    let cos_lat = abs_lat.cos();
    let cos_pow_1_25 = cos_lat * cos_lat.sqrt().sqrt();

    // Sea-level temperature curve: +28 C at equator -> -20 C at pole.
    let sea_level_temp = -20.0 + 48.0 * cos_pow_1_25;
    let lapse_cooling = if elev_m > 0.0 {
        (elev_m / 1_000.0) * 6.5
    } else {
        0.0
    };
    let polar_continent_chill = if lat_norm > 0.65 {
        continentality * (lat_norm - 0.65) * 26.0
    } else {
        0.0
    };
    let temp_noise = noise3(
        unit_dir.x * 3.2,
        unit_dir.y * 3.2,
        unit_dir.z * 3.2,
        seed.wrapping_add(307),
    ) * 2.5;
    let mean_temp_c = sea_level_temp - lapse_cooling - polar_continent_chill + temp_noise;

    // Zonal precipitation belts: ITCZ peak (0 deg), subtropical dry (~25 deg), storm track (~52 deg).
    let lat_deg = abs_lat.to_degrees();
    let itcz = 2_100.0 * (-(lat_deg / 11.5).powi(2)).exp();
    let storm_track = 950.0 * (-((lat_deg - 52.0) / 14.0).powi(2)).exp();
    let subtropical_dry = 1.0 - 0.68 * (-((lat_deg - 26.0) / 8.5).powi(2)).exp();
    let interior_drying = 1.0 - 0.45 * continentality;
    let moisture_var = 1.0
        + 0.25
            * noise3(
                unit_dir.x * 4.0,
                unit_dir.y * 4.0,
                unit_dir.z * 4.0,
                seed.wrapping_add(409),
            );

    let base_precip = (180.0 + itcz + storm_track) * subtropical_dry * interior_drying;
    let elev_rain_shadow = if elev_m > 2_500.0 {
        (1.0 - ((elev_m - 2_500.0) / 5_500.0).clamp(0.0, 0.65)).max(0.35)
    } else {
        1.0
    };
    let annual_precip_mm = (base_precip * moisture_var * elev_rain_shadow).clamp(15.0, 4_200.0);

    (mean_temp_c, annual_precip_mm)
}

/// Accurate angular distance in radians between two unit vectors even near zero angle.
#[must_use]
pub(crate) fn angular_distance(a: EcefMeters, b: EcefMeters) -> f64 {
    let chord = (a - b).norm();
    2.0 * (0.5 * chord).clamp(0.0, 1.0).asin()
}

pub(crate) fn point_to_arc_distance_rad(p: EcefMeters, a: EcefMeters, b: EcefMeters) -> f64 {
    let ab = b - a;
    let ab2 = ab.dot(ab);
    let closest_raw = if ab2 <= 1e-12 {
        a
    } else {
        let t = ((p - a).dot(ab) / ab2).clamp(0.0, 1.0);
        a + ab.scale(t)
    };
    let len = closest_raw.norm();
    let closest = if len > 0.0 {
        closest_raw.scale(1.0 / len)
    } else {
        a
    };
    angular_distance(p, closest)
}

#[inline]
fn gaussian_band(x: f64, center: f64, sigma: f64) -> f64 {
    let d = (x - center) / sigma;
    (-d * d).exp()
}

#[inline]
pub(crate) fn smoothstep(edge0: f64, edge1: f64, x: f64) -> f64 {
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

pub(crate) fn closest_on_unit_segment(
    p: EcefMeters,
    a: EcefMeters,
    b: EcefMeters,
) -> (EcefMeters, f64) {
    let ab = b - a;
    let ab2 = ab.dot(ab);
    if ab2 <= 1e-12 {
        return (a, 0.0);
    }
    let s = ((p - a).dot(ab) / ab2).clamp(0.0, 1.0);
    let raw = a + ab.scale(s);
    let len = raw.norm();
    let unit = if len > 0.0 { raw.scale(1.0 / len) } else { a };
    (unit, s)
}
