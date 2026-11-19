@group(0) @binding(1) var dem: texture_2d<f32>;
@group(0) @binding(2) var climate_grid: texture_2d_array<f32>;
@group(0) @binding(3) var<storage, read> gradients: array<u32>;
@group(0) @binding(4) var<storage, read> bins: array<vec4<u32>>;
@group(0) @binding(5) var<storage, read> features: array<mat4x4<f32>>;

fn dem_coordinates(dir: vec3<f32>) -> vec2<f32> {
    let latitude = atan2(dir.y, length(dir.xz));
    let longitude = atan2(dir.x, dir.z);
    let uv = vec2(fract(longitude / (2.0 * PI) + 0.5), 0.5 - latitude / PI);
    let size = vec2<f32>(textureDimensions(dem));
    if view.settings.w != 0u {
        return vec2(uv.x * size.x - 0.5, clamp(uv.y * size.y - 0.5, 0.0, size.y - 1.0));
    }
    return uv * vec2(size.x, size.y - 1.0);
}

fn dem_quad(dir: vec3<f32>) -> vec2<f32> {
    let size = vec2<i32>(textureDimensions(dem));
    let p = dem_coordinates(dir);
    let y = clamp(i32(floor(p.y)), 0, size.y - 2);
    let x = (i32(floor(p.x)) + size.x) % size.x;
    let next_x = (x + 1) % size.x;
    let top = mix(textureLoad(dem, vec2(x, y), 0).x, textureLoad(dem, vec2(next_x, y), 0).x, fract(p.x));
    let bottom = mix(textureLoad(dem, vec2(x, y + 1), 0).x,
        textureLoad(dem, vec2(next_x, y + 1), 0).x, fract(p.x));
    return vec2(top, bottom) * vec2(1.0 - (p.y - f32(y)), p.y - f32(y));
}

fn macro_fields(face: u32, uv: vec2<f32>) -> vec4<f32> {
    let n = f32(view.source.x);
    let p = uv * n;
    let xy = vec2<i32>(clamp(floor(p), vec2(0.0), vec2(n - 1.0)));
    let t = p - vec2<f32>(xy);
    let layer = i32(face);
    let a = textureLoad(climate_grid, xy, layer, 0);
    let b = textureLoad(climate_grid, xy + vec2(1, 0), layer, 0);
    let c = textureLoad(climate_grid, xy + vec2(0, 1), layer, 0);
    let d = textureLoad(climate_grid, xy + vec2(1, 1), layer, 0);
    return mix(mix(a, b, t.x), mix(c, d, t.x), t.y);
}

const GRAD = array<vec3<f32>, 16>(
    vec3(1,1,0),vec3(-1,1,0),vec3(1,-1,0),vec3(-1,-1,0),
    vec3(1,0,1),vec3(-1,0,1),vec3(1,0,-1),vec3(-1,0,-1),
    vec3(0,1,1),vec3(0,-1,1),vec3(0,1,-1),vec3(0,-1,-1),
    vec3(1,1,0),vec3(-1,1,0),vec3(0,-1,1),vec3(0,-1,-1));

fn grad_dot(at: vec3<u32>, offset: u32, side: u32, delta: vec3<f32>) -> f32 {
    let i = (at.z * side + at.y) * side + at.x;
    let word = gradients[offset + i / 8u];
    return dot(GRAD[(word >> ((i % 8u) * 4u)) & 15u], delta);
}

