// The same face order and equal-angle projection as strata_globe::CubeFace.
const NORMAL = array<vec3<f32>, 6>(
    vec3(1,0,0), vec3(-1,0,0), vec3(0,1,0), vec3(0,-1,0), vec3(0,0,1), vec3(0,0,-1));
const RIGHT = array<vec3<f32>, 6>(
    vec3(0,0,-1), vec3(0,0,1), vec3(1,0,0), vec3(1,0,0), vec3(1,0,0), vec3(-1,0,0));
const UP = array<vec3<f32>, 6>(
    vec3(0,1,0), vec3(0,1,0), vec3(0,0,-1), vec3(0,0,1), vec3(0,1,0), vec3(0,1,0));
const PI = 3.141592653589793;
const QUARTER_TURN = PI * 0.5;

struct FaceAnchor {
    // Relative sea-level position was evaluated in f64 with the authoritative camera eye.
    relative: vec4<f32>,
    // Unit direction and raw cube-vector length.
    unit_length: vec4<f32>,
    uv: vec4<f32>,
    tangent: vec4<f32>,
}

struct View {
    clip_from_eye: mat4x4<f32>,
    anchors: array<FaceAnchor, 6>,
    // Radius, occluding radius, eye's radial distance, target projection scale in pixels.
    planet: vec4<f32>,
    // Eye direction and clearance above the land/water, in meters.
    eye: vec4<f32>,
    // Maximum LOD, debug overlay, grid resolution, baked Earth atlas flag.
    settings: vec4<u32>,
    // Macro grid resolution, next noise lattice offset (words), reserved, first octave frequency.
    source: vec4<u32>,
    // Second octave frequency, followed by reserved fields.
    detail: vec4<u32>,
    material_origin: vec4<f32>,
}

@group(0) @binding(0) var<uniform> view: View;

fn face_of(v: vec3<f32>) -> u32 {
    let a = abs(v);
    if a.x >= a.y && a.x >= a.z { return select(1u, 0u, v.x >= 0.0); }
    if a.y >= a.z { return select(3u, 2u, v.y >= 0.0); }
    return select(5u, 4u, v.z >= 0.0);
}

fn face_uv(face: u32, v: vec3<f32>) -> vec2<f32> {
    let n = dot(v, NORMAL[face]);
    return clamp(atan2(vec2(dot(v, RIGHT[face]), dot(v, UP[face])), vec2(n))
        / QUARTER_TURN + 0.5, vec2(0.0), vec2(1.0));
}

fn cube_tangent(uv: vec2<f32>) -> vec2<f32> {
    let angle = 2.0 * uv - 1.0;
    return select(tan(angle * (PI * 0.25)), angle, abs(angle) == vec2(1.0));
}

fn direction(face: u32, uv: vec2<f32>) -> vec3<f32> {
    let t = cube_tangent(uv);
    return normalize(NORMAL[face] + RIGHT[face] * t.x + UP[face] * t.y);
}

fn relative_on_sphere(face: u32, uv: vec2<f32>) -> vec3<f32> {
    let anchor = view.anchors[face];
    // tan(a+d)-tan(a), then |a+d|-|a| by rationalizing the difference.
    // This keeps the significant bits of small offsets at an Earth-sized radius.
    let delta_tan = tan((uv - anchor.uv.xy) * QUARTER_TURN);
    let tangent = anchor.tangent.xy;
    let dt = delta_tan * (1.0 + tangent * tangent) / (1.0 - tangent * delta_tan);
    let delta = dt.x * RIGHT[face] + dt.y * UP[face];
    let raw = NORMAL[face] + tangent.x * RIGHT[face] + tangent.y * UP[face];
    let len = length(raw + delta);
    let delta_len = dot(2.0 * raw + delta, delta) / (anchor.unit_length.w + len);
    let delta_unit = (delta - anchor.unit_length.xyz * delta_len) / len;
    return anchor.relative.xyz + delta_unit * view.planet.x;
}

const RING_RADIUS = 3.0;

fn split_ratio(face: u32, lod: u32, tile: vec2<u32>) -> f32 {
    if lod < 4u { return 2.0; }
    let dim = f32(1u << lod);
    let angle = QUARTER_TURN / dim;
    let center = direction(face, (vec2<f32>(tile) + 0.5) / dim);
    // Both tests are monotone on ancestors. The angular margin guarantees 2:1
    // neighbors: a level l-1 neighbor center is at most 2 sqrt(2) cell widths away.
    let cell = view.planet.x * angle / f32(view.settings.z);
    let sagitta = angle * angle * view.planet.x / (4.0 * f32(view.settings.z * view.settings.z));
    let exaggeration = select(0.15, 0.30, view.settings.w != 0u);
    let error_m = max(sagitta, cell * exaggeration);
    return min(
        RING_RADIUS * angle / max(distance(center, view.eye.xyz), 1e-9),
        error_m * view.planet.w / (3.5 * view.eye.w)
    );
}

fn adjacent_tile(face: u32, lod: u32, tile: vec2<u32>, axis: u32, step: i32) -> vec3<u32> {
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
    return vec3(vec2<u32>(next), next_face);
}
