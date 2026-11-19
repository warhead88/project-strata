use strata_globe::{
    CubeFace, EcefMeters, ElevationRange, FaceUv, MAX_SUPPORTED_LOD, TileId, ecef_to_face_uv,
    face_uv_to_unit,
};

use crate::earth_geography::{ElevationGridSource, MacroSample, TerrainSource};
use crate::error::TerrainError;
use crate::hydrology_index::HydrologyIndex;
use crate::macro_climate::{evaluate_dem_macro, evaluate_earth_macro};
use crate::macro_mipmap::{MinMaxNode, build_mip_pyramid, compute_mip_layout};
use crate::noise::noise3;

/// Default per-face macro cache cell resolution (`6 x 128 x 128` cells).
pub const DEFAULT_MACRO_RESOLUTION: u32 = 128;

/// Maximum amplitude of fast 2-octave micro-detail noise at `roughness == 1.0`.
pub(crate) const MICRO_MAX_BOUND_M: f64 = 160.0;

/// Cached macro terrain and climate fields per grid vertex.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MacroCell {
    /// Macro elevation relative to sea level in meters.
    pub macro_elevation_m: f32,
    /// Dimensionless terrain roughness factor in `[0.0, 1.0]`.
    pub roughness: f32,
    /// Continentality factor in `[0.0, 1.0]`.
    pub continentality: f32,
    /// Mean annual surface temperature in degrees Celsius.
    pub mean_temp_c: f32,
    /// Mean annual precipitation in millimeters.
    pub annual_precip_mm: f32,
    /// Macro geology province ID (`0..=7`).
    pub geology_province: u8,
}

impl From<MacroSample> for MacroCell {
    fn from(s: MacroSample) -> Self {
        Self {
            macro_elevation_m: s.macro_elevation_m,
            roughness: s.roughness,
            continentality: s.continentality,
            mean_temp_c: s.mean_temp_c,
            annual_precip_mm: s.annual_precip_mm,
            geology_province: s.geology_province,
        }
    }
}

/// Pre-baked `6 x N x N` cube-face macro cache with boundary-exact `(N + 1) x (N + 1)`
/// sampling lattice and a per-face `N -> N/2 -> ... -> 1` min/max mipmap pyramid.
#[derive(Debug, Clone)]
pub(crate) struct CubeMacroCache {
    resolution: u32,
    lod_depth: u8,
    stride: usize,
    vertices: Vec<MacroCell>,
    level_offsets: Vec<usize>,
    mip_nodes: Vec<MinMaxNode>,
    face_stride_mip: usize,
    global_range: ElevationRange,
}

impl CubeMacroCache {
    /// Validates `resolution` (power of two in `16..=512`) and bakes the entire globe cache.
    pub(crate) fn bake(
        resolution: u32,
        source: &TerrainSource,
        elevation_source: &ElevationGridSource,
        hydrology: &HydrologyIndex,
    ) -> Result<Self, TerrainError> {
        if !(16..=512).contains(&resolution) || !resolution.is_power_of_two() {
            return Err(TerrainError::InvalidMacroResolution { resolution });
        }
        let lod_depth = u8::try_from(resolution.trailing_zeros()).unwrap_or(7);
        let n = resolution as usize;
        let stride = n + 1;
        let (vertices, vertex_bounds) = bake_vertices(resolution, source, elevation_source);
        let (level_offsets, total_mip_per_face) = compute_mip_layout(lod_depth);
        let (mip_nodes, global_range) = build_mip_pyramid(
            resolution,
            &vertices,
            &vertex_bounds,
            elevation_source,
            &level_offsets,
            total_mip_per_face,
            hydrology,
        )?;

        Ok(Self {
            resolution,
            lod_depth,
            stride,
            vertices,
            level_offsets,
            mip_nodes,
            face_stride_mip: total_mip_per_face,
            global_range,
        })
    }

    #[must_use]
    pub(crate) const fn resolution(&self) -> u32 {
        self.resolution
    }

    #[must_use]
    pub(crate) const fn global_range(&self) -> ElevationRange {
        self.global_range
    }

