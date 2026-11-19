use std::collections::HashMap;

use strata_globe::{
    EcefMeters, MAX_SUPPORTED_LOD, TileId, TileUv, ecef_to_face_uv, face_uv_to_tile_uv,
    face_uv_to_unit,
};

use crate::macro_climate::{angular_distance, point_to_arc_distance_rad};
use crate::patches_spatial::{
    Lod5Bucket, PATCH_BUCKET_LOD, PORTAL_PAD_BLEND_M, PORTAL_PAD_RADIUS_M, PatchEnvelope,
    PreparedKind, PreparedPatch,
};

pub(crate) fn lod5_tile_for_face_uv(
    face: strata_globe::CubeFace,
    face_uv: strata_globe::FaceUv,
) -> TileId {
    let (x, y, _) = face_uv_to_tile_uv(PATCH_BUCKET_LOD, face_uv);
    TileId {
        face,
        lod: PATCH_BUCKET_LOD,
        x,
        y,
    }
}

pub(crate) fn lod5_tile_for_unit(unit: EcefMeters) -> Option<TileId> {
    let Ok((face, face_uv)) = ecef_to_face_uv(unit) else {
        return None;
    };
    Some(lod5_tile_for_face_uv(face, face_uv))
}

pub(crate) fn lod5_ancestor(tile: TileId) -> TileId {
    let clamped_lod = tile.lod.min(MAX_SUPPORTED_LOD);
    let shift = clamped_lod.saturating_sub(PATCH_BUCKET_LOD);
    TileId {
        face: tile.face,
        lod: PATCH_BUCKET_LOD,
        x: tile.x >> shift,
        y: tile.y >> shift,
    }
}

pub(crate) fn rebuild_spatial_index(
    prepared: &[PreparedPatch],
) -> (
    HashMap<TileId, Lod5Bucket>,
    HashMap<TileId, PatchEnvelope>,
    PatchEnvelope,
) {
    let mut lod5_buckets: HashMap<TileId, Lod5Bucket> = HashMap::new();
    for (idx, patch) in prepared.iter().enumerate() {
        for &tile in &patch.affected_tiles {
            let entry = lod5_buckets.entry(tile).or_insert_with(|| Lod5Bucket {
                patch_indices: Vec::new(),
                envelope: PatchEnvelope::IDENTITY,
            });
            entry.patch_indices.push(idx);
            entry.envelope.accumulate_patch(patch);
        }
    }

    let mut coarse_envelopes: HashMap<TileId, PatchEnvelope> = HashMap::new();
    let mut global_envelope = PatchEnvelope::IDENTITY;
    for (&tile, bucket) in &lod5_buckets {
        global_envelope.union_spatial(bucket.envelope);
        let mut cur = tile.parent();
        while let Some(parent) = cur {
            let env = coarse_envelopes
                .entry(parent)
                .or_insert(PatchEnvelope::IDENTITY);
            env.union_spatial(bucket.envelope);
            cur = parent.parent();
        }
    }

    (lod5_buckets, coarse_envelopes, global_envelope)
}

pub(crate) fn collect_affected_lod5_tiles(
    kind: &PreparedKind,
    planet_radius_m: f64,
) -> Vec<TileId> {
    let mut tiles = Vec::new();
    for root in TileId::roots() {
        collect_recursive(root, kind, planet_radius_m, &mut tiles);
    }
    match *kind {
        PreparedKind::FlattenPad { center_unit, .. }
        | PreparedKind::ExcavationPit { center_unit, .. } => {
            if let Some(t) = lod5_tile_for_unit(center_unit) {
                tiles.push(t);
            }
        }
        PreparedKind::CorridorGrade {
            start_unit,
            end_unit,
            ..
        } => {
            if let Some(t) = lod5_tile_for_unit(start_unit) {
                tiles.push(t);
            }
            if let Some(t) = lod5_tile_for_unit(end_unit) {
                tiles.push(t);
            }
        }
        PreparedKind::UndergroundVoid {
            center_unit,
            portal_unit,
            ..
        } => {
            if let Some(t) = lod5_tile_for_unit(center_unit) {
                tiles.push(t);
            }
            if let Some(p) = portal_unit
                && let Some(t) = lod5_tile_for_unit(p)
            {
                tiles.push(t);
            }
        }
    }
    tiles.sort();
    tiles.dedup();
    tiles
}

fn collect_recursive(
    tile: TileId,
    kind: &PreparedKind,
    planet_radius_m: f64,
    out: &mut Vec<TileId>,
) {
    if !tile_overlaps_patch(tile, kind, planet_radius_m) {
        return;
    }
    if tile.lod == PATCH_BUCKET_LOD {
        out.push(tile);
        return;
    }
    if let Some(children) = tile.children() {
        for child in children {
            collect_recursive(child, kind, planet_radius_m, out);
        }
    }
}

fn tile_overlaps_patch(tile: TileId, kind: &PreparedKind, planet_radius_m: f64) -> bool {
    let (center, bound_rad) = tile_bounding_cap(tile);
    let pad = 1e-6;
    match *kind {
        PreparedKind::FlattenPad {
            center_unit,
            radius_m,
            blend_width_m,
            ..
        }
        | PreparedKind::ExcavationPit {
            center_unit,
            radius_m,
            blend_width_m,
            ..
        } => {
            let patch_rad = (radius_m + blend_width_m) / planet_radius_m;
            angular_distance(center, center_unit) <= bound_rad + patch_rad + pad
        }
        PreparedKind::CorridorGrade {
            start_unit,
            end_unit,
            half_width_m,
            blend_width_m,
            ..
        } => {
            let patch_rad = (half_width_m + blend_width_m) / planet_radius_m;
            point_to_arc_distance_rad(center, start_unit, end_unit) <= bound_rad + patch_rad + pad
        }
        PreparedKind::UndergroundVoid {
            center_unit,
            radius_m,
            portal_unit,
            ..
        } => {
            let void_rad = radius_m / planet_radius_m;
            let hits_void = angular_distance(center, center_unit) <= bound_rad + void_rad + pad;
            let hits_portal = portal_unit.is_some_and(|p| {
                let p_rad = (PORTAL_PAD_RADIUS_M + PORTAL_PAD_BLEND_M) / planet_radius_m;
                angular_distance(center, p) <= bound_rad + p_rad + pad
            });
            hits_void || hits_portal
        }
    }
}

fn tile_bounding_cap(tile: TileId) -> (EcefMeters, f64) {
    let center = face_uv_to_unit(
        tile.face,
        tile.tile_uv_to_face_uv(TileUv::clamped(0.5, 0.5)),
    );
    let corners = [
        TileUv::clamped(0.0, 0.0),
        TileUv::clamped(1.0, 0.0),
        TileUv::clamped(0.0, 1.0),
        TileUv::clamped(1.0, 1.0),
    ];
    let mut max_rad = 0.0_f64;
    for uv in corners {
        let p = face_uv_to_unit(tile.face, tile.tile_uv_to_face_uv(uv));
        max_rad = max_rad.max(angular_distance(center, p));
    }
    (center, max_rad)
}
