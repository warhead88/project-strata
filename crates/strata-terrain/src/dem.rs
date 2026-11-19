use std::f64::consts::{FRAC_PI_2, PI};

use strata_globe::EcefMeters;

use crate::error::TerrainError;

/// Validated equirectangular elevation raster (North = row `0`, South = row `height - 1`,
/// longitude `-180..+180 deg` wrapping horizontally) for importing real-world DEMs (GEBCO/ETOPO).
#[derive(Debug, Clone, PartialEq)]
pub struct DemGrid {
    width: usize,
    height: usize,
    elevations_m: Vec<f32>,
}

impl DemGrid {
    /// Creates a validated equirectangular [`DemGrid`].
    ///
    /// # Errors
    ///
    /// Returns [`TerrainError::InvalidDemDimensions`] if `width < 2`, `height < 2`, or
    /// `elevations_m.len() != width * height`, or [`TerrainError::NonFiniteDemValue`] if
    /// any sample is NaN or infinite.
    pub fn new(width: usize, height: usize, elevations_m: Vec<f32>) -> Result<Self, TerrainError> {
        let Some(expected) = width.checked_mul(height) else {
            return Err(TerrainError::InvalidDemDimensions {
                width,
                height,
                len: elevations_m.len(),
            });
        };
        if width < 2 || height < 2 || elevations_m.len() != expected {
            return Err(TerrainError::InvalidDemDimensions {
                width,
                height,
                len: elevations_m.len(),
            });
        }
        if let Some(index) = elevations_m.iter().position(|v| !v.is_finite()) {
            return Err(TerrainError::NonFiniteDemValue { index });
        }
        Ok(Self {
            width,
            height,
            elevations_m,
        })
    }

    /// Raster width in texels.
    #[must_use]
    pub const fn width(&self) -> usize {
        self.width
    }

    /// Raster height in texels.
    #[must_use]
    pub const fn height(&self) -> usize {
        self.height
    }

    /// Row-major elevation values in meters (`North -> South`, `-180..+180 deg E`).
    #[must_use]
    pub fn elevations_m(&self) -> &[f32] {
        &self.elevations_m
    }

    /// Bilinearly samples elevation in meters at unit direction `unit_dir` (`O(1)`).
    #[must_use]
    #[expect(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "DEM raster indices are bounded by validated dimensions and clamped non-negative"
    )]
    pub fn sample_bilinear(&self, unit_dir: EcefMeters) -> f64 {
        let lat_rad = unit_dir.y.clamp(-1.0, 1.0).asin();
        let lon_rad = unit_dir.x.atan2(unit_dir.z);

        let u = ((lon_rad + PI) / (2.0 * PI)).rem_euclid(1.0);
        let v = ((FRAC_PI_2 - lat_rad) / PI).clamp(0.0, 1.0);

        let fx = u * (self.width as f64);
        let fy = v * ((self.height - 1) as f64);

        let x0 = (fx.floor() as usize) % self.width;
        let x1 = (x0 + 1) % self.width;
        let y0 = (fy.floor() as usize).min(self.height - 2);
        let y1 = y0 + 1;

        let tx = fx - fx.floor();
        let ty = fy - (y0 as f64);

        let e00 = f64::from(self.elevations_m[y0 * self.width + x0]);
        let e10 = f64::from(self.elevations_m[y0 * self.width + x1]);
        let e01 = f64::from(self.elevations_m[y1 * self.width + x0]);
        let e11 = f64::from(self.elevations_m[y1 * self.width + x1]);

        let top = e00 + (e10 - e00) * tx;
        let bot = e01 + (e11 - e01) * tx;
        top + (bot - top) * ty
    }
}
