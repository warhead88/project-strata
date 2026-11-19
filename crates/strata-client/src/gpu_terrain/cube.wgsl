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
    // Maximum LOD, debug overlay, grid resolution, geographic raster pixel-centered flag.
    settings: vec4<u32>,
    // Macro grid resolution, next noise lattice offset (words), hydrology bins/axis.
    source: vec4<u32>,
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
