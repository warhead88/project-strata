"""Build the immutable Earth surface shipped with strata-terrain."""
import argparse
import hashlib
from pathlib import Path
import urllib.request

import numpy as np
import rasterio
import zstandard

from water import bake_water

DEM = "ETOPO_2022_v1_60s_N90W180_surface.tif"
LAYERS = ("land", "minor_islands", "lakes", "rivers_lake_centerlines", "geography_marine_polys")
SOURCES = {
    DEM: "https://www.ngdc.noaa.gov/mgg/global/relief/ETOPO2022/data/60s/60s_surface_elev_gtif/" + DEM,
    **{
        f"ne_10m_{layer}.zip": f"https://naturalearth.s3.amazonaws.com/10m_physical/ne_10m_{layer}.zip"
        for layer in LAYERS
    },
}


def inputs(directory):
    directory.mkdir(parents=True, exist_ok=True)
    checksums = dict(
        line.split()[::-1]
        for line in Path(__file__).with_name("sources.sha256").read_text().splitlines()
    )
    for name, url in SOURCES.items():
        path = directory / name
        if not path.exists():
            working = path.with_suffix(".partial")
            urllib.request.urlretrieve(url, working)
            working.rename(path)
        with path.open("rb") as file:
            digest = hashlib.file_digest(file, "sha256").hexdigest()
        if digest != checksums[name]:
            raise ValueError(f"Unexpected SHA-256 for {path}: {digest}")
    return directory / DEM


def write_grid(output, dem):
    # Modulo-16-bit horizontal predictor. Each row and each file can decode independently.
    for index, rows in enumerate(np.array_split(dem, 4)):
        previous = np.diff(rows, axis=1, prepend=np.zeros((len(rows), 1), dtype="<i2"))
        with (output / f"relief-{index}.zst").open("wb") as stream:
            zstandard.ZstdCompressor(level=10).copy_stream(
                memory_file(previous), stream, size=previous.nbytes
            )


    # Leaf inputs cover 8x8 DEM cells plus interpolation guards, aligned to water pages.
    guarded = np.pad(dem, ((0, 0), (1, 1)), mode="wrap")
    guarded = np.pad(guarded, ((1, 1), (0, 0)), mode="edge")
    with (output / "relief-bounds.zst").open("wb") as handle:
        with zstandard.ZstdCompressor(level=9).stream_writer(handle) as stream:
            for row in range(0, dem.shape[0], 8):
                scan = guarded[row:row + 10]
                min_col, max_col = scan.min(axis=0), scan.max(axis=0)
                lo = np.minimum(min_col[1:-1].reshape(-1, 8).min(axis=1),
                                np.minimum(min_col[:-2:8], min_col[9::8]))
                hi = np.maximum(max_col[1:-1].reshape(-1, 8).max(axis=1),
                                np.maximum(max_col[:-2:8], max_col[9::8]))
                stream.write(np.stack((lo, hi), axis=1).astype("<i2").tobytes())


def memory_file(array):
    import io
    return io.BytesIO(np.ascontiguousarray(array, dtype="<i2").data)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("cache", type=Path)
    parser.add_argument("output", type=Path)
    args = parser.parse_args()
    source = inputs(args.cache)
    args.output.mkdir(parents=True, exist_ok=True)
    with rasterio.open(source) as raster:
        if raster.shape != (10800, 21600):
            raise ValueError(f"Unexpected elevation raster: {raster.shape}")
        dem = np.empty(raster.shape, dtype="<i2")
        for row in range(0, raster.height, 128):
            window = rasterio.windows.Window(0, row, raster.width, min(128, raster.height - row))
            samples = raster.read(1, window=window)
            if not np.all(np.isfinite(samples) & (samples >= -11_500) & (samples <= 9_000)):
                raise ValueError(f"Invalid elevation at row {row}")
            dem[row:row + len(samples)] = np.rint(samples)
        # Longitude is undefined at the poles. Make the clamped end rows invariant.
        for row in (0, -1):
            dem[row] = round(float(np.mean(dem[row])))
    print("Packing elevation", flush=True)
    write_grid(args.output, dem)
    print("Baking water", flush=True)
    bake_water(args.cache, args.output, dem)


if __name__ == "__main__":
    main()