    /// Returns conservative elevation bounds for `tile` in strictly `O(1)` time.
    #[must_use]
    pub(crate) fn tile_range(&self, tile: TileId) -> ElevationRange {
        let clamped_lod = tile.lod.min(MAX_SUPPORTED_LOD);
        let (mip_lod, mx, my) = if clamped_lod <= self.lod_depth {
            (clamped_lod, tile.x as usize, tile.y as usize)
        } else {
            let shift = (clamped_lod - self.lod_depth).min(31);
            (
                self.lod_depth,
                (tile.x >> shift) as usize,
                (tile.y >> shift) as usize,
            )
        };
        let dim = 1_usize << mip_lod;
        let idx = tile.face.index() * self.face_stride_mip
            + self.level_offsets[usize::from(mip_lod)]
            + my.min(dim - 1) * dim
            + mx.min(dim - 1);
        let node = self.mip_nodes[idx];
        ElevationRange {
            min_meters: f64::from(node.min_m),
            max_meters: f64::from(node.max_m),
        }
    }

    /// Bilinearly samples the cached macro fields at `unit_dir` in strictly `O(1)` time.
    #[must_use]
    pub(crate) fn sample_bilinear(&self, unit_dir: EcefMeters) -> MacroCell {
        let Ok((face, uv)) = ecef_to_face_uv(unit_dir) else {
            return MacroCell::from(default_fallback_sample());
        };
        self.sample_bilinear_uv(face, uv)
    }

    /// Bilinearly samples the cached macro fields using precomputed `(face, uv)`.
    #[inline]
    #[must_use]
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "FaceUv is clamped to [0, 1] so cell indices stay within 0..resolution"
    )]
    pub(crate) fn sample_bilinear_uv(&self, face: CubeFace, uv: FaceUv) -> MacroCell {
        let n = self.resolution as usize;
        let res_f = f64::from(self.resolution);
        let gx = uv.u.clamp(0.0, 1.0) * res_f;
        let gy = uv.v.clamp(0.0, 1.0) * res_f;
        let cx = (gx.floor() as usize).min(n - 1);
        let cy = (gy.floor() as usize).min(n - 1);
        #[expect(clippy::cast_precision_loss, reason = "cx, cy <= 512, exact in f64")]
        let tx = (gx - (cx as f64)).clamp(0.0, 1.0) as f32;
        #[expect(clippy::cast_precision_loss, reason = "cx, cy <= 512, exact in f64")]
        let ty = (gy - (cy as f64)).clamp(0.0, 1.0) as f32;

        let base = face.index() * self.stride * self.stride;
        let c00 = self.vertices[base + cy * self.stride + cx];
        let c10 = self.vertices[base + cy * self.stride + cx + 1];
        let c01 = self.vertices[base + (cy + 1) * self.stride + cx];
        let c11 = self.vertices[base + (cy + 1) * self.stride + cx + 1];

        MacroCell {
            macro_elevation_m: bilerp_f32(
                c00.macro_elevation_m,
                c10.macro_elevation_m,
                c01.macro_elevation_m,
                c11.macro_elevation_m,
                tx,
                ty,
            ),
            roughness: bilerp_f32(
                c00.roughness,
                c10.roughness,
                c01.roughness,
                c11.roughness,
                tx,
                ty,
            ),
            continentality: bilerp_f32(
                c00.continentality,
                c10.continentality,
                c01.continentality,
                c11.continentality,
                tx,
                ty,
            ),
            mean_temp_c: bilerp_f32(
                c00.mean_temp_c,
                c10.mean_temp_c,
                c01.mean_temp_c,
                c11.mean_temp_c,
                tx,
                ty,
            ),
            annual_precip_mm: bilerp_f32(
                c00.annual_precip_mm,
                c10.annual_precip_mm,
                c01.annual_precip_mm,
                c11.annual_precip_mm,
                tx,
                ty,
            ),
            geology_province: pick_nearest_province(tx, ty, c00, c10, c01, c11),
        }
    }

    /// Fast roughness-only bilinear lookup using precomputed `(face, uv)`.
    #[inline]
    #[must_use]
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "FaceUv is clamped to [0, 1] so cell indices stay within 0..resolution"
    )]
    pub(crate) fn sample_roughness_uv(&self, face: CubeFace, uv: FaceUv) -> f64 {
        let n = self.resolution as usize;
        let res_f = f64::from(self.resolution);
        let gx = uv.u.clamp(0.0, 1.0) * res_f;
        let gy = uv.v.clamp(0.0, 1.0) * res_f;
        let cx = (gx.floor() as usize).min(n - 1);
        let cy = (gy.floor() as usize).min(n - 1);
        #[expect(clippy::cast_precision_loss, reason = "cx, cy <= 512, exact in f64")]
        let tx = (gx - (cx as f64)).clamp(0.0, 1.0) as f32;
        #[expect(clippy::cast_precision_loss, reason = "cx, cy <= 512, exact in f64")]
        let ty = (gy - (cy as f64)).clamp(0.0, 1.0) as f32;

        let base = face.index() * self.stride * self.stride;
        let r00 = self.vertices[base + cy * self.stride + cx].roughness;
        let r10 = self.vertices[base + cy * self.stride + cx + 1].roughness;
        let r01 = self.vertices[base + (cy + 1) * self.stride + cx].roughness;
        let r11 = self.vertices[base + (cy + 1) * self.stride + cx + 1].roughness;
        f64::from(bilerp_f32(r00, r10, r01, r11, tx, ty))
    }
}

