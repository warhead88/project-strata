use std::f64::consts::TAU;
use std::ops::{BitOr, BitOrAssign};

use strata_globe::EcefMeters;

use crate::climate::{Biome, ClimateSample};
use crate::hydrology::{BathymetryZone, WaterSample};
use crate::strata::{GeologyProvince, StrataColumn, SurfaceMaterial};
use crate::terrain_world::TerrainWorld;

/// Bitflags describing notable physical-geographic and engineering site tags.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct GeographicTags(u16);

impl GeographicTags {
    /// Located within the Arctic Circle (`>= +66.5 deg`).
    pub const ARCTIC: Self = Self(1 << 0);
    /// Located within the Antarctic Circle (`<= -66.5 deg`).
    pub const ANTARCTIC: Self = Self(1 << 1);
    /// Site spans or borders an ocean coastline.
    pub const COASTAL: Self = Self(1 << 2);
    /// Site intersects a river channel.
    pub const RIVERINE: Self = Self(1 << 3);
    /// Site borders or includes an inland lake.
    pub const LAKESHORE: Self = Self(1 << 4);
    /// High-altitude terrain (`mean >= 1500 m` or `max >= 2200 m`).
    pub const HIGHLAND: Self = Self(1 << 5);
    /// Hyper-arid hot or cold desert environment.
    pub const DESERT: Self = Self(1 << 6);
    /// Ground includes permanently frozen permafrost.
    pub const PERMAFROST: Self = Self(1 << 7);
    /// Adjacent marine depths satisfy deepwater port draft bounds (`-200 .. -12 m`).
    pub const DEEPWATER_PORT: Self = Self(1 << 8);
    /// Subsurface bedrock offers high structural stability and low permeability.
    pub const UNDERGROUND_FAVORABLE: Self = Self(1 << 9);
    /// Low-lying or riverine terrain with elevated flood risk.
    pub const FLOOD_PRONE: Self = Self(1 << 10);
    /// Covered primarily by polar/continental ice sheet.
    pub const GLACIATED: Self = Self(1 << 11);

    /// Returns an empty tag set.
    #[must_use]
    pub const fn empty() -> Self {
        Self(0)
    }

    /// Returns the raw `u16` bitmask.
    #[must_use]
    pub const fn bits(self) -> u16 {
        self.0
    }

    /// Returns `true` if all bits in `other` are present in `self`.
    #[must_use]
    pub const fn contains(self, other: Self) -> bool {
        (self.0 & other.0) == other.0
    }

    /// Sets all bits from `other` in `self`.
    pub fn insert(&mut self, other: Self) {
        self.0 |= other.0;
    }

    /// Returns `true` if no tags are set.
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }
}

impl BitOr for GeographicTags {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self::Output {
        Self(self.0 | rhs.0)
    }
}

impl BitOrAssign for GeographicTags {
    fn bitor_assign(&mut self, rhs: Self) {
        self.0 |= rhs.0;
    }
}

/// Complete point-sampled physical-geographic, geological, and cryosphere state.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TerrainSample {
    /// Surface elevation in meters relative to sea level (after patches).
    pub elevation_m: f64,
    /// Local terrain slope angle in radians (`0.0 .. pi/2`).
    pub slope_rad: f64,
    /// Local terrain grade in percent (`tan(slope_rad) * 100.0`).
    pub slope_percent: f64,
    /// Downhill azimuth angle in radians clockwise from North (`0.0 .. 2*pi`).
    pub aspect_rad: f64,
    /// Macro terrain roughness factor in `[0.0, 1.0]`.
    pub roughness: f32,
    /// Hydrological state at this point.
    pub water: WaterSample,
    /// Bathymetric zone classification.
    pub bathymetry: BathymetryZone,
    /// Climate and cryosphere indicators.
    pub climate: ClimateSample,
    /// Physical-geographic biome.
    pub biome: Biome,
    /// Macro geological province.
    pub province: GeologyProvince,
    /// Surface regolith, soil, rock, or ice material.
    pub surface_material: SurfaceMaterial,
    /// `true` when permanently frozen ground is present.
    pub permafrost: bool,
    /// `true` when an active [`crate::TerrainPatchKind::UndergroundVoid`] exists beneath this point.
    pub has_underground_void: bool,
}

/// Single-lookup render vertex bundle for client tile meshing (`Step 3`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RenderVertexSample {
    /// Final surface elevation in meters (including hydrology and patches).
    pub elevation_m: f64,
    /// Physical-geographic surface albedo RGB in `[0.0, 1.0]^3`.
    pub albedo_rgb: [f32; 3],
    /// Hydrological state at this vertex.
    pub water: WaterSample,
}

