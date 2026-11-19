use std::f64::consts::{FRAC_PI_4, PI};

use crate::coords::EcefMeters;
use crate::error::GlobeError;

/// Six faces of the cube-sphere parameterization.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum CubeFace {
    /// `+X` face.
    PosX,
    /// `-X` face.
    NegX,
    /// `+Y` face (North Pole).
    PosY,
    /// `-Y` face (South Pole).
    NegY,
    /// `+Z` face (Prime Meridian).
    PosZ,
    /// `-Z` face (Antimeridian).
    NegZ,
}

/// Right-handed orthonormal basis `(right, up, normal)` for a [`CubeFace`].
///
/// Satisfies `right x up == normal` so `(u, v)` increasing yields CCW outward triangles.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FaceBasis {
    /// Outward face normal unit vector.
    pub normal: EcefMeters,
    /// Local `+u` tangent unit vector.
    pub right: EcefMeters,
    /// Local `+v` tangent unit vector.
    pub up: EcefMeters,
}

impl CubeFace {
    /// All six cube faces in canonical order.
    pub const ALL: [Self; 6] = [
        Self::PosX,
        Self::NegX,
        Self::PosY,
        Self::NegY,
        Self::PosZ,
        Self::NegZ,
    ];

    /// Numeric index `0..6` for array indexing.
    #[must_use]
    pub const fn index(self) -> usize {
        match self {
            Self::PosX => 0,
            Self::NegX => 1,
            Self::PosY => 2,
            Self::NegY => 3,
            Self::PosZ => 4,
            Self::NegZ => 5,
        }
    }

    /// Short human-readable label (e.g. `"+X"`).
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::PosX => "+X",
            Self::NegX => "-X",
            Self::PosY => "+Y",
            Self::NegY => "-Y",
            Self::PosZ => "+Z",
            Self::NegZ => "-Z",
        }
    }

    /// Returns the right-handed basis `(right, up, normal)` of this face.
    #[must_use]
    pub const fn basis(self) -> FaceBasis {
        match self {
            Self::PosX => FaceBasis {
                normal: EcefMeters::from_raw(1.0, 0.0, 0.0),
                right: EcefMeters::from_raw(0.0, 0.0, -1.0),
                up: EcefMeters::from_raw(0.0, 1.0, 0.0),
            },
            Self::NegX => FaceBasis {
                normal: EcefMeters::from_raw(-1.0, 0.0, 0.0),
                right: EcefMeters::from_raw(0.0, 0.0, 1.0),
                up: EcefMeters::from_raw(0.0, 1.0, 0.0),
            },
            Self::PosY => FaceBasis {
                normal: EcefMeters::from_raw(0.0, 1.0, 0.0),
                right: EcefMeters::from_raw(1.0, 0.0, 0.0),
                up: EcefMeters::from_raw(0.0, 0.0, -1.0),
            },
            Self::NegY => FaceBasis {
                normal: EcefMeters::from_raw(0.0, -1.0, 0.0),
                right: EcefMeters::from_raw(1.0, 0.0, 0.0),
                up: EcefMeters::from_raw(0.0, 0.0, 1.0),
            },
            Self::PosZ => FaceBasis {
                normal: EcefMeters::from_raw(0.0, 0.0, 1.0),
                right: EcefMeters::from_raw(1.0, 0.0, 0.0),
                up: EcefMeters::from_raw(0.0, 1.0, 0.0),
            },
            Self::NegZ => FaceBasis {
                normal: EcefMeters::from_raw(0.0, 0.0, -1.0),
                right: EcefMeters::from_raw(-1.0, 0.0, 0.0),
                up: EcefMeters::from_raw(0.0, 1.0, 0.0),
            },
        }
    }

    /// Identifies the [`CubeFace`] whose outward normal equals the dominant axis of `v`.
    #[must_use]
    pub fn from_dominant_axis(v: EcefMeters) -> Self {
        let ax = v.x.abs();
        let ay = v.y.abs();
        let az = v.z.abs();
        if ax >= ay && ax >= az {
            if v.x >= 0.0 { Self::PosX } else { Self::NegX }
        } else if ay >= ax && ay >= az {
            if v.y >= 0.0 { Self::PosY } else { Self::NegY }
        } else if v.z >= 0.0 {
            Self::PosZ
        } else {
            Self::NegZ
        }
    }
}

/// Normalized face-space coordinates `(u, v)` in `[0.0, 1.0]^2`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FaceUv {
    /// Horizontal face coordinate in `[0.0, 1.0]`.
    pub u: f64,
    /// Vertical face coordinate in `[0.0, 1.0]`.
    pub v: f64,
}

impl FaceUv {
    /// Creates a [`FaceUv`] clamped to `[0.0, 1.0]^2`.
    #[must_use]
    pub fn clamped(u: f64, v: f64) -> Self {
        Self {
            u: u.clamp(0.0, 1.0),
            v: v.clamp(0.0, 1.0),
        }
    }
}

/// Maps `(CubeFace, FaceUv)` to a unit direction vector using the invertible
/// equal-angle tangent warp (`tan(s * pi / 4)`).
#[must_use]
pub fn face_uv_to_unit(face: CubeFace, uv: FaceUv) -> EcefMeters {
    let s = 2.0 * uv.u.clamp(0.0, 1.0) - 1.0;
    let t = 2.0 * uv.v.clamp(0.0, 1.0) - 1.0;
    let xf = equal_angle_tan(s);
    let yf = equal_angle_tan(t);
    let basis = face.basis();
    let cube_pt = basis.normal + basis.right.scale(xf) + basis.up.scale(yf);
    let len = cube_pt.norm();
    cube_pt.scale(1.0 / len)
}

/// Maps a unit/non-zero ECEF direction to `(CubeFace, FaceUv)`.
///
/// # Errors
///
/// Returns [`GlobeError`] if `vec` is non-finite or zero-magnitude.
pub fn ecef_to_face_uv(vec: EcefMeters) -> Result<(CubeFace, FaceUv), GlobeError> {
    let unit = vec.normalized()?;
    let face = CubeFace::from_dominant_axis(unit);
    let basis = face.basis();
    let dn = unit.dot(basis.normal);
    let xf = unit.dot(basis.right) / dn;
    let yf = unit.dot(basis.up) / dn;
    let s = (xf.atan() * (4.0 / PI)).clamp(-1.0, 1.0);
    let t = (yf.atan() * (4.0 / PI)).clamp(-1.0, 1.0);
    Ok((face, FaceUv::clamped(0.5 * (s + 1.0), 0.5 * (t + 1.0))))
}

/// Equal-angle tangent warp preserving exact `{-1.0, 0.0, +1.0}` endpoints
/// so cube-face seams match bit-for-bit before normalization.
#[inline]
#[expect(
    clippy::float_cmp,
    reason = "exact boundary values -1.0, 0.0, 1.0 must map cleanly to exact cube edges"
)]
fn equal_angle_tan(s: f64) -> f64 {
    if s == -1.0 {
        -1.0
    } else if s == 1.0 {
        1.0
    } else if s == 0.0 {
        0.0
    } else {
        (s * FRAC_PI_4).tan()
    }
}
