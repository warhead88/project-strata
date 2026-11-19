use std::f64::consts::FRAC_PI_2;

use strata_globe::EcefMeters;

use crate::dem::DemGrid;
use crate::earth_data::{
    PROVINCE_CONTINENTAL_SHELF, PROVINCE_CRATON, PROVINCE_OCEANIC_ABYSS, PROVINCE_OLD_OROGEN_RIFT,
    PROVINCE_POLAR_SHIELD, PROVINCE_SEDIMENTARY_BASIN, PROVINCE_YOUNG_OROGEN,
};
use crate::earth_geography::MacroSample;
use crate::noise::{fbm3, ridged_fbm3};

/// Evaluates procedural continent/mountain/climate macro fields for `Procedural` mode.
#[must_use]
#[expect(
    clippy::cast_possible_truncation,
    reason = "macro cache fields intentionally narrow f64 values into f32 storage"
)]
pub(crate) fn evaluate_procedural_macro(unit_dir: EcefMeters, seed: u64) -> MacroSample {
    let cont_base = fbm3(unit_dir, 1.8, 4, 2.05, 0.52, seed.wrapping_add(3));
    let cont_detail = fbm3(unit_dir, 4.5, 3, 2.1, 0.5, seed.wrapping_add(19));
    let sea_level_offset = 0.02;
    let land_val = cont_base * 0.78 + cont_detail * 0.22 - sea_level_offset;

    let ridge = ridged_fbm3(unit_dir, 3.6, 4, 2.15, 0.55, seed.wrapping_add(41));
    let trench_noise = ridged_fbm3(unit_dir, 2.8, 3, 2.0, 0.5, seed.wrapping_add(67));

    let (elev, roughness, province) = if land_val > 0.02 {
        let inland = smoothstep(0.02, 0.42, land_val);
        let mtn_belt = smoothstep(0.55, 0.92, ridge) * smoothstep(0.02, 0.22, land_val);
        let e = 35.0 + inland * 850.0 + mtn_belt * 5400.0;
        let r = (0.12 + 0.72 * mtn_belt + 0.12 * ridge).clamp(0.05, 1.0);
        let p = if unit_dir.y.abs() > 0.92 && e > 400.0 {
            PROVINCE_POLAR_SHIELD
        } else if mtn_belt > 0.35 {
            PROVINCE_YOUNG_OROGEN
        } else if inland > 0.5 {
            PROVINCE_CRATON
        } else {
            PROVINCE_SEDIMENTARY_BASIN
        };
        (e, r, p)
    } else if land_val > -0.09 {
        let t = smoothstep(-0.09, 0.02, land_val);
        (-190.0 + 225.0 * t, 0.04, PROVINCE_CONTINENTAL_SHELF)
    } else {
        let depth_t = smoothstep(-0.45, -0.09, land_val);
        let trench = smoothstep(0.76, 0.96, trench_noise) * 3200.0;
        let e = -4400.0 + depth_t * 4210.0 - trench;
        (e, 0.04, PROVINCE_OCEANIC_ABYSS)
    };

    let continentality = smoothstep(-0.04, 0.36, land_val).clamp(0.0, 1.0);
    let (mean_temp_c, annual_precip_mm) = generic_climate(unit_dir, elev, continentality, seed);

    MacroSample {
        macro_elevation_m: elev as f32,
        roughness: roughness as f32,
        continentality: continentality as f32,
        mean_temp_c: mean_temp_c as f32,
        annual_precip_mm: annual_precip_mm as f32,
        geology_province: province,
    }
}

/// Evaluates macro cell attributes from an imported [`DemGrid`].
#[must_use]
#[expect(
    clippy::cast_possible_truncation,
    reason = "macro cache fields intentionally narrow f64 values into f32 storage"
)]
pub(crate) fn evaluate_dem_macro(grid: &DemGrid, unit_dir: EcefMeters, seed: u64) -> MacroSample {
    let elev = grid.sample_bilinear(unit_dir);
    let continentality = if elev >= 0.0 {
        smoothstep(0.0, 900.0, elev).clamp(0.2, 1.0)
    } else {
        0.0
    };
    let roughness = if elev >= 0.0 {
        (0.10 + 0.65 * smoothstep(300.0, 4500.0, elev)).clamp(0.05, 1.0)
    } else {
        0.04
    };
    let province = if elev < -2000.0 {
        PROVINCE_OCEANIC_ABYSS
    } else if elev < 0.0 {
        PROVINCE_CONTINENTAL_SHELF
    } else if elev > 2200.0 {
        PROVINCE_YOUNG_OROGEN
    } else if elev > 700.0 {
        PROVINCE_OLD_OROGEN_RIFT
    } else {
        PROVINCE_SEDIMENTARY_BASIN
    };
    let (mean_temp_c, annual_precip_mm) = generic_climate(unit_dir, elev, continentality, seed);

    MacroSample {
        macro_elevation_m: elev as f32,
        roughness: roughness as f32,
        continentality: continentality as f32,
        mean_temp_c: mean_temp_c as f32,
        annual_precip_mm: annual_precip_mm as f32,
        geology_province: province,
    }
}

