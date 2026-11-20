use strata_globe::{EcefMeters, ElevationRange, TileId};

use crate::macro_climate::closest_on_unit_segment;
use crate::macro_climate::{angular_distance, smoothstep};
use crate::patches::PatchId;

pub(crate) const PATCH_BUCKET_LOD: u8 = 5;
pub(crate) const PORTAL_PAD_RADIUS_M: f64 = 25.0;
pub(crate) const PORTAL_PAD_BLEND_M: f64 = 20.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct PatchEnvelope {
    pub abs_min: f64,
    pub abs_max: f64,
    pub min_delta: f64,
    pub max_delta: f64,
}

impl PatchEnvelope {
    pub(crate) const IDENTITY: Self = Self {
        abs_min: f64::INFINITY,
        abs_max: f64::NEG_INFINITY,
        min_delta: 0.0,
        max_delta: 0.0,
    };

    pub(crate) fn accumulate_patch(&mut self, patch: &PreparedPatch) {
        match patch.kind {
            PreparedKind::FlattenPad {
                target_elevation_m, ..
            } => {
                self.abs_min = self.abs_min.min(target_elevation_m);
                self.abs_max = self.abs_max.max(target_elevation_m);
            }
            PreparedKind::CorridorGrade {
                start_elevation_m,
                end_elevation_m,
                ..
            } => {
                self.abs_min = self.abs_min.min(start_elevation_m.min(end_elevation_m));
                self.abs_max = self.abs_max.max(start_elevation_m.max(end_elevation_m));
            }
            PreparedKind::ExcavationPit { depth_m, .. } => {
                self.min_delta -= depth_m;
            }
            PreparedKind::UndergroundVoid {
                portal_target_elev_m: Some(target_m),
                ..
            } => {
                self.abs_min = self.abs_min.min(target_m);
                self.abs_max = self.abs_max.max(target_m);
            }
            PreparedKind::UndergroundVoid {
                portal_target_elev_m: None,
                ..
            } => {}
        }
    }

    pub(crate) fn union_spatial(&mut self, other: Self) {
        self.abs_min = self.abs_min.min(other.abs_min);
        self.abs_max = self.abs_max.max(other.abs_max);
        self.min_delta = self.min_delta.min(other.min_delta);
        self.max_delta = self.max_delta.max(other.max_delta);
    }

    #[must_use]
    pub(crate) fn apply_to_range(self, base: ElevationRange) -> ElevationRange {
        let min_meters = base.min_meters.min(self.abs_min) + self.min_delta - 1e-3;
        let max_meters = base.max_meters.max(self.abs_max) + self.max_delta + 1e-3;
        ElevationRange {
            min_meters,
            max_meters: max_meters.max(min_meters),
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct Lod5Bucket {
    pub patch_indices: Vec<usize>,
    pub envelope: PatchEnvelope,
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum PreparedKind {
    FlattenPad {
        center_unit: EcefMeters,
        radius_m: f64,
        blend_width_m: f64,
        target_elevation_m: f64,
    },
    CorridorGrade {
        start_unit: EcefMeters,
        end_unit: EcefMeters,
        total_arc_rad: f64,
        half_width_m: f64,
        blend_width_m: f64,
        start_elevation_m: f64,
        end_elevation_m: f64,
    },
    ExcavationPit {
        center_unit: EcefMeters,
        radius_m: f64,
        blend_width_m: f64,
        depth_m: f64,
    },
    UndergroundVoid {
        center_unit: EcefMeters,
        radius_m: f64,
        top_depth_m: f64,
        bottom_depth_m: f64,
        portal_unit: Option<EcefMeters>,
        portal_target_elev_m: Option<f64>,
    },
}

#[derive(Debug, Clone)]
pub(crate) struct PreparedPatch {
    pub id: PatchId,
    pub kind: PreparedKind,
    pub affected_tiles: Vec<TileId>,
}

impl PreparedPatch {
    #[inline]
    pub(crate) fn apply_elevation(
        &self,
        unit: EcefMeters,
        planet_radius_m: f64,
        elev_m: f64,
    ) -> f64 {
        match self.kind {
            PreparedKind::FlattenPad {
                center_unit,
                radius_m,
                blend_width_m,
                target_elevation_m,
            } => {
                let dist_m = angular_distance(unit, center_unit) * planet_radius_m;
                let w = radial_weight(dist_m, radius_m, blend_width_m);
                if w > 0.0 {
                    elev_m + (target_elevation_m - elev_m) * w
                } else {
                    elev_m
                }
            }
            PreparedKind::CorridorGrade {
                start_unit,
                end_unit,
                total_arc_rad,
                half_width_m,
                blend_width_m,
                start_elevation_m,
                end_elevation_m,
            } => {
                let (closest, _) = closest_on_unit_segment(unit, start_unit, end_unit);
                let dist_m = angular_distance(unit, closest) * planet_radius_m;
                let w = radial_weight(dist_m, half_width_m, blend_width_m);
                if w > 0.0 {
                    let s_arc =
                        (angular_distance(start_unit, closest) / total_arc_rad).clamp(0.0, 1.0);
                    let target = start_elevation_m + (end_elevation_m - start_elevation_m) * s_arc;
                    elev_m + (target - elev_m) * w
                } else {
                    elev_m
                }
            }
            PreparedKind::ExcavationPit {
                center_unit,
                radius_m,
                blend_width_m,
                depth_m,
            } => {
                let dist_m = angular_distance(unit, center_unit) * planet_radius_m;
                let w = radial_weight(dist_m, radius_m, blend_width_m);
                elev_m - depth_m * w
            }
            PreparedKind::UndergroundVoid {
                portal_unit: Some(portal),
                portal_target_elev_m: Some(target),
                ..
            } => {
                let dist_m = angular_distance(unit, portal) * planet_radius_m;
                let w = radial_weight(dist_m, PORTAL_PAD_RADIUS_M, PORTAL_PAD_BLEND_M);
                if w > 0.0 {
                    elev_m + (target - elev_m) * w
                } else {
                    elev_m
                }
            }
            PreparedKind::UndergroundVoid { .. } => elev_m,
        }
    }

    #[inline]
    pub(crate) fn contains_void_at(
        &self,
        unit: EcefMeters,
        planet_radius_m: f64,
        depth_m: f64,
    ) -> bool {
        if let PreparedKind::UndergroundVoid {
            center_unit,
            radius_m,
            top_depth_m,
            bottom_depth_m,
            ..
        } = self.kind
        {
            depth_m >= top_depth_m
                && depth_m <= bottom_depth_m
                && angular_distance(unit, center_unit) * planet_radius_m <= radius_m
        } else {
            false
        }
    }

    #[inline]
    pub(crate) fn contains_any_void(&self, unit: EcefMeters, planet_radius_m: f64) -> bool {
        if let PreparedKind::UndergroundVoid {
            center_unit,
            radius_m,
            ..
        } = self.kind
        {
            angular_distance(unit, center_unit) * planet_radius_m <= radius_m
        } else {
            false
        }
    }
}

#[inline]
fn radial_weight(dist_m: f64, inner_m: f64, blend_m: f64) -> f64 {
    if dist_m <= inner_m {
        1.0
    } else if dist_m < inner_m + blend_m {
        smoothstep(inner_m + blend_m, inner_m, dist_m)
    } else {
        0.0
    }
}
