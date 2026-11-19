struct Draw {
    index_count: u32,
    instance_count: atomic<u32>,
    first_index: u32,
    base_vertex: i32,
    first_instance: u32,
}
@group(0) @binding(1) var<storage, read> bounds: array<vec2<f32>>;
@group(1) @binding(0) var<storage, read_write> draws: array<Draw>;
@group(1) @binding(1) var<storage, read_write> land_tiles: array<vec4<u32>>;
@group(1) @binding(2) var<storage, read_write> water_tiles: array<vec4<u32>>;
@group(1) @binding(3) var<storage, read> buckets: array<vec4<u32>>;

const RING_RADIUS = 3.0;

fn must_split(face: u32, lod: u32, tile: vec2<u32>) -> bool {
    if lod >= view.settings.x { return false; }
    if lod < 4u { return true; }
    let dim = f32(1u << lod);
    let angle = QUARTER_TURN / dim;
    let center = direction(face, (vec2<f32>(tile) + 0.5) / dim);
    // Both tests are monotone on ancestors. The angular margin also guarantees 2:1
    // neighbors: a level l-1 neighbor center is at most 2 sqrt(2) cell widths away.
    let cell = view.planet.x * angle / f32(view.settings.z);
    let sagitta = angle * angle * view.planet.x / (4.0 * f32(view.settings.z * view.settings.z));
    let error_m = max(sagitta, cell * 0.15);
    return distance(center, view.eye.xyz) < RING_RADIUS * angle
        && error_m * view.planet.w > 3.5 * view.eye.w;
}

fn tile_range(face: u32, lod: u32, tile: vec2<u32>) -> vec2<f32> {
    let last = firstTrailingBit(view.source.x);
    let mip = min(lod, last);
    let xy = tile >> vec2(lod - mip);
    let dim = 1u << mip;
    let stride = ((1u << (2u * (last + 1u))) - 1u) / 3u;
    return bounds[face * stride + (dim * dim - 1u) / 3u + xy.y * dim + xy.x];
}

fn visible(face: u32, uv: vec2<f32>, angle: f32, range: vec2<f32>) -> bool {
    let unit = direction(face, uv);
    // The equal-angle chart's partial derivatives have norm <= sqrt(2).
    let ang_radius = angle * sqrt(2.0);
    let r_max = view.planet.x + range.y;
    let occluder = view.planet.y;
    if view.planet.z > occluder {
        let horizon = acos(clamp(occluder / view.planet.z, 0.0, 1.0))
            + acos(clamp(occluder / r_max, 0.0, 1.0)) + ang_radius;
        if horizon < PI && dot(unit, view.eye.xyz) < cos(horizon) { return false; }
    }
    let mid = (range.x + range.y) * 0.5;
    let center = relative_on_sphere(face, uv) + unit * mid;
    let radius = ang_radius * r_max + (range.y - mid) + 4.0;
    let rows = transpose(view.clip_from_eye);
    let clip = view.clip_from_eye * vec4(center, 1.0);
    return abs(clip.x) - clip.w <= radius * length((rows[0] + rows[3]).xyz)
        && abs(clip.y) - clip.w <= radius * length((rows[1] + rows[3]).xyz)
        && clip.w >= -radius;
}

fn needs_stitch(face: u32, lod: u32, tile: vec2<u32>, axis: u32, step: i32) -> bool {
    if lod == 0u { return false; }
    let dim = 1u << lod;
    var next = vec2<i32>(tile);
    next[axis] += step;
    var next_face = face;
    if next[axis] < 0 || next[axis] >= i32(dim) {
        let along = 1u - axis;
        let across = select(RIGHT[face], UP[face], axis == 1u) * f32(step);
        let edge = select(UP[face], RIGHT[face], axis == 1u);
        next_face = face_of(across);
        // Integer chart-space seam transform. Along-edge indices are preserved or reversed.
        let other_axis = u32(abs(dot(RIGHT[next_face], NORMAL[face])) < 0.5);
        let other_across = select(RIGHT[next_face], UP[next_face], other_axis == 1u);
        let other_edge = select(UP[next_face], RIGHT[next_face], other_axis == 1u);
        next[other_axis] = select(0, i32(dim) - 1, dot(other_across, NORMAL[face]) > 0.0);
        next[1u - other_axis] = select(i32(tile[along]), i32(dim - 1u - tile[along]),
            dot(edge, other_edge) < 0.0);
    }
    return !must_split(next_face, lod - 1u, vec2<u32>(next) >> vec2(1u));
}

fn may_have_water(face: u32, lod: u32, tile: vec2<u32>, min_height: f32) -> bool {
    if min_height <= 0.0 { return true; }
    let dim = view.source.z;
    let bucket_lod = firstTrailingBit(dim);
    if lod < bucket_lod { return true; }
    let xy = tile >> vec2(lod - bucket_lod);
    let bucket = buckets[(face * dim + xy.y) * dim + xy.x];
    return bucket.y > 0u;
}

@compute @workgroup_size(64)
fn select_tiles(@builtin(workgroup_id) group: vec3<u32>, @builtin(local_invocation_index) lane: u32) {
    let face = group.x;
    let lod = group.y;
    let dim = 1u << lod;
    var first = vec2<u32>(0u);
    var end = vec2<u32>(dim);
    if lod > 4u {
        let angle = RING_RADIUS * QUARTER_TURN / f32(dim >> 1u);
        let n = dot(view.eye.xyz, NORMAL[face]);
        if n + angle < inverseSqrt(3.0) { return; }
        // A split parent must lie in the chord cap. Its 2D axis projections have
        // length >= 1/sqrt(3), so each chart angle moves by at most asin(angle * sqrt(3)).
        let uv = view.anchors[face].uv.xy;
        let extent = asin(min(1.0, angle * sqrt(3.0))) / QUARTER_TURN;
        first = vec2<u32>(clamp(floor((uv - extent) * f32(dim >> 1u)) - 1.0,
            vec2(0.0), vec2(f32(dim >> 1u)))) * 2u;
        end = vec2<u32>(clamp(ceil((uv + extent) * f32(dim >> 1u)) + 1.0,
            vec2(0.0), vec2(f32(dim >> 1u)))) * 2u;
    }

    let width = end.x - first.x;
    let count = width * (end.y - first.y);
    for (var i = lane; i < count; i += 64u) {
        let tile = first + vec2(i % width, i / width);
        if lod > 0u && !must_split(face, lod - 1u, tile >> vec2(1u)) { continue; }
        if must_split(face, lod, tile) { continue; }
        let range = tile_range(face, lod, tile);
        let water = may_have_water(face, lod, tile, range.x);
        // Water may be above the highest land sample in a fully submerged tile.
        let cull_range = vec2(range.x, select(range.y, max(range.y, 0.0), water));
        if !visible(face, (vec2<f32>(tile) + 0.5) / f32(dim), QUARTER_TURN / f32(dim), cull_range) {
            continue;
        }
        let edges = u32(needs_stitch(face, lod, tile, 0u, -1))
            | (u32(needs_stitch(face, lod, tile, 0u, 1)) << 1u)
            | (u32(needs_stitch(face, lod, tile, 1u, -1)) << 2u)
            | (u32(needs_stitch(face, lod, tile, 1u, 1)) << 3u);
        let item = vec4(tile, lod, face | (edges << 3u));
        land_tiles[atomicAdd(&draws[0].instance_count, 1u)] = item;
        if water { water_tiles[atomicAdd(&draws[1].instance_count, 1u)] = item; }
    }
}
