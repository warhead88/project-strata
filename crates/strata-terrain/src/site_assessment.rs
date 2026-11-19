use std::f64::consts::TAU;

use strata_globe::EcefMeters;

use crate::climate::Biome;
use crate::error::TerrainError;
use crate::hydrology::{Navigability, WaterBodyKind, is_deepwater_port_depth};
use crate::query::{
    GeographicTags, SiteAssessment, TerrainSample, argmax, biome_index, geodesic_offset,
    tangent_basis,
};
use crate::strata::{GeologyProvince, SurfaceMaterial};
use crate::terrain_world::TerrainWorld;

const SAMPLE_COUNT_U32: u32 = 25;
const SAMPLE_COUNT: usize = SAMPLE_COUNT_U32 as usize;

const BIOME_VARIANTS: [Biome; 14] = [
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

const PROVINCE_VARIANTS: [GeologyProvince; 8] = [
    GeologyProvince::OceanicAbyss,
    GeologyProvince::ContinentalShelf,
    GeologyProvince::CratonShield,
    GeologyProvince::SedimentaryBasin,
    GeologyProvince::YoungOrogen,
    GeologyProvince::OldOrogenRift,
    GeologyProvince::SubductionArc,
    GeologyProvince::PolarShield,
];

struct SiteAccum {
    center_elev: f64,
    min_elev: f64,
    max_elev: f64,
    sum_elev: f64,
    sum_slope: f64,
    max_slope: f64,
    dry_count: u32,
    buildable_count: u32,
    flat_count: u32,
    ocean_count: u32,
    river_count: u32,
    lake_count: u32,
    permafrost_count: u32,
    ice_count: u32,
    wetland_count: u32,
    has_freshwater: bool,
    has_navigable_river: bool,
    has_deepwater_port_access: bool,
    sum_excav_all: f64,
    sum_ug_all: f64,
    sum_excav_dry: f64,
    sum_ug_dry: f64,
    sum_flood_dry: f64,
    biome_counts: [u16; 14],
    province_counts: [u16; 8],
}

impl SiteAccum {
    fn new() -> Self {
        Self {
            center_elev: 0.0,
            min_elev: f64::INFINITY,
            max_elev: f64::NEG_INFINITY,
            sum_elev: 0.0,
            sum_slope: 0.0,
            max_slope: 0.0,
            dry_count: 0,
            buildable_count: 0,
            flat_count: 0,
            ocean_count: 0,
            river_count: 0,
            lake_count: 0,
            permafrost_count: 0,
            ice_count: 0,
            wetland_count: 0,
            has_freshwater: false,
            has_navigable_river: false,
            has_deepwater_port_access: false,
            sum_excav_all: 0.0,
            sum_ug_all: 0.0,
            sum_excav_dry: 0.0,
            sum_ug_dry: 0.0,
            sum_flood_dry: 0.0,
            biome_counts: [0; 14],
            province_counts: [0; 8],
        }
    }

    fn record_sample(&mut self, is_center: bool, s: TerrainSample, excav: f64, ug: f64) {
        if is_center {
            self.center_elev = s.elevation_m;
        }
        self.min_elev = self.min_elev.min(s.elevation_m);
        self.max_elev = self.max_elev.max(s.elevation_m);
        self.sum_elev += s.elevation_m;
        self.sum_slope += s.slope_percent;
        self.max_slope = self.max_slope.max(s.slope_percent);
        self.sum_excav_all += excav;
        self.sum_ug_all += ug;

        match s.water.kind {
            WaterBodyKind::Dry => self.record_dry(s, excav, ug),
            WaterBodyKind::Ocean => {
                self.ocean_count += 1;
                if is_deepwater_port_depth(s.elevation_m) {
                    self.has_deepwater_port_access = true;
                }
            }
            WaterBodyKind::River(_) => {
                self.river_count += 1;
                if s.water.is_freshwater {
                    self.has_freshwater = true;
                }
                if s.water.navigability != Navigability::Unnavigable {
                    self.has_navigable_river = true;
                }
                if s.water.navigability == Navigability::SeaGoingEstuary {
                    self.has_deepwater_port_access = true;
                }
            }
            WaterBodyKind::Lake(_) => {
                self.lake_count += 1;
                if s.water.is_freshwater {
                    self.has_freshwater = true;
                }
            }
        }

        if s.permafrost {
            self.permafrost_count += 1;
        }
        if s.biome == Biome::PolarIce || s.surface_material == SurfaceMaterial::IceSheet {
            self.ice_count += 1;
        }
        self.biome_counts[biome_index(s.biome)] += 1;
        self.province_counts[usize::from(s.province.id()).min(7)] += 1;
    }

    fn record_dry(&mut self, s: TerrainSample, excav: f64, ug: f64) {
        self.dry_count += 1;
        if s.slope_percent <= 12.0 {
            self.buildable_count += 1;
        }
        if s.slope_percent <= 3.0 {
            self.flat_count += 1;
        }
        self.sum_excav_dry += excav;
        self.sum_ug_dry += ug;
        let low = ((18.0 - s.elevation_m) / 18.0).clamp(0.0, 1.0);
        let rain = (f64::from(s.climate.annual_precip_mm) / 1400.0).clamp(0.0, 1.15);
        let flat = (1.0 - s.slope_percent / 8.0).clamp(0.0, 1.0);
        let mut local_flood = low * rain * flat * 0.85;
        if s.biome == Biome::CoastalWetland {
            local_flood += 0.35;
            self.wetland_count += 1;
        }
        self.sum_flood_dry += local_flood.clamp(0.0, 1.0);
    }
}

pub(crate) fn assess_site_impl(
    world: &TerrainWorld,
    center_unit: EcefMeters,
    radius_m: f64,
) -> Result<SiteAssessment, TerrainError> {
    if !radius_m.is_finite() || radius_m <= 0.0 {
        return Err(TerrainError::InvalidSiteRadius { radius_m });
    }
    let center = center_unit.normalized()?;
    let points = build_ring_points(center, radius_m, world.planet().radius_meters());
    let mut acc = SiteAccum::new();

    for (i, &pt) in points.iter().enumerate() {
        let s = world.sample_terrain(pt);
        let strata = world.sample_strata_column(pt);
        acc.record_sample(
            i == 0,
            s,
            f64::from(strata.excavation_cost_multiplier(10.0)),
            f64::from(strata.underground_suitability(60.0)),
        );
    }

    Ok(finalize_assessment(center, &acc))
}

fn build_ring_points(
    center: EcefMeters,
    radius_m: f64,
    planet_radius_m: f64,
) -> [EcefMeters; SAMPLE_COUNT] {
    let (east, north) = tangent_basis(center);
    let mut points = [center; SAMPLE_COUNT];
    let mut idx = 1_usize;
    for k in 0..8 {
        let theta = TAU * (f64::from(k) / 8.0);
        points[idx] = geodesic_offset(
            center,
            east,
            north,
            0.45 * radius_m / planet_radius_m,
            theta,
        );
        idx += 1;
    }
    for k in 0..16 {
        let theta = TAU * ((f64::from(k) + 0.5) / 16.0);
        points[idx] = geodesic_offset(
            center,
            east,
            north,
            0.90 * radius_m / planet_radius_m,
            theta,
        );
        idx += 1;
    }
    points
}

#[expect(
    clippy::cast_possible_truncation,
    reason = "aggregate fractions and scores intentionally narrow to f32"
)]
fn finalize_assessment(center: EcefMeters, acc: &SiteAccum) -> SiteAssessment {
    let n_f = f64::from(SAMPLE_COUNT_U32);
    let mean_elevation_m = acc.sum_elev / n_f;
    let mean_slope_percent = acc.sum_slope / n_f;

    let (excavation_difficulty, underground_suitability, mut flood_risk) = if acc.dry_count > 0 {
        let d_f = f64::from(acc.dry_count);
        (
            (acc.sum_excav_dry / d_f) as f32,
            (acc.sum_ug_dry / d_f) as f32,
            (acc.sum_flood_dry / d_f) as f32,
        )
    } else {
        (
            (acc.sum_excav_all / n_f) as f32,
            (acc.sum_ug_all / n_f) as f32,
            0.0,
        )
    };

    if acc.dry_count > 0 && acc.river_count > 0 {
        let river_boost = if mean_elevation_m < 120.0 && mean_slope_percent < 8.0 {
            0.42
        } else {
            0.18
        };
        flood_risk = (flood_risk + river_boost).clamp(0.0, 1.0);
    }

    let dominant_biome = BIOME_VARIANTS[argmax(&acc.biome_counts)];
    let dominant_province = PROVINCE_VARIANTS[argmax(&acc.province_counts)];
    let geographic_tags = build_site_tags(
        center,
        acc,
        mean_elevation_m,
        underground_suitability,
        flood_risk,
        dominant_biome,
    );

    SiteAssessment {
        center_elevation_m: acc.center_elev,
        min_elevation_m: acc.min_elev,
        max_elevation_m: acc.max_elev,
        mean_elevation_m,
        mean_slope_percent,
        max_slope_percent: acc.max_slope,
        dry_land_fraction: (f64::from(acc.dry_count) / n_f) as f32,
        buildable_land_fraction: (f64::from(acc.buildable_count) / n_f) as f32,
        flat_airfield_fraction: (f64::from(acc.flat_count) / n_f) as f32,
        has_freshwater: acc.has_freshwater,
        has_navigable_river: acc.has_navigable_river,
        has_deepwater_port_access: acc.has_deepwater_port_access,
        excavation_difficulty,
        underground_suitability,
        flood_risk,
        dominant_biome,
        dominant_province,
        geographic_tags,
    }
}