pub(crate) fn earth_climate(
    unit_dir: EcefMeters,
    elev_m: f64,
    continentality: f64,
    seed: u64,
) -> (f64, f64) {
    let (mut temp_c, mut precip_mm) = generic_climate(unit_dir, elev_m, continentality, seed);
    let lat_deg = unit_dir.y.clamp(-1.0, 1.0).asin().to_degrees();
    let lon_deg = unit_dir.x.atan2(unit_dir.z).to_degrees();

    // Sahara + Arabia hyper-arid belt (~14..32 N, -17..58 E).
    let sahara_lat = gaussian_band(lat_deg, 23.0, 8.5);
    let sahara_lon = gaussian_band(lon_deg, 18.0, 26.0);
    let sahara = sahara_lat * sahara_lon;
    if sahara > 0.05 {
        precip_mm *= 1.0 - 0.88 * sahara;
        temp_c += 3.5 * sahara;
    }

    // Amazon & Congo humid equatorial convergence zones.
    let amazon = gaussian_band(lat_deg, -3.5, 9.0) * gaussian_band(lon_deg, -62.0, 14.0);
    let congo = gaussian_band(lat_deg, 0.0, 7.5) * gaussian_band(lon_deg, 22.0, 10.0);
    let monsoon_se = gaussian_band(lat_deg, 16.0, 12.0) * gaussian_band(lon_deg, 98.0, 18.0);
    let humid_boost = (amazon + congo + 0.8 * monsoon_se).clamp(0.0, 1.0);
    precip_mm += humid_boost * 950.0;

    // Gulf Stream moderation for Western/Northern Europe.
    let europe_warm = gaussian_band(lat_deg, 54.0, 12.0) * gaussian_band(lon_deg, 6.0, 18.0);
    temp_c += europe_warm * 5.0;

    (temp_c, precip_mm.clamp(10.0, 4500.0))
}

pub(crate) fn generic_climate(
    unit_dir: EcefMeters,
    elev_m: f64,
    continentality: f64,
    seed: u64,
) -> (f64, f64) {
    let abs_lat = unit_dir.y.abs().clamp(0.0, 1.0).asin();
    let lat_norm = abs_lat / FRAC_PI_2;
    let cos_lat = abs_lat.cos();

    // Sea-level temperature curve: +28 C at equator -> -20 C at pole.
    let sea_level_temp = -20.0 + 48.0 * cos_lat.powf(1.25);
    let lapse_cooling = if elev_m > 0.0 {
        (elev_m / 1000.0) * 6.5
    } else {
        0.0
    };
    let polar_continent_chill = if lat_norm > 0.65 {
        continentality * (lat_norm - 0.65) * 26.0
    } else {
        0.0
    };
    let temp_noise = fbm3(unit_dir, 3.2, 2, 2.0, 0.5, seed.wrapping_add(307)) * 2.5;
    let mean_temp_c = sea_level_temp - lapse_cooling - polar_continent_chill + temp_noise;

    // Zonal precipitation belts: ITCZ peak (0 deg), subtropical dry (~25 deg), storm track (~52 deg).
    let lat_deg = abs_lat.to_degrees();
    let itcz = 2100.0 * (-(lat_deg / 11.5).powi(2)).exp();
    let storm_track = 950.0 * (-((lat_deg - 52.0) / 14.0).powi(2)).exp();
    let subtropical_dry = 1.0 - 0.68 * (-((lat_deg - 26.0) / 8.5).powi(2)).exp();
    let interior_drying = 1.0 - 0.45 * continentality;
    let moisture_var = 1.0 + 0.28 * fbm3(unit_dir, 4.0, 3, 2.0, 0.5, seed.wrapping_add(409));

    let base_precip = (180.0 + itcz + storm_track) * subtropical_dry * interior_drying;
    let elev_rain_shadow = if elev_m > 2500.0 {
        (1.0 - ((elev_m - 2500.0) / 5500.0).clamp(0.0, 0.65)).max(0.35)
    } else {
        1.0
    };
    let annual_precip_mm = (base_precip * moisture_var * elev_rain_shadow).clamp(15.0, 4200.0);

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
