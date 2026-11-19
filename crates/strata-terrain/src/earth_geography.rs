use std::f64::consts::PI;

use strata_globe::EcefMeters;

use crate::dem::DemGrid;
use crate::earth_data::{
    EARTH_LOBES, EARTH_RIDGES, GeoLobeSpec, GeoRidgeSpec, PROVINCE_CONTINENTAL_SHELF,
    PROVINCE_OCEANIC_ABYSS, PROVINCE_SUBDUCTION_ARC,
};
use crate::macro_climate::{
    angular_distance, earth_climate, point_to_arc_distance_rad, smoothstep,
};
use crate::noise::{fbm3, ridged_fbm3};

/// Source model used to bake [`crate::CubeMacroCache`] and drive world hydrology.
#[derive(Debug, Clone, PartialEq)]
pub enum TerrainSource {
    /// Earth-oriented macro geography with recognizable continents, mountain belts, seas,
    /// and trenches plus seed-deterministic micro-detail.
    Earth {
        /// Deterministic seed for micro-detail noise.
        seed: u64,
    },
    /// Seed-driven procedural world preserving Earth-like climate belts and scaling cleanly
    /// to smaller planet radii.
    Procedural {
        /// Deterministic seed for continents, mountains, and hydrology.
        seed: u64,
    },
    /// Externally supplied equirectangular DEM raster plugged straight into the macro cache.
    Dem {
        /// Validated elevation raster.
        grid: DemGrid,
        /// Deterministic seed for micro-detail noise.
        seed: u64,
    },
}

/// Macro-scale terrain and climate sample baked into each macro-cache vertex.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MacroSample {
    /// Macro elevation relative to sea level in meters.
    pub macro_elevation_m: f32,
    /// Dimensionless terrain roughness factor in `[0.0, 1.0]`.
    pub roughness: f32,
    /// Continentality factor in `[0.0, 1.0]` (`0` = open ocean, `1` = deep continental interior).
    pub continentality: f32,
    /// Mean annual surface temperature in degrees Celsius.
    pub mean_temp_c: f32,
    /// Mean annual precipitation in millimeters.
    pub annual_precip_mm: f32,
    /// Macro geology province identifier (`0..=7`).
    pub geology_province: u8,
}

/// Converts geodetic `(lat_deg, lon_deg)` to a unit [`EcefMeters`] direction using
/// `strata-globe` conventions (`+Y` = North, `+Z` = `0 deg E`, `+X` = `+90 deg E`).
#[must_use]
pub fn lat_lon_to_unit(lat_deg: f64, lon_deg: f64) -> EcefMeters {
    let (sin_lat, cos_lat) = lat_deg.to_radians().sin_cos();
    let (sin_lon, cos_lon) = lon_deg.to_radians().sin_cos();
    EcefMeters::from_raw(cos_lat * sin_lon, sin_lat, cos_lat * cos_lon)
}

#[derive(Debug, Clone, Copy)]
struct PreparedLobe {
    center: EcefMeters,
    east: EcefMeters,
    north: EcefMeters,
    inv_r_u: f64,
    inv_r_v: f64,
    min_cos: f64,
    weight: f64,
    base_elev_m: f64,
    province: u8,
}

#[derive(Debug, Clone, Copy)]
struct PreparedRidge {
    a: EcefMeters,
    b: EcefMeters,
    inv_half_width_rad: f64,
    min_cos: f64,
    peak_elev_m: f64,
    roughness_boost: f64,
    province: u8,
}

struct LobeAccum {
    net_land: f64,
    sea_carve: f64,
    base_land_elev: f64,
    sea_elev_num: f64,
    sea_elev_den: f64,
    dominant_province: u8,
}

/// Precomputed 3D Earth lobe and mountain/trench primitives for fast macro baking.
#[derive(Debug, Clone)]
pub(crate) struct PreparedEarthModel {
    lobes: Vec<PreparedLobe>,
    ridges: Vec<PreparedRidge>,
}

impl PreparedEarthModel {
    pub(crate) fn new() -> Self {
        let lobes = EARTH_LOBES.iter().copied().map(prepare_lobe).collect();
        let ridges = EARTH_RIDGES.iter().copied().map(prepare_ridge).collect();
        Self { lobes, ridges }
    }