/// Aggregate engineering, hydrological, and geographic assessment over a circular site.
#[derive(Debug, Clone, PartialEq)]
pub struct SiteAssessment {
    /// Elevation at the site center in meters.
    pub center_elevation_m: f64,
    /// Minimum sampled elevation within the site radius in meters.
    pub min_elevation_m: f64,
    /// Maximum sampled elevation within the site radius in meters.
    pub max_elevation_m: f64,
    /// Mean sampled elevation across the site in meters.
    pub mean_elevation_m: f64,
    /// Mean terrain grade across the site in percent.
    pub mean_slope_percent: f64,
    /// Maximum sampled terrain grade across the site in percent.
    pub max_slope_percent: f64,
    /// Fraction of site area that is dry emergent land (`0.0..=1.0`).
    pub dry_land_fraction: f32,
    /// Fraction of site area suitable for general construction (dry and slope `<= 12%`).
    pub buildable_land_fraction: f32,
    /// Fraction of site area flat enough for airfields/runways (dry and slope `<= 3%`).
    pub flat_airfield_fraction: f32,
    /// `true` if a freshwater river or freshwater lake intersects the site.
    pub has_freshwater: bool,
    /// `true` if a navigable river intersects the site.
    pub has_navigable_river: bool,
    /// `true` if adjacent ocean depths satisfy deepwater port draft (`-200 .. -12 m`).
    pub has_deepwater_port_access: bool,
    /// Mean shallow excavation difficulty multiplier across the site.
    pub excavation_difficulty: f32,
    /// Mean subterranean construction suitability `[0.0, 1.0]`.
    pub underground_suitability: f32,
    /// Aggregate flood hazard score `[0.0, 1.0]`.
    pub flood_risk: f32,
    /// Most frequent biome across the site.
    pub dominant_biome: Biome,
    /// Most frequent geological province across the site.
    pub dominant_province: GeologyProvince,
    /// Derived geographic classification tags.
    pub geographic_tags: GeographicTags,
}

/// Computes canonical orthonormal `(east, north)` tangent vectors at unit direction `unit`.
#[must_use]
pub(crate) fn tangent_basis(unit: EcefMeters) -> (EcefMeters, EcefMeters) {
    let ref_up = if unit.y.abs() > 0.999 {
        EcefMeters::from_raw(0.0, 0.0, 1.0)
    } else {
        EcefMeters::from_raw(0.0, 1.0, 0.0)
    };
    let east_raw = ref_up.cross(unit);
    let east_len = east_raw.norm();
    let east = if east_len > 0.0 {
        east_raw.scale(1.0 / east_len)
    } else {
        EcefMeters::from_raw(1.0, 0.0, 0.0)
    };
    let north_raw = unit.cross(east);
    let north_len = north_raw.norm();
    let north = if north_len > 0.0 {
        north_raw.scale(1.0 / north_len)
    } else {
        EcefMeters::from_raw(0.0, 0.0, 1.0)
    };
    (east, north)
}

/// Evaluates `(slope_rad, slope_percent, aspect_rad)` via central tangent finite differences.
#[must_use]
pub(crate) fn compute_slope_and_aspect(world: &TerrainWorld, unit: EcefMeters) -> (f64, f64, f64) {
    let radius_m = world.planet().radius_meters();
    let delta_m = (radius_m * 5.0e-6).clamp(2.0, 25.0);
    let eps = delta_m / radius_m;
    let (east, north) = tangent_basis(unit);

    let e_pos = offset_unit(unit, east, eps);
    let e_neg = offset_unit(unit, east, -eps);
    let n_pos = offset_unit(unit, north, eps);
    let n_neg = offset_unit(unit, north, -eps);

    let slope_east = (world.sample_elevation_unit_prevalidated(e_pos)
        - world.sample_elevation_unit_prevalidated(e_neg))
        / (2.0 * delta_m);
    let slope_north = (world.sample_elevation_unit_prevalidated(n_pos)
        - world.sample_elevation_unit_prevalidated(n_neg))
        / (2.0 * delta_m);
    let grad = (slope_east * slope_east + slope_north * slope_north).sqrt();
    let slope_rad = grad.atan();
    let slope_percent = grad * 100.0;
    let aspect_rad = if grad > 1e-9 {
        (-slope_east).atan2(-slope_north).rem_euclid(TAU)
    } else {
        0.0
    };
    (slope_rad, slope_percent, aspect_rad)
}

