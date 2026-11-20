use std::f64::consts::{FRAC_PI_2, PI, TAU};
use std::sync::{Arc, OnceLock};

use strata_globe::EcefMeters;

use crate::earth_pack::{invalid, read_rows};
use crate::error::TerrainError;

static RELIEF: [&[u8]; 4] = [
    include_bytes!("../data/earth/relief-0.zst"),
    include_bytes!("../data/earth/relief-1.zst"),
    include_bytes!("../data/earth/relief-2.zst"),
    include_bytes!("../data/earth/relief-3.zst"),
];
static GLOBAL_EARTH_DEM: OnceLock<Result<Arc<EarthDem>, TerrainError>> = OnceLock::new();

/// Pixel-centered global ETOPO 2022 relief at 60 arc seconds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct EarthDem {
    width: usize,
    height: usize,
    elevations_m: Vec<i16>,
}

impl EarthDem {
    pub(crate) fn global_embedded() -> Result<Arc<Self>, TerrainError> {
        GLOBAL_EARTH_DEM
            .get_or_init(|| Self::decode().map(Arc::new))
            .clone()
    }

    fn decode() -> Result<Self, TerrainError> {
        let (width, height) = (21_600, 10_800);
        let mut elevations_m = Vec::with_capacity(width * height);
        for bytes in RELIEF {
            read_rows(bytes, 2 * width, height / RELIEF.len(), |row| {
                let mut previous = 0_i16;
                elevations_m.extend(row.as_chunks::<2>().0.iter().map(|&delta| {
                    previous = previous.wrapping_add(i16::from_le_bytes(delta));
                    previous
                }));
            })?;
        }
        if elevations_m.len() != width * height {
            return Err(invalid("unexpected elevation sample count"));
        }
        Ok(Self {
            width,
            height,
            elevations_m,
        })
    }

    #[must_use]
    pub(crate) const fn width(&self) -> usize {
        self.width
    }

    #[must_use]
    pub(crate) const fn height(&self) -> usize {
        self.height
    }

    #[must_use]
    pub(crate) fn elevations_m(&self) -> &[i16] {
        &self.elevations_m
    }

    #[must_use]
    #[inline]
    pub(crate) fn sample_bilinear(&self, unit_dir: EcefMeters) -> f64 {
        let lat_rad = unit_dir.y.clamp(-1.0, 1.0).asin();
        let lon_rad = unit_dir.x.atan2(unit_dir.z);
        self.sample_bilinear_lat_lon(lat_rad, lon_rad)
    }

    #[must_use]
    #[inline]
    pub(crate) fn sample_bilinear_lat_lon(&self, lat_rad: f64, lon_rad: f64) -> f64 {
        let (x0, x1, y0, y1, tx, ty) = self.texel_quad(lat_rad, lon_rad);
        let r0 = y0 * self.width;
        let r1 = y1 * self.width;
        let e00 = f64::from(self.elevations_m[r0 + x0]);
        let e10 = f64::from(self.elevations_m[r0 + x1]);
        let e01 = f64::from(self.elevations_m[r1 + x0]);
        let e11 = f64::from(self.elevations_m[r1 + x1]);
        let top = e00 + (e10 - e00) * tx;
        let bot = e01 + (e11 - e01) * tx;
        top + (bot - top) * ty
    }

    pub(crate) fn shading_normal(&self, unit: EcefMeters, radius_m: f64) -> EcefMeters {
        let horizontal = unit.x.hypot(unit.z);
        if horizontal < 1e-6 {
            return unit;
        }
        let step = PI / 10_800.0;
        let lat = unit.y.atan2(horizontal);
        let lon = unit.x.atan2(unit.z);
        let east = EcefMeters::from_raw(unit.z / horizontal, 0.0, -unit.x / horizontal);
        let north = unit.cross(east);
        let slope_east = (self.sample_bilinear_lat_lon(lat, lon + step)
            - self.sample_bilinear_lat_lon(lat, lon - step))
            / (2.0 * step * radius_m * horizontal.max(1e-4));
        let slope_north = (self.sample_bilinear_lat_lon(lat + step, lon)
            - self.sample_bilinear_lat_lon(lat - step, lon))
            / (2.0 * step * radius_m);
        (unit - east.scale(slope_east) - north.scale(slope_north))
            .normalized()
            .unwrap_or(unit)
    }