    #[expect(
        clippy::cast_possible_truncation,
        reason = "macro cache fields intentionally narrow f64 values into f32 storage"
    )]
    pub(crate) fn evaluate(&self, unit_dir: EcefMeters, seed: u64) -> MacroSample {
        let wdir = warp_coast_direction(unit_dir, seed);
        let acc = self.accumulate_lobes(wdir);

        let coast_noise = fbm3(unit_dir, 9.0, 3, 2.05, 0.5, seed.wrapping_add(101)) * 0.09;
        let land_val = acc.net_land + coast_noise;

        let mut elev = shelf_to_abyss_elevation(unit_dir, land_val, acc.base_land_elev, seed);
        if acc.sea_elev_den > 0.25 && elev < -50.0 {
            let sea_target = acc.sea_elev_num / acc.sea_elev_den;
            let sea_blend = smoothstep(0.25, 0.85, acc.sea_carve).min(0.85);
            elev += (sea_target - elev) * sea_blend;
        }

        let mut roughness = if elev >= 0.0 {
            0.14 + 0.16 * ridged_fbm3(unit_dir, 7.5, 3, 2.0, 0.5, seed.wrapping_add(77))
        } else if elev < -2000.0 {
            0.07
        } else {
            0.03
        };

        let mut dominant_province = acc.dominant_province;
        self.apply_ridges(
            unit_dir,
            wdir,
            seed,
            &mut elev,
            &mut roughness,
            &mut dominant_province,
        );

        let continentality = smoothstep(0.28, 0.82, acc.net_land).clamp(0.0, 1.0);
        let province = if elev < -2000.0 {
            if dominant_province == PROVINCE_SUBDUCTION_ARC {
                dominant_province
            } else {
                PROVINCE_OCEANIC_ABYSS
            }
        } else if elev < 0.0 {
            PROVINCE_CONTINENTAL_SHELF
        } else {
            dominant_province
        };

        let (mean_temp_c, annual_precip_mm) = earth_climate(unit_dir, elev, continentality, seed);

        MacroSample {
            macro_elevation_m: elev as f32,
            roughness: roughness.clamp(0.01, 1.0) as f32,
            continentality: continentality as f32,
            mean_temp_c: mean_temp_c as f32,
            annual_precip_mm: annual_precip_mm as f32,
            geology_province: province,
        }
    }

    fn accumulate_lobes(&self, wdir: EcefMeters) -> LobeAccum {
        let mut land_signal = 0.0_f64;
        let mut elev_num = 0.0_f64;
        let mut elev_den = 0.0_f64;
        let mut sea_carve = 0.0_f64;
        let mut sea_elev_num = 0.0_f64;
        let mut sea_elev_den = 0.0_f64;
        let mut dominant_province = PROVINCE_OCEANIC_ABYSS;
        let mut dominant_strength = 0.0_f64;

        for lobe in &self.lobes {
            let dot = wdir.dot(lobe.center);
            if dot < lobe.min_cos {
                continue;
            }
            let delta = wdir - lobe.center.scale(dot);
            let du = delta.dot(lobe.east) * lobe.inv_r_u;
            let dv = delta.dot(lobe.north) * lobe.inv_r_v;
            let r2 = du * du + dv * dv;
            if r2 >= 1.45 {
                continue;
            }
            let falloff = (-r2 * 1.65).exp() * (1.0 - (r2 / 1.45)).clamp(0.0, 1.0);
            if lobe.weight >= 0.0 {
                let s = falloff * lobe.weight;
                land_signal += s;
                elev_num += s * lobe.base_elev_m;
                elev_den += s;
                if s > dominant_strength {
                    dominant_strength = s;
                    dominant_province = lobe.province;
                }
            } else {
                let s = falloff * (-lobe.weight);
                sea_carve += s;
                sea_elev_num += s * lobe.base_elev_m;
                sea_elev_den += s;
            }
        }

        LobeAccum {
            net_land: land_signal - sea_carve * 0.92,
            sea_carve,
            base_land_elev: if elev_den > 1e-6 {
                elev_num / elev_den
            } else {
                280.0
            },
            sea_elev_num,
            sea_elev_den,
            dominant_province,
        }
    }

    fn apply_ridges(
        &self,
        unit_dir: EcefMeters,
        wdir: EcefMeters,
        seed: u64,
        elev: &mut f64,
        roughness: &mut f64,
        dominant_province: &mut u8,
    ) {
        let ridge_detail = ridged_fbm3(unit_dir, 14.0, 3, 2.1, 0.55, seed.wrapping_add(211));
        for ridge in &self.ridges {
            if wdir.dot(ridge.a) < ridge.min_cos && wdir.dot(ridge.b) < ridge.min_cos {
                continue;
            }
            let d_rad = point_to_arc_distance_rad(wdir, ridge.a, ridge.b);
            let norm_d = d_rad * ridge.inv_half_width_rad;
            if norm_d >= 1.6 {
                continue;
            }
            let profile = (-norm_d * norm_d * 1.4).exp() * (1.0 - norm_d / 1.6).clamp(0.0, 1.0);
            if ridge.peak_elev_m >= 0.0 {
                let mountain_shape = 0.70 + 0.55 * ridge_detail;
                *elev += profile * ridge.peak_elev_m * mountain_shape;
            } else {
                *elev += profile * ridge.peak_elev_m;
            }
            *roughness = (*roughness + profile * ridge.roughness_boost).min(1.0);
            if profile > 0.35 {
                *dominant_province = ridge.province;
            }
        }
    }
}

