@group(0) @binding(7) var local_amplitudes: texture_2d<f32>;
@group(0) @binding(8) var local_shapes: texture_2d_array<f32>;
@group(0) @binding(9) var amplitude_sampler: sampler;
@group(0) @binding(10) var shape_sampler: sampler;

struct LocalRelief {
    height: f32,
    grade: vec3<f32>,
    material: f32,
}

fn local_sheet(p: vec2<f32>, layer: i32) -> vec4<f32> {
    return textureSampleLevel(local_shapes, shape_sampler, (p + 0.5) / 256.0, layer, 0.0);
}

fn local_projection(position: vec3<f32>, dir: vec3<f32>, layer: i32, step: f32) -> LocalRelief {
    let p = position / step;
    let a = local_sheet(p.yz + vec2(71.0, 113.0), layer);
    let b = local_sheet(p.zx + vec2(13.0, 41.0), layer);
    let c = local_sheet(p.xy, layer);
    let w = dir * dir;
    let weights = vec3(a.x, b.x, c.x) * w;
    let grade = a.yzz * vec3(0.0, w.x, w.x)
        + b.zyy * vec3(w.y, 0.0, w.y)
        + c.yzy * vec3(w.z, w.z, 0.0);
    return LocalRelief(weights.x + weights.y + weights.z, grade / step,
        dot(vec3(a.w, b.w, c.w), w));
}

fn local_relief(uv: vec2<f32>, position: vec3<f32>, dir: vec3<f32>,
                dry: f32, band: vec2<f32>) -> LocalRelief {
    var result = LocalRelief(0.0, vec3(0.0), 0.0);
    if dry <= 0.0 || all(band <= vec2(0.0)) { return result; }
    let controls = textureSampleLevel(local_amplitudes, amplitude_sampler, uv, 0.0).xy * dry;
    let weights = vec3(controls * band.x, (controls.x + controls.y) * band.y);
    let amplitudes = vec3(120.0, 12.0, 3.0) * weights;
    for (var i = 0; i < 3; i++) {
        if amplitudes[i] > 0.0 {
            let s = local_projection(position, dir, i, select(32.0, 2.0, i == 2));
            result.height += s.height * amplitudes[i];
            result.grade += s.grade * amplitudes[i];
            result.material += s.material * weights[i] * select(0.24, 0.27, i == 2);
        }
    }
    return result;
}

fn local_position(sphere_relative: vec3<f32>) -> vec3<f32> {
    return sphere_relative + view.material_origin.xyz;
}

fn earth_ground_material(position: vec3<f32>, dir: vec3<f32>, footprint: f32) -> LocalRelief {
    let fade = 1.0 - smoothstep(0.07, 0.75, footprint);
    if fade <= 0.0 { return LocalRelief(0.0, vec3(0.0), 0.0); }
    let sample = local_projection(position, dir, 3, 0.25);
    let chips = 1.0 - smoothstep(0.035, 0.11, footprint);
    var result = LocalRelief(0.0, sample.grade * (0.23 * fade), sample.material * (0.52 * fade));
    if chips > 0.0 {
        let stones = local_projection(position + vec3(1.25, 5.75, 19.0), dir, 3, 0.0625);
        result.grade += stones.grade * (0.026 * chips);
        result.material += stones.material * (0.33 * chips);
    }
    return result;
}