fn detail_noise(dir: vec3<f32>, freq: u32, offset: u32) -> f32 {
    // Clamp the lattice cell, so an exact +axis uses the last lattice vertex.
    let p = clamp(dir, vec3(-1.0), vec3(1.0)) * f32(freq);
    let cell = min(floor(p), vec3(f32(freq - 1u)));
    let c = vec3<u32>(cell + f32(freq));
    let d = p - cell;
    let w = d*d*d * (d*(d*6.0 - 15.0) + 10.0);
    let s = 2u * freq + 1u;
    let a = mix(grad_dot(c, offset, s, d),
        grad_dot(c + vec3(1u,0u,0u), offset, s, d - vec3(1,0,0)), w.x);
    let b = mix(grad_dot(c + vec3(0u,1u,0u), offset, s, d - vec3(0,1,0)),
        grad_dot(c + vec3(1u,1u,0u), offset, s, d - vec3(1,1,0)), w.x);
    let e = mix(grad_dot(c + vec3(0u,0u,1u), offset, s, d - vec3(0,0,1)),
        grad_dot(c + vec3(1u,0u,1u), offset, s, d - vec3(1,0,1)), w.x);
    let f = mix(grad_dot(c + vec3(0u,1u,1u), offset, s, d - vec3(0,1,1)),
        grad_dot(c + vec3(1u,1u,1u), offset, s, d - vec3(1,1,1)), w.x);
    return clamp(mix(mix(a,b,w.y), mix(e,f,w.y), w.z) * 1.05, -1.0, 1.0);
}

fn hydrology_bin(face: u32, uv: vec2<f32>) -> vec4<u32> {
    let dim = view.source.z;
    let xy = min(vec2<u32>(uv * f32(dim)), vec2(dim - 1u));
    return bins[(face * dim + xy.y) * dim + xy.x];
}

// Kind: 0 = dry, 1 = ocean, 2 = lake, 3 = saline lake, 4 = river.
// Water level is also retained outside inland water in the bank/valley collar.
struct Surface { height: f32, level: f32, kind: f32 }
fn sample_surface(face: u32, uv: vec2<f32>, dir: vec3<f32>) -> Surface {
    let q = dem_quad(dir);
    var height = q.x + q.y;
    if height > 0.0 {
        let roughness = macro_fields(face, uv).y;
        if roughness > 0.04 {
            let n0 = detail_noise(dir, view.source.w, 0u);
            let n1 = detail_noise(dir, view.detail.x, view.source.y);
            height += (n0 * 0.68 + n1 * 0.32) * 160.0 * roughness * smoothstep(0.0, 40.0, height);
        }
    }
    let bin = hydrology_bin(face, uv);
    var level = 0.0;
    var kind = 0.0;
    var depth = 0.0;
    for (var i = bin.x; i < bin.x + bin.z; i++) {
        let lake = features[i];
        let distance_rad = 2.0 * asin(min(1.0, length(dir - lake[0].xyz) * 0.5));
        let radius = lake[0].w;
        let collar = radius * 1.38;
        if distance_rad < radius {
            let r = distance_rad / radius;
            let d = lake[1].y * pow(1.0 - r*r, 0.85);
            if d > depth {
                depth = d;
                level = lake[1].x;
                height = level - depth;
                kind = lake[1].z;
            }
        } else if distance_rad < collar && kind == 0.0 {
            level = lake[1].x;
            height = mix(height, level, 0.9 * (1.0 - smoothstep(radius, collar, distance_rad)));
        }
    }
    if kind != 0.0 { return Surface(height, level, kind); }

    var best = 1e20;
    for (var i = bin.x + bin.z; i < bin.x + bin.y; i++) {
        let river = features[i];
        let a = river[0].xyz;
        let ab = river[1].xyz;
        let t = clamp(dot(dir - a, ab) / max(dot(ab, ab), 1e-12), 0.0, 1.0);
        let raw = a + ab * t;
        let d = 2.0 * asin(min(1.0, length(dir - normalize(raw)) * 0.5)) * view.planet.x;
        let p = mix(river[2], river[3], t);
        let ratio = d / max(p.y, 1.0);
        if d >= p.w || ratio >= best { continue; }
        level = p.x;
        if d < p.y {
            best = ratio;
            height = level - p.z * (1.0 - ratio*ratio);
            kind = 4.0;
        } else {
            best = p.w / max(p.y, 1.0) + ratio;
            height = mix(height, level, 0.85 * (1.0 - smoothstep(p.y, p.w, d)));
            kind = 0.0;
        }
    }
    if kind == 0.0 && height < 0.0 { return Surface(height, 0.0, 1.0); }
    return Surface(height, level, kind);
}