fn warp_coast_direction(unit_dir: EcefMeters, seed: u64) -> EcefMeters {
    let wx = fbm3(unit_dir, 4.2, 3, 2.1, 0.5, seed.wrapping_add(11));
    let wy = fbm3(unit_dir, 4.2, 3, 2.1, 0.5, seed.wrapping_add(23));
    let wz = fbm3(unit_dir, 4.2, 3, 2.1, 0.5, seed.wrapping_add(37));
    let warped = EcefMeters::from_raw(
        unit_dir.x + wx * 0.045,
        unit_dir.y + wy * 0.045,
        unit_dir.z + wz * 0.045,
    );
    let warp_len = warped.norm();
    if warp_len > 0.0 {
        warped.scale(1.0 / warp_len)
    } else {
        unit_dir
    }
}

fn shelf_to_abyss_elevation(
    unit_dir: EcefMeters,
    land_val: f64,
    base_land_elev: f64,
    seed: u64,
) -> f64 {
    if land_val >= 0.44 {
        let inland = ((land_val - 0.44) / 0.48).clamp(0.0, 1.0);
        let coastal_ramp = smoothstep(0.0, 1.0, inland);
        40.0 + base_land_elev * (0.32 + 0.68 * coastal_ramp)
    } else if land_val >= 0.30 {
        let t = smoothstep(0.30, 0.44, land_val);
        -180.0 + 220.0 * t
    } else {
        let t = smoothstep(0.06, 0.30, land_val);
        let abyss = -4350.0 + fbm3(unit_dir, 3.0, 3, 2.0, 0.5, seed.wrapping_add(55)) * 550.0;
        abyss + (t * (-180.0 - abyss))
    }
}

fn prepare_lobe(spec: GeoLobeSpec) -> PreparedLobe {
    let center = lat_lon_to_unit(spec.lat_deg, spec.lon_deg);
    let world_up = if center.y.abs() > 0.99 {
        EcefMeters::from_raw(0.0, 0.0, 1.0)
    } else {
        EcefMeters::from_raw(0.0, 1.0, 0.0)
    };
    let base_east_raw = world_up.cross(center);
    let base_east = base_east_raw.scale(1.0 / base_east_raw.norm());
    let base_north = center.cross(base_east);

    let (sin_r, cos_r) = spec.rot_deg.to_radians().sin_cos();
    let east = base_east.scale(cos_r) + base_north.scale(sin_r);
    let north = base_north.scale(cos_r) - base_east.scale(sin_r);

    let r_u = spec.radius_u_deg.to_radians().max(1e-3);
    let r_v = spec.radius_v_deg.to_radians().max(1e-3);
    let max_r = r_u.max(r_v) * 1.3;
    PreparedLobe {
        center,
        east,
        north,
        inv_r_u: 1.0 / r_u,
        inv_r_v: 1.0 / r_v,
        min_cos: max_r.min(PI).cos(),
        weight: spec.weight,
        base_elev_m: spec.base_elev_m,
        province: spec.province,
    }
}

fn prepare_ridge(spec: GeoRidgeSpec) -> PreparedRidge {
    let a = lat_lon_to_unit(spec.lat_a_deg, spec.lon_a_deg);
    let b = lat_lon_to_unit(spec.lat_b_deg, spec.lon_b_deg);
    let hw = spec.half_width_deg.to_radians().max(1e-4);
    let seg_half = 0.5 * angular_distance(a, b);
    let bound = (seg_half + hw * 1.7).min(PI);
    PreparedRidge {
        a,
        b,
        inv_half_width_rad: 1.0 / hw,
        min_cos: bound.cos(),
        peak_elev_m: spec.peak_elev_m,
        roughness_boost: spec.roughness_boost,
        province: spec.province,
    }
}
