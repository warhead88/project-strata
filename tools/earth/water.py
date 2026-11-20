"""Rasterize shoreline polygons and connected channel centerlines before launch."""
import math
from pathlib import Path
import zipfile

import numpy as np
from rasterio.features import rasterize
from rasterio.transform import from_origin
from scipy.ndimage import distance_transform_edt, map_coordinates
import shapefile
import shapely
from shapely.affinity import translate
from shapely.geometry import LineString, Polygon, box, shape
from shapely.strtree import STRtree
import zstandard

PIXELS_PER_DEGREE = 240
PAGE = 16
STRIDE = PAGE + 1  # East/south guard texels for bilinear sampling.
WIDTH = 360 * PIXELS_PER_DEGREE
HEIGHT = 180 * PIXELS_PER_DEGREE
HALO = 48
BATCH = PAGE * 60  # Four degrees, independent of latitude.
PAGE_FLAG = 1 << 31


def read_layer(cache, layer):
    name = f"ne_10m_{layer}"
    with zipfile.ZipFile(cache / f"{name}.zip") as archive:
        reader = shapefile.Reader(shp=archive.open(f"{name}.shp"), dbf=archive.open(f"{name}.dbf"))
        for item in reader.iterShapeRecords():
            yield shapely.make_valid(shape(item.shape.__geo_interface__)), item.record.as_dict()


def geometries(geometry, kind):
    if geometry.geom_type == kind:
        yield geometry
    elif hasattr(geometry, "geoms"):
        for part in geometry.geoms:
            yield from geometries(part, kind)


def elevations(dem, lon, lat):
    y = np.clip((90 - lat) * 60 - 0.5, 0, dem.shape[0] - 1)
    x = (180 + lon) * 60 - 0.5
    # y is clamped independently, x uses the periodic longitude grid.
    return map_coordinates(dem, [y, x], output=np.float32, order=1, mode="grid-wrap")


def water_code(slot, elevation):
    return slot | ((int(elevation) + 16384) << 16)


class Layer:
    def __init__(self, shapes):
        self.shapes = shapes
        self.tree = STRtree([geometry for geometry, _ in shapes])

    def visible(self, west, south, east, north):
        result = []
        for shift in (-360, 0, 360):
            if east + shift < -180 or west + shift > 180:
                continue
            extent = box(west + shift, south, east + shift, north)
            for i in sorted(self.tree.query(extent, predicate="intersects")):
                geometry, code = self.shapes[i]
                # Clip before GDAL scan conversion, especially the continental outlines.
                clipped = shapely.intersection(geometry, extent)
                if not clipped.is_empty:
                    result.append((translate(clipped, xoff=-shift) if shift else clipped, code))
        return result


