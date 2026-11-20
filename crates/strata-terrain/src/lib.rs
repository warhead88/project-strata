//! Pure-Rust planetary terrain, hydrology, climate, and biome library for Project Strata.
//!
//! `strata-terrain` runs without a graphics stack and provides `O(1)`
//! zero-allocation queries for elevation, conservative tile bounds, oceans/lakes/rivers,
//! climate, biomes, and physical-geographic surface colors.

pub mod climate;
pub mod dem;
mod earth_bounds;
mod earth_data;
mod earth_dem;
pub mod earth_geography;
mod earth_pack;
mod earth_water;
pub mod error;
pub mod hydrology;
pub mod macro_cache;
mod macro_climate;
mod macro_mipmap;
pub mod noise;
pub mod patches;
mod patches_index;
mod patches_spatial;
mod patches_validate;
pub mod query;
mod site_assessment;
pub mod strata;
mod strata_derive;
mod strata_layers;
mod strata_minerals;
mod surface_data;
pub mod terrain_config;
pub mod terrain_world;
mod terrain_world_queries;
mod water_catalog;
pub use earth_water::EarthWaterRaster;
pub use surface_data::{SurfaceData, SurfaceRaster};

pub use climate::{Biome, ClimateSample, classify_biome, surface_albedo_rgb};
pub use dem::DemGrid;
pub use earth_data::{
    PROVINCE_CONTINENTAL_SHELF, PROVINCE_CRATON, PROVINCE_OCEANIC_ABYSS, PROVINCE_OLD_OROGEN_RIFT,
    PROVINCE_POLAR_SHIELD, PROVINCE_SEDIMENTARY_BASIN, PROVINCE_SUBDUCTION_ARC,
    PROVINCE_YOUNG_OROGEN,
};
pub use earth_geography::{MacroSample, TerrainSource, lat_lon_to_unit};
pub use error::TerrainError;
pub use hydrology::{
    BathymetryZone, LakeDef, LakeId, Navigability, RiverDef, RiverId, RiverWaypoint,
    SEA_LEVEL_METERS, Salinity, WaterBodyKind, WaterSample, bathymetry_zone,
    is_deepwater_port_depth,
};
pub use macro_cache::{DEFAULT_MACRO_RESOLUTION, MacroCell};
pub use noise::{fbm3, noise3, ridged_fbm3};
pub use patches::{PatchId, PatchReceipt, TerrainPatch, TerrainPatchKind};
pub use query::{
    GeographicTags, RenderVertexSample, SiteAssessment, TerrainSample, TileVertexBase,
};
pub use strata::{
    BedrockKind, GeologyProvince, MAX_STRATA_DEPTH_M, MAX_STRATA_LAYERS, MineralPotential,
    StrataColumn, StratumLayer, SurfaceMaterial,
};
pub use terrain_config::{DEFAULT_EARTH_SEED, TerrainConfig};
pub use terrain_world::TerrainWorld;