    /// Evaluates `(bilinear_elev_m, local_relief_m, quad_min_m, quad_max_m, land_fraction)`
    /// in a single texel quad lookup using integer offsets for land fraction.
    #[must_use]
    #[inline]
    pub(crate) fn sample_earth_vertex_features(
        &self,
        lat_rad: f64,
        lon_rad: f64,
    ) -> (f64, f64, f64, f64, f64) {
        let (x0, x1, y0, y1, tx, ty) = self.texel_quad(lat_rad, lon_rad);
        let r0 = y0 * self.width;
        let r1 = y1 * self.width;
        let i00 = self.elevations_m[r0 + x0];
        let i10 = self.elevations_m[r0 + x1];
        let i01 = self.elevations_m[r1 + x0];
        let i11 = self.elevations_m[r1 + x1];
        let qmin = i00.min(i10).min(i01).min(i11);
        let qmax = i00.max(i10).max(i01).max(i11);
        let relief = f64::from(i32::from(qmax) - i32::from(qmin));

        let e00 = f64::from(i00);
        let e10 = f64::from(i10);
        let e01 = f64::from(i01);
        let e11 = f64::from(i11);
        let top = e00 + (e10 - e00) * tx;
        let bot = e01 + (e11 - e01) * tx;
        let elev = top + (bot - top) * ty;
        let land_frac = self.land_fraction_at_texel(x0, y0);
        (elev, relief, f64::from(qmin), f64::from(qmax), land_frac)
    }

    /// Minimum and maximum of the 4 surrounding DEM texels for conservative tile bounds.
    #[must_use]
    #[inline]
    pub(crate) fn sample_quad_minmax(&self, unit_dir: EcefMeters) -> (f64, f64) {
        let lat_rad = unit_dir.y.clamp(-1.0, 1.0).asin();
        let lon_rad = unit_dir.x.atan2(unit_dir.z);
        let (x0, x1, y0, y1, _, _) = self.texel_quad(lat_rad, lon_rad);
        let r0 = y0 * self.width;
        let r1 = y1 * self.width;
        let e00 = self.elevations_m[r0 + x0];
        let e10 = self.elevations_m[r0 + x1];
        let e01 = self.elevations_m[r1 + x0];
        let e11 = self.elevations_m[r1 + x1];
        let min_e = e00.min(e10).min(e01).min(e11);
        let max_e = e00.max(e10).max(e01).max(e11);
        (f64::from(min_e), f64::from(max_e))
    }

    #[inline]
    fn land_fraction_at_texel(&self, col: usize, row: usize) -> f64 {
        let w_i = self.width.cast_signed();
        let h_i = self.height.cast_signed();
        let c_i = col.cast_signed();
        let r_i = row.cast_signed();
        // Offsets scaled cleanly to the grid (`1440x720` => 0.25 deg/texel).
        let d_lat_sm = (h_i * 7) / 360;
        let d_lon_sm = (w_i * 4) / 360;
        let d_lat_lg = (h_i * 7) / 180;
        let d_lon_lg = (w_i * 7) / 360;
        let offsets = [
            (0_isize, 0_isize),
            (d_lat_sm, 0),
            (-d_lat_sm, 0),
            (0, d_lon_sm),
            (0, -d_lon_sm),
            (d_lat_lg, d_lon_lg),
            (d_lat_lg, -d_lon_lg),
            (-d_lat_lg, d_lon_lg),
            (-d_lat_lg, -d_lon_lg),
        ];

        let mut land = 0_u32;
        for (dr, dc) in offsets {
            let rr = (r_i + dr).clamp(0, h_i - 1).cast_unsigned();
            let cc = (c_i + dc).rem_euclid(w_i).cast_unsigned();
            if self.elevations_m[rr * self.width + cc] > 0 {
                land += 1;
            }
        }
        f64::from(land) / 9.0
    }

    #[inline]
    #[expect(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "pixel-centered coordinates are wrapped/clamped into valid grid indices"
    )]
    fn texel_quad(&self, lat_rad: f64, lon_rad: f64) -> (usize, usize, usize, usize, f64, f64) {
        let u = wrap_unit((lon_rad + PI) * INV_TAU);
        let v = ((FRAC_PI_2 - lat_rad) * INV_PI).clamp(0.0, 1.0);

        let w_f = self.width as f64;
        let h_f = self.height as f64;
        let mut fx = u * w_f - 0.5;
        if fx < 0.0 {
            fx += w_f;
        } else if fx >= w_f {
            fx -= w_f;
        }
        let fy = (v * h_f - 0.5).clamp(0.0, h_f - 1.0);

        let x0_floor = fx.floor();
        let x0 = (x0_floor as usize) % self.width;
        let x1 = if x0 + 1 == self.width { 0 } else { x0 + 1 };
        let y0 = (fy as usize).min(self.height - 2);
        let y1 = y0 + 1;

        let tx = fx - x0_floor;
        let ty = fy - (y0 as f64);
        (x0, x1, y0, y1, tx, ty)
    }
}

const INV_PI: f64 = 1.0 / PI;
const INV_TAU: f64 = 1.0 / TAU;

#[inline]
fn wrap_unit(mut u: f64) -> f64 {
    if !(-1.0..2.0).contains(&u) {
        return u.rem_euclid(1.0);
    }
    if u < 0.0 {
        u += 1.0;
    } else if u >= 1.0 {
        u -= 1.0;
    }
    u
}