def source_layers(cache, dem, output):
    land = [
        (part, 0)
        for layer in ("land", "minor_islands")
        for geometry, _ in read_layer(cache, layer)
        for part in geometries(geometry, "Polygon")
    ]
    lakes = []
    metadata = ["0\t0\t0\t0\t0\t0\t0\t0\tDry", "1\t0\t1\t0\t0\t0\t0\t0\tOcean"]
    natural_lakes = list(read_layer(cache, "lakes"))
    lake_index = STRtree([geometry for geometry, _ in natural_lakes])
    marine = list(read_layer(cache, "geography_marine_polys"))
    marine_index = STRtree([geometry for geometry, _ in marine])
    for geometry, _ in land:
        for ring in geometry.interiors:
            hole = Polygon(ring)
            if hole.area < 0.01 or len(lake_index.query(hole.representative_point(), predicate="within")):
                continue
            # Natural Earth encodes inland seas as holes in its land polygons.
            center = hole.representative_point()
            labels = marine_index.query(center, predicate="within")
            stable_id = 2_000_000_000 + int((center.x + 180) * 10) * 1800 + int((center.y + 90) * 10)
            record = marine[min(labels, key=lambda i: marine[i][0].area)][1] if len(labels) else {
                "ne_id": stable_id, "name": ""
            }
            natural_lakes.append((hole, {**record, "featurecla": "Inland Sea"}))

    for shape_, record in natural_lakes:
        if shape_.is_empty:
            continue
        center = shape_.representative_point()
        west, south, east, north = shape_.bounds
        xx, yy = np.meshgrid(np.linspace(west, east, 31), np.linspace(south, north, 31))
        inside = shapely.contains_xy(shape_, xx, yy)
        samples = elevations(dem, xx[inside], yy[inside]) if inside.any() else elevations(
            dem, np.array([center.x]), np.array([center.y])
        )
        coast_x, coast_y = shapely.get_coordinates(shapely.buffer(shape_, 0.006).boundary).T
        shore = elevations(dem, coast_x, coast_y)
        # ETOPO includes bed elevations for some lakes. The lower quartile just
        # outside their boundary estimates the surface before banks climb.
        level = round(float(np.percentile(shore, 25)))
        bathymetry = min(0, float(np.percentile(samples, 0.5)) - level)
        kind = 3 if record["featurecla"] in ("Alkaline Lake", "Inland Sea") else 2
        slot = len(metadata)
        lakes.extend((part, water_code(slot, level)) for part in geometries(shape_, "Polygon"))
        radius = math.sqrt(shape_.area / math.pi)
        name = record["name"] or ("Inland sea" if kind == 3 else "Lake")
        metadata.append(f"{slot}\t{record['ne_id']}\t{kind}\t{level}\t{center.x}\t{center.y}\t{radius}\t{max(10, -bathymetry):.1f}\t{name}")

    rivers = []
    profiles = []
    ranks = {}
    for shape_, record in read_layer(cache, "rivers_lake_centerlines"):
        if record["featurecla"] == "Lake Centerline" and "Canal" not in record["name"]:
            continue
        for line in geometries(shape_, "LineString"):
            if line.length == 0:
                continue
            # Densify so the longitudinal profile tracks the regional valley.
            points = shapely.get_coordinates(shapely.segmentize(line, 1 / 60))
            heights = elevations(dem, points[:, 0], points[:, 1])
            if np.median(heights[:5]) < np.median(heights[-5:]):
                heights = heights[::-1]
                points = points[::-1]
            # Prevent short spikes where source linework crosses a DEM ridge.
            levels = np.rint(np.minimum.accumulate(heights))
            slot = len(metadata)
            name = record["name"] or "River"
            ranks[record["ne_id"]] = record["scalerank"]
            metadata.append(f"{slot}\t{record['ne_id']}\t4\t0\t0\t0\t0\t0\t{name}")
            for a, b, za, zb in zip(points[:-1], points[1:], levels[:-1], levels[1:]):
                rivers.append((LineString([a, b]), water_code(slot, round(float((za + zb) / 2)))))
            profiles.append(np.column_stack((np.full(len(points), slot), points * 1_000_000, levels)))

    metadata = add_navigability(metadata, ranks)
    (output / "water.tsv").write_text("\n".join(metadata) + "\n", encoding="utf-8")
    with (output / "rivers.zst").open("wb") as file:
        file.write(zstandard.ZstdCompressor(level=10).compress(np.rint(np.concatenate(profiles)).astype("<i4").tobytes()))
    return Layer(land), Layer(lakes), Layer(rivers)



def add_navigability(metadata, ranks):
    """Approximate gameplay classes from the hierarchy of mapped waterways."""
    rows = []
    for row in metadata:
        slot, id_, kind, level, x, y, radius, depth, name = row.split("\t", 8)
        rank = ranks.get(int(id_), 99)
        navigation = 2 if kind == "4" and rank <= 3 else 0
        if kind == "4" and (4 <= rank <= 6 or "Canal" in name):
            navigation = 1
        rows.append(f"{slot}\t{id_}\t{kind}\t{level}\t{x}\t{y}\t{radius}\t{depth}\t{navigation}\t{name}")
    return rows

def codes_at(layers, x, y, w, h, scale):
    west = x / scale - 180
    north = 90 - y / scale
    extent = (west, north - h / scale, west + w / scale, north)
    transform = from_origin(west, north, 1 / scale, 1 / scale)
    land, lakes, rivers = (layer.visible(*extent) for layer in layers)
    codes = np.full((h, w), 1, dtype=np.uint32)
    for features in (land, lakes):
        if features:
            rasterize(features, out=codes, transform=transform)
    if rivers:
        channels = rasterize(rivers, out_shape=(h, w), transform=transform, all_touched=True, dtype="uint32")
        replace = (codes == 0) & (channels != 0)
        codes[replace] = channels[replace]
    return codes


