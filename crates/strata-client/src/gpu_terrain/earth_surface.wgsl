@group(0) @binding(4) var earth_dem: texture_2d_array<i32>;
@group(0) @binding(5) var water_directory: texture_2d<u32>;
@group(0) @binding(6) var water_pages: texture_2d_array<i32>;

const RELIEF_SIZE = vec2(21600.0, 10800.0);
const WATER_SIZE = vec2(86400.0, 43200.0);

fn earth_uv(dir: vec3<f32>) -> vec2<f32> {
    return vec2(atan2(dir.x, dir.z) / (2.0 * PI) + 0.5,
        0.5 - atan2(dir.y, length(dir.xz)) / PI);
}

fn earth_texel(point: vec2<i32>) -> f32 {
    let x = (point.x + 21600) % 21600;
    let y = clamp(point.y, 0, 10799);
    let layer = y / 5400 * 4 + x / 5400;
    return f32(textureLoad(earth_dem, vec2(x % 5400, y % 5400), layer, 0).x);
}

fn earth_relief(point: vec2<f32>) -> f32 {
    let p = vec2<i32>(floor(point));
    let t = fract(point);
    let a = earth_texel(p);
    let b = earth_texel(p + vec2(1, 0));
    let c = earth_texel(p + vec2(0, 1));
    let d = earth_texel(p + vec2(1, 1));
    return mix(mix(a, b, t.x), mix(c, d, t.x), t.y);
}

struct EarthWaterPoint {
    offset: f32,
    level: f32,
    kind: f32,
    coverage: f32,
    protection: f32,
}

fn water_texel(page: u32, local: vec2<i32>) -> vec3<f32> {
    let row_count = textureDimensions(water_pages).y;
    let p = vec2(local.y * 17 + local.x, i32(page % row_count));
    return vec3<f32>(textureLoad(water_pages, p, i32(page / row_count), 0).xyz);
}

fn earth_water(uv: vec2<f32>) -> EarthWaterPoint {
    let point = uv * WATER_SIZE - 0.5;
    let p = vec2(
        (i32(floor(point.x)) + 86400) % 86400,
        clamp(i32(floor(point.y)), 0, 43199)
    );
    let entry = textureLoad(water_directory, p / 16, 0).x;
    if entry < 0x80000000u {
        let kind = f32((entry >> 12u) & 15u);
        let level_code = entry >> 16u;
        let level = select(f32(level_code) - 16384.0, 0.0, level_code == 0u);
        return EarthWaterPoint(0.0, level, kind, f32(kind > 0.0), f32(kind > 0.0));
    }

    let page = entry & 0x7fffffffu;
    let cell = p % 16;
    let quad = array(
        water_texel(page, cell), water_texel(page, cell + vec2(1, 0)),
        water_texel(page, cell + vec2(0, 1)), water_texel(page, cell + vec2(1, 1))
    );
    let t = vec2(fract(point.x), fract(clamp(point.y, 0.0, 43199.0)));
    let weights = vec4((1.0 - t.x) * (1.0 - t.y), t.x * (1.0 - t.y),
        (1.0 - t.x) * t.y, t.x * t.y);
    var wet = 0.0;
    var offset = 0.0;
    var protection = 0.0;
    var level = 0.0;
    var best = -1.0;
    var kind = 0.0;
    for (var i = 0u; i < 4u; i++) {
        offset += quad[i].x * weights[i];
        if quad[i].x != 0.0 || quad[i].z > 0.0 { protection += weights[i]; }
        if quad[i].z > 0.0 {
            wet += weights[i];
            level += quad[i].y * weights[i];
            if weights[i] > best {
                best = weights[i];
                kind = f32(u32(quad[i].z) >> 12u);
            }
        }
    }
    return EarthWaterPoint(offset, level / max(wet, 1e-12), kind, wet, protection);
}

fn earth_point(dir: vec3<f32>) -> vec2<f32> {
    let p = earth_uv(dir) * RELIEF_SIZE - 0.5;
    return vec2(p.x, clamp(p.y, 0.0, RELIEF_SIZE.y - 1.0));
}

fn earth_normal(dir: vec3<f32>, footprint_m: f32, grade: vec3<f32>) -> vec3<f32> {
    let p = earth_point(dir);
    let east_length = length(dir.xz);
    let east = vec3(dir.z, 0.0, -dir.x) / max(east_length, 1e-6);
    let north = cross(dir, east);
    let meters_per_texel = PI * view.planet.x / RELIEF_SIZE.y;
    let scale = 0.5 / meters_per_texel;
    let slope_x = (earth_relief(p + vec2(1.0, 0.0)) - earth_relief(p - vec2(1.0, 0.0)))
        * scale / max(east_length, 1e-4);
    let slope_y = (earth_relief(p + vec2(0.0, 1.0)) - earth_relief(p - vec2(0.0, 1.0))) * scale;
    // Derivatives of the immutable DEM are independent of the triangulation/selection.
    let weight = 1.0 - smoothstep(meters_per_texel * 0.5, meters_per_texel * 2.0, footprint_m);
    let tangent_grade = grade - dir * dot(grade, dir);
    return normalize(dir - (east * slope_x - north * slope_y) * weight - tangent_grade);
}

fn earth_land(height: f32, slope: f32, fields: vec4<f32>) -> vec3<f32> {
    let temp = fields.z - 0.0065 * (max(0.0, height) - max(0.0, fields.x));
    let rain = fields.w;
    let humidity = smoothstep(0.32, 1.65, rain / (220.0 + max(temp, 0.0) * 34.0));
    let warmth = smoothstep(0.0, 18.0, temp);
    let sand = mix(vec3(0.57, 0.54, 0.46), vec3(0.76, 0.62, 0.42), warmth);
    let humid = mix(vec3(0.47, 0.50, 0.37), vec3(0.24, 0.36, 0.19), warmth);
    var color = mix(sand, humid, humidity);
    let rock = max(smoothstep(0.22, 0.72, slope), 0.48 * smoothstep(2800.0, 4700.0, height));
    color = mix(color, vec3(0.37, 0.35, 0.32), rock);
    let snow = (1.0 - smoothstep(-11.0, 2.0, temp))
        * mix(smoothstep(75.0, 250.0, rain), 1.0, 1.0 - smoothstep(-11.0, -3.0, temp));
    return mix(color, vec3(0.89, 0.92, 0.95), snow * (1.0 - 0.55 * rock));
}