/// Fast 2-octave coastline-safe micro-detail offset bounded by `+-roughness * MICRO_MAX_BOUND_M`.
///
/// Returns `0.0` over oceans (`dem_elev_m <= 0.0`) and flat plains (`roughness <= 0.04`),
/// and tapers smoothly across `0..40 m` coastlines so shorelines remain crisp.
#[inline]
#[must_use]
pub(crate) fn micro_detail_fast(
    unit_dir: EcefMeters,
    dem_elev_m: f64,
    roughness: f64,
    seed: u64,
) -> f64 {
    if dem_elev_m <= 0.0 || roughness <= 0.04 {
        return 0.0;
    }
    let t = (dem_elev_m * 0.025).min(1.0);
    let coast_taper = t * t * (3.0 - 2.0 * t);
    let n0 = noise3(
        unit_dir.x * 64.0,
        unit_dir.y * 64.0,
        unit_dir.z * 64.0,
        seed.wrapping_add(701),
    );
    let n1 = noise3(
        unit_dir.x * 140.0,
        unit_dir.y * 140.0,
        unit_dir.z * 140.0,
        seed.wrapping_add(911),
    );
    let f = n0 * 0.68 + n1 * 0.32;
    roughness * coast_taper * MICRO_MAX_BOUND_M * f
}

fn bake_vertices(
    resolution: u32,
    source: &TerrainSource,
    elevation_source: &ElevationGridSource,
) -> (Vec<MacroCell>, Vec<(f64, f64)>) {
    let n = resolution as usize;
    let stride = n + 1;
    let count = 6 * stride * stride;
    let mut vertices = Vec::with_capacity(count);
    let mut vertex_bounds = Vec::with_capacity(count);
    let inv_n = 1.0 / f64::from(resolution);

    for face in CubeFace::ALL {
        for iy in 0..stride {
            #[expect(clippy::cast_precision_loss, reason = "iy <= 512, exact in f64")]
            let v = if iy == n { 1.0 } else { (iy as f64) * inv_n };
            for ix in 0..stride {
                #[expect(clippy::cast_precision_loss, reason = "ix <= 512, exact in f64")]
                let u = if ix == n { 1.0 } else { (ix as f64) * inv_n };
                let unit_dir = face_uv_to_unit(face, FaceUv::clamped(u, v));
                let (sample, qmin, qmax) = match (source, elevation_source) {
                    (TerrainSource::Earth { seed }, ElevationGridSource::Earth(dem)) => {
                        evaluate_earth_macro(dem, unit_dir, *seed)
                    }
                    (TerrainSource::Dem { seed, .. }, ElevationGridSource::CustomDem(grid)) => {
                        evaluate_dem_macro(grid, unit_dir, *seed)
                    }
                    _ => (default_fallback_sample(), 0.0, 0.0),
                };
                vertices.push(MacroCell::from(sample));
                vertex_bounds.push((qmin, qmax));
            }
        }
    }
    (vertices, vertex_bounds)
}

#[inline]
fn bilerp_f32(v00: f32, v10: f32, v01: f32, v11: f32, tx: f32, ty: f32) -> f32 {
    let top = v00 + (v10 - v00) * tx;
    let bot = v01 + (v11 - v01) * tx;
    top + (bot - top) * ty
}

#[inline]
fn pick_nearest_province(
    tx: f32,
    ty: f32,
    c00: MacroCell,
    c10: MacroCell,
    c01: MacroCell,
    c11: MacroCell,
) -> u8 {
    match (tx >= 0.5, ty >= 0.5) {
        (false, false) => c00.geology_province,
        (true, false) => c10.geology_province,
        (false, true) => c01.geology_province,
        (true, true) => c11.geology_province,
    }
}

pub(crate) const fn default_fallback_sample() -> MacroSample {
    MacroSample {
        macro_elevation_m: 0.0,
        roughness: 0.0,
        continentality: 0.0,
        mean_temp_c: 15.0,
        annual_precip_mm: 800.0,
        geology_province: 0,
    }
}