def bake_relief(dem, codes, metadata, x, y, w, h):
    lon = (np.arange(x, x + w) + 0.5) / PIXELS_PER_DEGREE - 180
    lat = np.clip(90 - (np.arange(y, y + h) + 0.5) / PIXELS_PER_DEGREE, -90, 90)
    xx, yy = np.meshgrid(lon, lat)
    base = elevations(dem, xx, yy)
    slots = (codes & 65535).astype(np.uint16)
    levels = np.where(codes <= 1, 0, (codes >> 16).astype(np.int16) - 16384)
    wet = slots != 0
    kinds = metadata[slots]
    delta = np.zeros(base.shape, dtype=np.float32)
    # Beds are baked against surveyed sea/lake levels. Keep existing bathymetry where present.
    depth = np.where(kinds == 4, 4, np.where(kinds >= 2, 10, 2))
    delta[wet] = np.minimum(0, (levels - depth - base)[wet])
    # Feather the bank/valley into source relief. Both sides have a common physical level.
    if wet.any() and not wet.all():
        distance, nearest = distance_transform_edt(
            ~wet,
            return_indices=True
        )
        bank_level = levels[tuple(nearest)]
        channel = kinds[tuple(nearest)] == 4
        t = np.clip((distance - 1) / np.where(channel, 6, 8), 0, 1)
        feather = 1 - t * t * (3 - 2 * t)
        bank = np.where(channel, np.clip(base, bank_level + 1, bank_level + distance * 6),
                        np.clip(base, bank_level + 1, bank_level + distance * 15))
        delta[~wet] = ((bank - base) * feather)[~wet]
    return np.rint(delta).astype("<i2"), levels, (kinds << 12) | slots


def bake_water(cache: Path, output: Path, dem):
    layers = source_layers(cache, dem, output)
    metadata = np.array([int(row.split("\t")[2]) for row in (output / "water.tsv").read_text().splitlines()])
    if len(metadata) > 4096:
        raise ValueError("Water catalog exceeds the packed identifier range")
    index = np.zeros((HEIGHT // PAGE, WIDTH // PAGE), dtype="<u4")
    page_count = 0
    compressor = zstandard.ZstdCompressor(level=9)
    with (output / "water-pages.zst").open("wb") as handle, compressor.stream_writer(handle) as stream:
        for y in range(0, HEIGHT, BATCH):
            if y % (BATCH * 5) == 0:
                print(f"Water rows {y}/{HEIGHT}, {page_count} detailed pages", flush=True)
            for x in range(0, WIDTH, BATCH):
                w, h = min(BATCH, WIDTH - x), min(BATCH, HEIGHT - y)
                # Exclude homogeneous deep ocean and land from dense rasterization.
                west, east = (x - HALO) / 240 - 180, (x + w + HALO + 1) / 240 - 180
                north, south = 90 - (y - HALO) / 240, 90 - (y + h + HALO + 1) / 240
                region = box(west, south, east, north)
                candidates = layers[0].tree.query(region, predicate="intersects")
                has_water = any(len(layer.tree.query(region)) for layer in layers[1:])
                wholly_land = any(layers[0].shapes[i][0].covers(region) for i in candidates)
                interior = np.s_[y // PAGE:(y + h) // PAGE, x // PAGE:(x + w) // PAGE]
                if west > -180 and east < 180 and north < 90 and south > -90 and not has_water:
                    if len(candidates) == 0 or wholly_land:
                        index[interior] = 0 if wholly_land else (1 << 12) | 1
                        continue

                w_ext, h_ext = w + 2 * HALO + 1, h + 2 * HALO + 1
                codes = codes_at(layers, x - HALO, y - HALO, w_ext, h_ext, PIXELS_PER_DEGREE)
                # Clamp guard samples at the poles, matching the runtime addressing.
                if y == 0:
                    codes[:HALO] = codes[HALO]
                if y + h == HEIGHT:
                    codes[HALO + h:] = codes[HALO + h - 1]
                delta, level, slot = bake_relief(dem, codes, metadata, x - HALO, y - HALO, w_ext, h_ext)
                offsets = np.s_[HALO:HALO + h:PAGE, HALO:HALO + w:PAGE]
                windows = np.lib.stride_tricks.sliding_window_view
                code_tiles = windows(codes, (STRIDE, STRIDE))[offsets]
                delta_tiles = windows(delta, (STRIDE, STRIDE))[offsets]
                detailed = np.any(code_tiles != code_tiles[:, :, :1, :1], axis=(-2, -1))
                detailed |= np.any(delta_tiles, axis=(-2, -1))
                entries = (codes[offsets] & 0xFFFF0000) | slot[offsets].astype(np.uint32)
                count = np.count_nonzero(detailed)
                entries[detailed] = PAGE_FLAG | np.arange(page_count, page_count + count, dtype=np.uint32)
                index[interior] = entries
                if count:
                    correction = delta_tiles[detailed]
                    level_tiles = windows(level, (STRIDE, STRIDE))[offsets][detailed]
                    slot_tiles = windows(slot, (STRIDE, STRIDE))[offsets][detailed]
                    data = np.stack((correction, level_tiles, slot_tiles, np.zeros_like(correction)), axis=-1)
                    stream.write(data.astype("<i2").tobytes())
                    page_count += count

    with (output / "water-index.zst").open("wb") as file:
        file.write(compressor.compress(index.tobytes()))
    print(f"Baked {page_count} water pages", flush=True)