#[inline]
fn offset_unit(base: EcefMeters, dir: EcefMeters, eps: f64) -> EcefMeters {
    let raw = base + dir.scale(eps);
    let len = raw.norm();
    if len > 0.0 {
        raw.scale(1.0 / len)
    } else {
        base
    }
}

pub(crate) fn geodesic_offset(
    center: EcefMeters,
    east: EcefMeters,
    north: EcefMeters,
    ang_rad: f64,
    theta_rad: f64,
) -> EcefMeters {
    let (sin_a, cos_a) = ang_rad.sin_cos();
    let (sin_t, cos_t) = theta_rad.sin_cos();
    let tangent = east.scale(cos_t) + north.scale(sin_t);
    let raw = center.scale(cos_a) + tangent.scale(sin_a);
    let len = raw.norm();
    if len > 0.0 {
        raw.scale(1.0 / len)
    } else {
        center
    }
}

pub(crate) const fn biome_index(biome: Biome) -> usize {
    match biome {
        Biome::PolarIce => 0,
        Biome::Tundra => 1,
        Biome::BorealForest => 2,
        Biome::TemperateForest => 3,
        Biome::TemperateGrassland => 4,
        Biome::Steppe => 5,
        Biome::HotDesert => 6,
        Biome::ColdDesert => 7,
        Biome::Savanna => 8,
        Biome::TropicalRainforest => 9,
        Biome::AlpineTundra => 10,
        Biome::AlpineRock => 11,
        Biome::CoastalWetland => 12,
        Biome::OpenWater => 13,
    }
}

pub(crate) fn argmax(counts: &[u16]) -> usize {
    let mut best_i = 0_usize;
    let mut best_v = 0_u16;
    for (i, &v) in counts.iter().enumerate() {
        if v > best_v {
            best_v = v;
            best_i = i;
        }
    }
    best_i
}

pub(crate) fn build_strata_column_for_unit(
    world: &TerrainWorld,
    unit: EcefMeters,
    elev: f64,
    water: WaterSample,
    cell: crate::macro_cache::MacroCell,
    seed: u64,
) -> StrataColumn {
    let climate = crate::climate::compute_climate_sample(cell, elev, water);
    let biome = crate::climate::classify_biome(elev, water, climate);
    let province = GeologyProvince::from_id(cell.geology_province);
    let (slope_rad, _, _) = compute_slope_and_aspect(world, unit);
    crate::strata_derive::derive_strata_column(
        unit,
        elev,
        slope_rad.sin(),
        water,
        climate,
        biome,
        province,
        seed,
    )
}

pub(crate) fn build_terrain_sample_for_unit(
    world: &TerrainWorld,
    unit: EcefMeters,
    elev: f64,
    water: WaterSample,
    cell: crate::macro_cache::MacroCell,
    has_underground_void: bool,
) -> TerrainSample {
    let climate = crate::climate::compute_climate_sample(cell, elev, water);
    let biome = crate::climate::classify_biome(elev, water, climate);
    let province = GeologyProvince::from_id(cell.geology_province);
    let (slope_rad, slope_percent, aspect_rad) = compute_slope_and_aspect(world, unit);
    let surface_material =
        crate::strata_derive::derive_surface_material(elev, slope_rad.sin(), water, climate, biome);
    let permafrost = water.kind == crate::hydrology::WaterBodyKind::Dry
        && (climate.mean_temp_c < -1.0 || climate.permafrost_prob > 0.15);

    TerrainSample {
        elevation_m: elev,
        slope_rad,
        slope_percent,
        aspect_rad,
        roughness: cell.roughness,
        water,
        bathymetry: crate::hydrology::bathymetry_zone(elev),
        climate,
        biome,
        province,
        surface_material,
        permafrost,
        has_underground_void,
    }
}

pub(crate) fn build_render_vertex_for_sample(
    elev: f64,
    slope_sin: f64,
    water: WaterSample,
    cell: crate::macro_cache::MacroCell,
) -> RenderVertexSample {
    let climate = crate::climate::compute_climate_sample(cell, elev, water);
    let biome = crate::climate::classify_biome(elev, water, climate);
    let albedo_rgb = crate::climate::surface_albedo_rgb(elev, slope_sin, water, climate, biome);
    RenderVertexSample {
        elevation_m: elev,
        albedo_rgb,
        water,
    }
}