fn build_site_tags(
    center: EcefMeters,
    acc: &SiteAccum,
    mean_elevation_m: f64,
    underground_suitability: f32,
    flood_risk: f32,
    dominant_biome: Biome,
) -> GeographicTags {
    let lat_deg = center.y.clamp(-1.0, 1.0).asin().to_degrees();
    let mut tags = GeographicTags::empty();
    if lat_deg >= 66.5 {
        tags.insert(GeographicTags::ARCTIC);
    } else if lat_deg <= -66.5 {
        tags.insert(GeographicTags::ANTARCTIC);
    }
    if (acc.dry_count > 0 && acc.ocean_count > 0) || acc.wetland_count > 0 {
        tags.insert(GeographicTags::COASTAL);
    }
    if acc.river_count > 0 {
        tags.insert(GeographicTags::RIVERINE);
    }
    if acc.lake_count > 0 {
        tags.insert(GeographicTags::LAKESHORE);
    }
    if mean_elevation_m >= 1500.0 || acc.max_elev >= 2200.0 {
        tags.insert(GeographicTags::HIGHLAND);
    }
    if matches!(dominant_biome, Biome::HotDesert | Biome::ColdDesert) {
        tags.insert(GeographicTags::DESERT);
    }
    if acc.permafrost_count > 0 {
        tags.insert(GeographicTags::PERMAFROST);
    }
    if acc.has_deepwater_port_access {
        tags.insert(GeographicTags::DEEPWATER_PORT);
    }
    if acc.dry_count > 0 && underground_suitability >= 0.60 {
        tags.insert(GeographicTags::UNDERGROUND_FAVORABLE);
    }
    if acc.dry_count > 0 && flood_risk >= 0.45 {
        tags.insert(GeographicTags::FLOOD_PRONE);
    }
    if dominant_biome == Biome::PolarIce || acc.ice_count >= 12 {
        tags.insert(GeographicTags::GLACIATED);
    }
    tags
}
