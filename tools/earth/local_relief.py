"""Bake periodic landforms and a world-addressed atlas of their amplitudes."""

import numpy as np
import zstandard


def periodic_field(rng, side, cells):
    """Interpolate a periodic control lattice with zero derivative at its knots."""
    knots = rng.uniform(-1, 1, (cells, cells))
    i, t = divmod(np.arange(side) * cells / side, 1.0)
    i = i.astype(int)
    j = (i + 1) % cells
    t = t * t * (3 - 2 * t)
    rows = knots[:, i] * (1 - t) + knots[:, j] * t
    return rows[i] * (1 - t[:, None]) + rows[j] * t[:, None]


def normalized(field):
    field = field - field.mean()
    return field / np.max(np.abs(field))


def sheets():
    side = 256
    rng = np.random.default_rng(672013)
    x = np.arange(side)[None, :] / side
    y = x.T
    warp = periodic_field(rng, side, 4)
    folds = np.sin(2 * np.pi * (4 * x + 2 * y + 0.7 * warp))
    gullies = np.sin(2 * np.pi * (9 * y - 3 * x + 0.9 * periodic_field(rng, side, 5)))
    broad = periodic_field(rng, side, 7)
    # Curving ridges split into tributary gullies which deepen on their flanks.
    hills = 0.55 * folds + 0.3 * broad - 0.5 * (1 - gullies**2)**5 * (1 - folds**2)
    plains = 0.75 * broad + 0.15 * gullies + 0.1 * periodic_field(rng, side, 19)
    shear = periodic_field(rng, side, 11)
    bedding = np.sin(2 * np.pi * (7 * x + 13 * y + 1.15 * shear))
    ledges = (0.4 * np.tanh(3 * bedding) + 0.75 * periodic_field(rng, side, 14)
              + 0.22 * periodic_field(rng, side, 31))
    grit = periodic_field(rng, side, 34) + 0.8 * periodic_field(rng, side, 81)
    result = []
    for height, material in ((hills, gullies + broad), (plains, broad + 0.3 * gullies),
                             (ledges, bedding + shear), (grit, grit)):
        height = normalized(height)
        dx = (np.roll(height, -1, axis=1) - np.roll(height, 1, axis=1)) * 0.5
        dy = (np.roll(height, -1, axis=0) - np.roll(height, 1, axis=0)) * 0.5
        result.append(np.stack((height, dx, dy, normalized(material)), axis=-1))
    return np.rint(np.stack(result) * 127).astype("i1")


def smooth(value, lower, upper):
    t = np.clip((value - lower) / (upper - lower), 0, 1)
    return t * t * (3 - 2 * t)


def amplitudes(dem):
    """Fit relief to the underlying slope and protect regional valley floors."""
    rows, columns = dem.shape[0] // 4, dem.shape[1] // 4
    blocks = dem.reshape(rows, 4, columns, 4)
    low = blocks.min(axis=(1, 3)).astype(np.float32)
    high = blocks.max(axis=(1, 3)).astype(np.float32)
    grade = high - low
    lat = np.pi * (0.5 - (np.arange(rows, dtype=np.float32) + 0.5) / rows)
    polar_fade = smooth(np.cos(lat), 0.03, 0.13)[:, None]
    # The dry-side ETOPO valley floor receives less displacement than its shoulders.
    center = (low + high) * 0.5
    neighbor = (np.roll(center, -1, axis=1) + np.roll(center, 1, axis=1)
                + np.roll(center, -1, axis=0) + np.roll(center, 1, axis=0)) * 0.25
    valley = 1 - 0.7 * smooth(neighbor - center, 45, 270)
    mountain = smooth(grade, 70, 1050)
    active = polar_fade * valley
    coarse = active * mountain
    rolling = active * (1 - mountain) * (0.42 + 0.58 * smooth(grade, 0, 150))
    controls = np.stack((coarse, rolling), axis=-1)
    controls = controls.reshape(rows // 2, 2, columns // 2, 2, 2).mean(axis=(1, 3))
    hill = np.rint(controls[..., 0] * 63).astype("u1")
    plain = np.minimum(np.rint(controls[..., 1] * 63), 63 - hill).astype("u1")
    return np.stack((hill, plain), axis=-1) * 4


def bake_local_relief(output, dem):
    packed = zstandard.ZstdCompressor(level=10)
    (output / "local-shapes.zst").write_bytes(packed.compress(sheets().tobytes()))
    (output / "local-amplitudes.zst").write_bytes(packed.compress(amplitudes(dem).tobytes()))
