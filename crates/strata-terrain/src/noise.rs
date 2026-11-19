use strata_globe::EcefMeters;

/// Sixteen 3D gradient directions (12 cube-edge vectors + 4 symmetric repeats)
/// enabling branchless `hash & 15` selection with zero heap allocations.
const GRADIENTS: [[f64; 3]; 16] = [
    [1.0, 1.0, 0.0],
    [-1.0, 1.0, 0.0],
    [1.0, -1.0, 0.0],
    [-1.0, -1.0, 0.0],
    [1.0, 0.0, 1.0],
    [-1.0, 0.0, 1.0],
    [1.0, 0.0, -1.0],
    [-1.0, 0.0, -1.0],
    [0.0, 1.0, 1.0],
    [0.0, -1.0, 1.0],
    [0.0, 1.0, -1.0],
    [0.0, -1.0, -1.0],
    [1.0, 1.0, 0.0],
    [-1.0, 1.0, 0.0],
    [0.0, -1.0, 1.0],
    [0.0, -1.0, -1.0],
];

/// Deterministic `SplitMix64` integer hash for 3D lattice coordinates.
#[inline]
const fn hash3(ix: i32, iy: i32, iz: i32, seed: u64) -> u64 {
    let mut h = seed
        .wrapping_add(
            (ix as i64)
                .cast_unsigned()
                .wrapping_mul(0x9E37_79B9_7F4A_7C15),
        )
        .wrapping_add(
            (iy as i64)
                .cast_unsigned()
                .wrapping_mul(0xBF58_476D_1CE4_E5B9),
        )
        .wrapping_add(
            (iz as i64)
                .cast_unsigned()
                .wrapping_mul(0x94D0_49BB_1331_11EB),
        );
    h ^= h >> 30;
    h = h.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    h ^= h >> 27;
    h = h.wrapping_mul(0x94D0_49BB_1331_11EB);
    h ^ (h >> 31)
}

#[inline]
fn quintic_fade(t: f64) -> f64 {
    t * t * t * (t * (t * 6.0 - 15.0) + 10.0)
}

#[inline]
fn lerp(a: f64, b: f64, t: f64) -> f64 {
    a + (b - a) * t
}

#[inline]
fn grad_dot(ix: i32, iy: i32, iz: i32, dx: f64, dy: f64, dz: f64, seed: u64) -> f64 {
    let idx = (hash3(ix, iy, iz, seed) & 15) as usize;
    let g = GRADIENTS[idx];
    g[0] * dx + g[1] * dy + g[2] * dz
}

/// Evaluates deterministic 3D gradient noise in `[-1.0, 1.0]` with zero heap allocations.
#[must_use]
#[expect(
    clippy::cast_possible_truncation,
    reason = "lattice floor coordinates for bounded spherical queries stay well within i32"
)]
pub fn noise3(px: f64, py: f64, pz: f64, seed: u64) -> f64 {
    let fx = px.floor();
    let fy = py.floor();
    let fz = pz.floor();

    let ix = fx as i32;
    let iy = fy as i32;
    let iz = fz as i32;

    let dx = px - fx;
    let dy = py - fy;
    let dz = pz - fz;

    let fade_x = quintic_fade(dx);
    let fade_y = quintic_fade(dy);
    let fade_z = quintic_fade(dz);

    let c000 = grad_dot(ix, iy, iz, dx, dy, dz, seed);
    let c100 = grad_dot(ix.wrapping_add(1), iy, iz, dx - 1.0, dy, dz, seed);
    let c010 = grad_dot(ix, iy.wrapping_add(1), iz, dx, dy - 1.0, dz, seed);
    let c110 = grad_dot(
        ix.wrapping_add(1),
        iy.wrapping_add(1),
        iz,
        dx - 1.0,
        dy - 1.0,
        dz,
        seed,
    );
    let c001 = grad_dot(ix, iy, iz.wrapping_add(1), dx, dy, dz - 1.0, seed);
    let c101 = grad_dot(
        ix.wrapping_add(1),
        iy,
        iz.wrapping_add(1),
        dx - 1.0,
        dy,
        dz - 1.0,
        seed,
    );
    let c011 = grad_dot(
        ix,
        iy.wrapping_add(1),
        iz.wrapping_add(1),
        dx,
        dy - 1.0,
        dz - 1.0,
        seed,
    );
    let c111 = grad_dot(
        ix.wrapping_add(1),
        iy.wrapping_add(1),
        iz.wrapping_add(1),
        dx - 1.0,
        dy - 1.0,
        dz - 1.0,
        seed,
    );

    let x00 = lerp(c000, c100, fade_x);
    let x10 = lerp(c010, c110, fade_x);
    let x01 = lerp(c001, c101, fade_x);
    let x11 = lerp(c011, c111, fade_x);

    let y0 = lerp(x00, x10, fade_y);
    let y1 = lerp(x01, x11, fade_y);

    // Edge-vector 3D gradient noise peaks near ~0.96; scale slightly and clamp strictly.
    (lerp(y0, y1, fade_z) * 1.05).clamp(-1.0, 1.0)
}

/// Evaluates normalized 3D fractional Brownian motion on a unit direction vector in `[-1.0, 1.0]`.
///
/// Operating directly on 3D `EcefMeters` guarantees bit-identical results across
/// cube-face seams, edges, and poles.
#[must_use]
pub fn fbm3(
    dir: EcefMeters,
    base_freq: f64,
    octaves: u8,
    lacunarity: f64,
    gain: f64,
    seed: u64,
) -> f64 {
    if octaves == 0 {
        return 0.0;
    }
    let mut freq = base_freq;
    let mut amp = 1.0;
    let mut sum = 0.0;
    let mut weight_sum = 0.0;

    for octave in 0..octaves {
        let oct_seed = seed.wrapping_add(u64::from(octave).wrapping_mul(0x9E37_79B9_7F4A_7C15));
        let n = noise3(dir.x * freq, dir.y * freq, dir.z * freq, oct_seed);
        sum += n * amp;
        weight_sum += amp;
        freq *= lacunarity;
        amp *= gain;
    }

    if weight_sum > 0.0 {
        (sum / weight_sum).clamp(-1.0, 1.0)
    } else {
        0.0
    }
}

/// Evaluates normalized 3D ridged multifractal noise on a unit direction vector in `[0.0, 1.0]`.
///
/// Produces sharp tectonic ridges and eroded crests without cube-face seams.
#[must_use]
pub fn ridged_fbm3(
    dir: EcefMeters,
    base_freq: f64,
    octaves: u8,
    lacunarity: f64,
    gain: f64,
    seed: u64,
) -> f64 {
    if octaves == 0 {
        return 0.0;
    }
    let mut freq = base_freq;
    let mut amp = 1.0;
    let mut sum = 0.0;
    let mut weight_sum = 0.0;
    let mut prev_ridge = 1.0;

    for octave in 0..octaves {
        let oct_seed = seed.wrapping_add(u64::from(octave).wrapping_mul(0xBF58_476D_1CE4_E5B9));
        let n = noise3(dir.x * freq, dir.y * freq, dir.z * freq, oct_seed);
        let mut ridge = 1.0 - n.abs();
        ridge *= ridge;
        let w = amp * prev_ridge;
        sum += ridge * w;
        weight_sum += w;
        prev_ridge = (ridge * 1.8).clamp(0.15, 1.0);
        freq *= lacunarity;
        amp *= gain;
    }

    if weight_sum > 0.0 {
        (sum / weight_sum).clamp(0.0, 1.0)
    } else {
        0.0
    }
}
