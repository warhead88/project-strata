use strata_globe::EcefMeters;

use crate::error::TerrainError;
use crate::macro_climate::angular_distance;
use crate::patches::{PatchId, TerrainPatchKind};
use crate::patches_index::collect_affected_lod5_tiles;
use crate::patches_spatial::{PreparedKind, PreparedPatch};

pub(crate) fn validate_and_prepare(
    id: PatchId,
    kind: TerrainPatchKind,
    planet_radius_m: f64,
    portal_elev_sampler: impl FnOnce(EcefMeters) -> f64,
) -> Result<PreparedPatch, TerrainError> {
    let prepared_kind = match kind {
        TerrainPatchKind::FlattenPad {
            center_unit,
            radius_m,
            blend_width_m,
            target_elevation_m,
        } => prepare_flatten_pad(center_unit, radius_m, blend_width_m, target_elevation_m)?,
        TerrainPatchKind::CorridorGrade {
            start_unit,
            end_unit,
            half_width_m,
            blend_width_m,
            start_elevation_m,
            end_elevation_m,
        } => prepare_corridor(
            start_unit,
            end_unit,
            half_width_m,
            blend_width_m,
            start_elevation_m,
            end_elevation_m,
        )?,
        TerrainPatchKind::ExcavationPit {
            center_unit,
            radius_m,
            blend_width_m,
            depth_m,
        } => prepare_pit(center_unit, radius_m, blend_width_m, depth_m)?,
        TerrainPatchKind::UndergroundVoid {
            center_unit,
            radius_m,
            top_depth_m,
            bottom_depth_m,
            portal_surface_unit,
        } => prepare_void(
            center_unit,
            radius_m,
            top_depth_m,
            bottom_depth_m,
            portal_surface_unit,
            portal_elev_sampler,
        )?,
    };

    let affected_tiles = collect_affected_lod5_tiles(&prepared_kind, planet_radius_m);
    Ok(PreparedPatch {
        id,
        kind: prepared_kind,
        affected_tiles,
    })
}

fn prepare_flatten_pad(
    center_unit: EcefMeters,
    radius_m: f64,
    blend_width_m: f64,
    target_elevation_m: f64,
) -> Result<PreparedKind, TerrainError> {
    let center_unit = center_unit.normalized()?;
    validate_positive_finite(radius_m, "flatten pad radius_m must be finite and > 0")?;
    validate_positive_finite(
        blend_width_m,
        "flatten pad blend_width_m must be finite and > 0",
    )?;
    if !target_elevation_m.is_finite() {
        return Err(TerrainError::NonFiniteCoordinate);
    }
    Ok(PreparedKind::FlattenPad {
        center_unit,
        radius_m,
        blend_width_m,
        target_elevation_m,
    })
}

fn prepare_corridor(
    start_unit: EcefMeters,
    end_unit: EcefMeters,
    half_width_m: f64,
    blend_width_m: f64,
    start_elevation_m: f64,
    end_elevation_m: f64,
) -> Result<PreparedKind, TerrainError> {
    let start_unit = start_unit.normalized()?;
    let end_unit = end_unit.normalized()?;
    let total_arc_rad = angular_distance(start_unit, end_unit);
    if total_arc_rad <= 1e-9 {
        return Err(TerrainError::InvalidPatch {
            reason: "corridor start_unit and end_unit must not coincide",
        });
    }
    validate_positive_finite(half_width_m, "corridor half_width_m must be finite and > 0")?;
    validate_positive_finite(
        blend_width_m,
        "corridor blend_width_m must be finite and > 0",
    )?;
    if !start_elevation_m.is_finite() || !end_elevation_m.is_finite() {
        return Err(TerrainError::NonFiniteCoordinate);
    }
    Ok(PreparedKind::CorridorGrade {
        start_unit,
        end_unit,
        total_arc_rad,
        half_width_m,
        blend_width_m,
        start_elevation_m,
        end_elevation_m,
    })
}

fn prepare_pit(
    center_unit: EcefMeters,
    radius_m: f64,
    blend_width_m: f64,
    depth_m: f64,
) -> Result<PreparedKind, TerrainError> {
    let center_unit = center_unit.normalized()?;
    validate_positive_finite(radius_m, "excavation pit radius_m must be finite and > 0")?;
    validate_positive_finite(
        blend_width_m,
        "excavation pit blend_width_m must be finite and > 0",
    )?;
    validate_positive_finite(depth_m, "excavation pit depth_m must be finite and > 0")?;
    Ok(PreparedKind::ExcavationPit {
        center_unit,
        radius_m,
        blend_width_m,
        depth_m,
    })
}

fn prepare_void(
    center_unit: EcefMeters,
    radius_m: f64,
    top_depth_m: f64,
    bottom_depth_m: f64,
    portal_surface_unit: Option<EcefMeters>,
    portal_elev_sampler: impl FnOnce(EcefMeters) -> f64,
) -> Result<PreparedKind, TerrainError> {
    let center_unit = center_unit.normalized()?;
    validate_positive_finite(radius_m, "underground void radius_m must be finite and > 0")?;
    if !top_depth_m.is_finite() || !bottom_depth_m.is_finite() {
        return Err(TerrainError::NonFiniteCoordinate);
    }
    if top_depth_m < 0.0 || bottom_depth_m <= top_depth_m {
        return Err(TerrainError::InvalidPatch {
            reason: "underground void depths must satisfy 0.0 <= top_depth_m < bottom_depth_m",
        });
    }
    let portal_unit = match portal_surface_unit {
        Some(u) => Some(u.normalized()?),
        None => None,
    };
    let portal_target_elev_m = portal_unit.map(portal_elev_sampler);
    Ok(PreparedKind::UndergroundVoid {
        center_unit,
        radius_m,
        top_depth_m,
        bottom_depth_m,
        portal_unit,
        portal_target_elev_m,
    })
}

fn validate_positive_finite(value: f64, reason: &'static str) -> Result<(), TerrainError> {
    if !value.is_finite() {
        return Err(TerrainError::NonFiniteCoordinate);
    }
    if value <= 0.0 {
        return Err(TerrainError::InvalidPatch { reason });
    }
    Ok(())
}
