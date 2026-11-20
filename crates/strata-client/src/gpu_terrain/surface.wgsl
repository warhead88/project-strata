@group(0) @binding(1) var dem: texture_2d<f32>;
@group(0) @binding(2) var climate_grid: texture_2d_array<f32>;
@group(0) @binding(3) var<storage, read> gradients: array<u32>;

fn dem_coordinates(dir: vec3<f32>) -> vec2<f32> {
    let latitude = atan2(dir.y, length(dir.xz));
    let longitude = atan2(dir.x, dir.z);
    let uv = vec2(fract(longitude / (2.0 * PI) + 0.5), 0.5 - latitude / PI);
    let size = vec2<f32>(textureDimensions(dem));
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

// Kind: 0 = dry, 1 = ocean, 2 = lake, 3 = saline lake, 4 = river.
struct Surface { height: f32, level: f32, kind: f32, shore: f32, wet_kind: f32 }
fn sample_surface(face: u32, uv: vec2<f32>, dir: vec3<f32>) -> Surface {
    if view.settings.w != 0u {
        let water = earth_water(earth_uv(dir));
        let height = earth_relief(earth_point(dir)) + water.offset;
        let kind = select(0.0, water.kind, water.coverage >= 0.5 && water.level > height);
        let shore = min(water.coverage - 0.5, (water.level - height) / 12.0);
        return Surface(height, water.level, kind, shore, water.kind);
    }
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
    let kind = select(0.0, 1.0, height < 0.0);
    return Surface(height, 0.0, kind, -height, kind);
}
