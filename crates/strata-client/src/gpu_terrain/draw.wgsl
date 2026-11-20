@group(1) @binding(0) var<storage, read> tiles: array<vec4<u32>>;
override water_pass: bool = false;

struct RasterVertex {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) @interpolate(flat) tile: vec4<u32>,
    @location(2) relative: vec3<f32>,
    @location(3) height: f32,
    @location(4) water: vec2<f32>,
    @location(5) sphere_relative: vec3<f32>,
}

struct TerrainVertex { relative: vec3<f32>, surface: Surface, sphere_relative: vec3<f32> }

fn terrain_vertex(face: u32, uv: vec2<f32>) -> TerrainVertex {
    // Canonicalize cube edges using exact chart coordinates, including all three cube corners.
    let chart = NORMAL[face] + RIGHT[face] * (2.0 * uv.x - 1.0) + UP[face] * (2.0 * uv.y - 1.0);
    let owner = face_of(chart);
    let canonical_uv = vec2(dot(chart, RIGHT[owner]), dot(chart, UP[owner])) * 0.5 + 0.5;
    let dir = direction(owner, canonical_uv);
    let sphere_relative = relative_on_sphere(owner, canonical_uv);
    var s: Surface;
    if view.settings.w != 0u {
        if water_pass {
            let water = earth_water(earth_uv(dir));
            s = earth_surface(earth_relief(earth_point(dir)) + water.offset, water);
        } else {
            s = earth_vertex(dir, sphere_relative, true);
        }
    } else {
        s = sample_surface(owner, canonical_uv, dir);
    }
    let radius_min = min(1.0, view.planet.x * 0.5);
    let level = select(s.height, s.level, water_pass);
    let rel = sphere_relative + dir * max(level, radius_min - view.planet.x);
    return TerrainVertex(rel, s, sphere_relative);
}

fn morph_vertex(tile: vec4<u32>, face: u32, point: vec2<u32>, base: TerrainVertex, stitched: bool) -> TerrainVertex {
    let odd = point & vec2(1u);
    if all(odd == vec2(0u)) || (!stitched && (tile.z <= 4u || view.settings.w == 0u)) {
        return base;
    }
    let parent = tile.xy >> vec2(1u);
    let lod = tile.z - 1u;
    let n = view.settings.z;
    let stride = n * 2u;
    let local = point - parent * stride;
    var ratio = split_ratio(face, lod, parent);
    // Adjacent parents agree on the morph of odd samples on their shared edge.
    // At a corner both indices are even, so there is no morph.
    for (var axis = 0u; axis < 2u; axis++) {
        if local[axis] == 0u || local[axis] == stride {
            let other = adjacent_tile(face, lod, parent, axis, select(1, -1, local[axis] == 0u));
            ratio = min(ratio, split_ratio(other.z, lod, other.xy));
        }
    }
    let blend = select(smoothstep(1.0, 1.22, ratio), 0.0, stitched);
    if blend >= 1.0 { return base; }
    let scale = f32(n << tile.z);
    // The midpoint lies on a parent-grid edge or its SW-NE triangle diagonal.
    let a = terrain_vertex(face, vec2<f32>(point - odd) / scale);
    let b = terrain_vertex(face, vec2<f32>(point + odd) / scale);
    return TerrainVertex(
        mix((a.relative + b.relative) * 0.5, base.relative, blend), base.surface,
        mix((a.sphere_relative + b.sphere_relative) * 0.5, base.sphere_relative, blend)
    );
}

fn raster_vertex(tile: vec4<u32>, vertex_id: u32) -> RasterVertex {
    let face = tile.w & 7u;
    let edges = tile.w >> 3u;
    let n = view.settings.z;
    var grid = vec2(vertex_id % (n + 1u), vertex_id / (n + 1u));
    let stitched = (grid.x == 0u && (edges & 1u) != 0u)
        || (grid.x == n && (edges & 2u) != 0u)
        || (grid.y == 0u && (edges & 4u) != 0u)
        || (grid.y == n && (edges & 8u) != 0u);
    // Earth keeps the parameterization of the parent triangles even along a stitched
    // edge. An odd boundary point goes to the segment midpoint, not an endpoint.
    if view.settings.w == 0u && stitched {
        grid &= vec2(~1u);
    }
    let point = tile.xy * n + grid;
    let uv = vec2<f32>(point) / f32(n << tile.z);
    let base = morph_vertex(tile, face, point, terrain_vertex(face, uv), stitched);
    let s = base.surface;
    let rel = base.relative;
    let wet = select(-max(abs(s.level - s.height), 0.01), s.level - s.height, s.kind > 0.0);
    return RasterVertex(view.clip_from_eye * vec4(rel, 1.0), uv, tile, rel, s.height, vec2(wet, s.kind), base.sphere_relative);
}

@vertex
fn vertex(@builtin(vertex_index) vertex_id: u32, @builtin(instance_index) instance: u32) -> RasterVertex {
    return raster_vertex(tiles[instance], vertex_id);
}

fn linear(rgb: vec3<f32>) -> vec3<f32> {
    return select(rgb / 12.92, pow((rgb + 0.055) / 1.055, vec3(2.4)), rgb > vec3(0.04045));
}

fn color_land(h: f32, slope: f32, fields: vec4<f32>) -> vec3<f32> {
    let temp = fields.z - 0.0065 * (max(0.0, h) - max(0.0, fields.x));
    let rain = fields.w;
    let arid = max(temp + 6.0, 0.5) * 42.0 / max(rain, 10.0);
    let snow = clamp((2.0 - temp) / 12.0, 0.0, 1.0) * select(clamp(rain / 180.0, 0.2, 1.0), 1.0, temp < -12.0);
    if temp < -9.0 || (snow >= 0.88 && temp < -3.0 && h < 3600.0) { return vec3(0.89, 0.93, 0.97); }
    var color: vec3<f32>;
    if h >= 4400.0 || (h >= 3300.0 && temp < -3.5 && snow < 0.72) {
        color = vec3(0.38, 0.36, 0.34);
    } else if h >= 2600.0 && temp < 5.0 {
        color = vec3(0.40, 0.41, 0.32);
    } else if temp < 2.0 {
        color = mix(vec3(0.43, 0.46, 0.34), vec3(0.58, 0.54, 0.46), smoothstep(120.0, 190.0, 320.0-rain));
    } else if arid >= 4.2 || rain < 190.0 {
        color = mix(vec3(0.58, 0.54, 0.46), vec3(0.76, 0.62, 0.42), smoothstep(12.0, 16.0, temp));
    } else if h <= 14.0 && rain >= 950.0 && temp > 3.0 {
        color = vec3(0.22, 0.33, 0.21);
    } else if temp < 8.0 {
        color = mix(vec3(0.55, 0.49, 0.31), vec3(0.14, 0.24, 0.15), smoothstep(270.0, 380.0, rain));
    } else if temp >= 20.5 {
        color = mix(vec3(0.52, 0.48, 0.24), vec3(0.10, 0.27, 0.11), smoothstep(1300.0, 1800.0, rain));
    } else {
        color = mix(mix(vec3(0.55, 0.49, 0.31), vec3(0.36, 0.46, 0.22), smoothstep(380.0, 490.0, rain)),
            vec3(0.18, 0.32, 0.16), smoothstep(550.0, 750.0, rain));
    }
    if h >= 0.0 && h < 6.0 && slope < 0.25 {
        color = mix(color, vec3(0.74, 0.68, 0.52), 0.55 * (1.0 - h / 6.0));
    }
    let rock = smoothstep(0.30, 0.75, slope);
    color = mix(color, vec3(0.36, 0.34, 0.32), rock * 0.85);
    return mix(color, vec3(0.90, 0.93, 0.96), snow * (1.0 - rock * 0.75) * 0.88);
}

fn color_water(depth: f32, kind: f32) -> vec3<f32> {
    if kind > 3.6 { return vec3(0.06, 0.29, 0.42); }
    if kind > 2.6 {
        return mix(vec3(0.08, 0.39, 0.51), vec3(0.06, 0.31, 0.38), smoothstep(0.0, 120.0, depth));
    }
    if kind > 1.6 { return vec3(0.04, 0.24, 0.43); }
    let shelf = mix(vec3(0.11, 0.48, 0.56), vec3(0.08, 0.39, 0.51), clamp(depth / 12.0, 0.0, 1.0));
    let slope = mix(shelf, vec3(0.03, 0.20, 0.38), clamp((depth - 12.0) / 188.0, 0.0, 1.0));
    return mix(slope, vec3(0.012, 0.065, 0.17), clamp((depth - 200.0) / 4000.0, 0.0, 1.0));
}

// A periodic material field in ECEF coordinates. The eye is reduced modulo the
// field period on CPU, and screen derivatives suppress frequencies below a pixel.
fn material_hash(cell: vec3<i32>, mask: u32) -> f32 {
    let p = bitcast<vec3<u32>>(cell) & vec3(mask);
    let n = (p.x * 0x9e3779b9u) ^ (p.y * 0x85ebca6bu) ^ (p.z * 0xc2b2ae35u);
    let h = (n ^ (n >> 16u)) * 0x7feb352du;
    return f32((h ^ (h >> 15u)) & 65535u) / 65535.0 - 0.5;
}

fn material_noise(p: vec3<f32>, mask: u32) -> f32 {
    let a = vec3<i32>(floor(p));
    let t = fract(p);
    let w = t*t * (3.0 - 2.0*t);
    let v00 = mix(material_hash(a, mask), material_hash(a + vec3(1,0,0), mask), w.x);
    let v10 = mix(material_hash(a + vec3(0,1,0), mask), material_hash(a + vec3(1,1,0), mask), w.x);
    let v01 = mix(material_hash(a + vec3(0,0,1), mask), material_hash(a + vec3(1,0,1), mask), w.x);
    let v11 = mix(material_hash(a + vec3(0,1,1), mask), material_hash(a + vec3(1,1,1), mask), w.x);
    return mix(mix(v00, v10, w.y), mix(v01, v11, w.y), w.z);
}

fn land_grain(rel: vec3<f32>, footprint: f32) -> f32 {
    let dist = length(rel);
    if dist >= 2000.0 { return 1.0; }
    let p = rel + view.material_origin.xyz;
    var grain = 0.0;
    let cells = array(16.0, 2.0, 0.5);
    let weights = array(0.12, 0.18, 0.16);
    for (var i = 0u; i < 3u; i++) {
        let width = cells[i];
        if footprint < width * 0.75 {
            let fade = 1.0 - smoothstep(width * 0.2, width * 0.75, footprint);
            grain += material_noise(p / width, u32(256.0 / width) - 1u) * weights[i] * fade;
        }
    }
    return 1.0 + grain * (1.0 - smoothstep(250.0, 2000.0, dist));
}

@fragment
fn fragment(input: RasterVertex) -> @location(0) vec4<f32> {
    let face = input.tile.w & 7u;
    let radial = direction(face, input.uv);
    let normal = normalize(cross(dpdy(input.relative), dpdx(input.relative)));
    let footprint = max(length(dpdx(input.relative)), length(dpdy(input.relative)));
    let earth = view.settings.w != 0u;
    var surface_normal = select(normal, radial, water_pass);

    let sun = normalize(vec3(1.0, 0.8, 0.6));
    var illumination = 0.38 + 0.9 * max(0.0, dot(sun, surface_normal));
    let grid = input.uv * f32(1u << input.tile.z);
    let edge = min(fract(grid), 1.0 - fract(grid)) / max(fwidth(grid), vec2(1e-7));
    let seam = min(input.uv, 1.0 - input.uv) / max(fwidth(input.uv), vec2(1e-7));
    if earth && (view.settings.y & 1u) == 0u {
        let uv = earth_uv(radial);
        let water = earth_water(uv);
        let base = earth_relief(earth_point(radial)) + water.offset;
        let p = local_position(input.sphere_relative);
        var local = LocalRelief(0.0, vec3(0.0), 0.0);
        if !water_pass {
            let bands = 1.0 - smoothstep(vec2(8.0, 0.60), vec2(26.0, 2.8), vec2(footprint));
            local = local_relief(uv, p, radial, dry_weight(water), bands);
            let material = earth_ground_material(p, radial, footprint);
            surface_normal = earth_normal(radial, footprint, local.grade + material.grade);
            local.material += material.material;
            illumination = 0.38 + 0.9 * max(0.0, dot(sun, surface_normal));
        }
        let exact = earth_surface(base + local.height, water);
        let edge_width = clamp(fwidth(exact.shore) * 0.5, 0.005, 0.3);
        var coverage = smoothstep(-edge_width, edge_width, exact.shore);
        if exact.wet_kind == 4.0 {
            coverage *= 1.0 - smoothstep(700.0, 3200.0, footprint);
        }
        let depth = exact.level - exact.height;
        let water_light = 0.38 + 0.9 * max(0.0, dot(sun, radial));
        let water_rgb = linear(color_water(max(0.0, depth), exact.wet_kind)) * water_light;
        if water_pass {
            if coverage < 0.995 { discard; }
            return vec4(water_rgb, 1.0);
        }
        let slope = length(cross(surface_normal, radial));
        let land = earth_land(exact.height, slope, macro_fields(face, input.uv)) * (1.0 + local.material);
        let bed = mix(vec3(0.52, 0.48, 0.37), vec3(0.12, 0.16, 0.19), smoothstep(0.0, 2400.0, depth));
        let camera_above = -dot(input.relative, radial) > depth;
        let surface_rgb = select(linear(bed) * illumination, water_rgb, camera_above);
        return vec4(mix(linear(land) * illumination, surface_rgb, coverage), 1.0);
    }
    let vertex_water = input.water;
    var depth = vertex_water.x;
    var kind = select(0.0, vertex_water.y, depth > 0.0);
    // Resolve narrow channels, lake boundaries and sea-level crossings at the pixel.
    // Skip further surface sampling for the dry interior and deep open ocean.
    let cell = f32(view.settings.z << input.tile.z);
    let pixel_step = max(max(fwidth(input.uv.x), fwidth(input.uv.y)) * cell, 0.001);
    let relief = fwidth(input.height) / pixel_step;
    if !earth && abs(input.height) < max(20.0, 2.0 * relief) {
        let exact = sample_surface(face, input.uv, radial);
        depth = exact.level - exact.height;
        kind = exact.kind;
    }
    if water_pass && kind == 0.0 { discard; }
    var rgb: vec3<f32>;
    if (view.settings.y & 1u) != 0u {
        let colors = array(
            vec3(0.28,0.48,0.62), vec3(0.32,0.56,0.50), vec3(0.58,0.62,0.68),
            vec3(0.54,0.58,0.64), vec3(0.34,0.52,0.42), vec3(0.46,0.48,0.36));
        rgb = mix(vec3(0.8), colors[face], smoothstep(0.4, 1.8, min(edge.x, edge.y)));
        rgb = mix(vec3(0.95, 0.78, 0.3), rgb, smoothstep(0.5, 2.0, min(seam.x, seam.y)));
    } else if kind > 0.0 {
        // The terrain pass also colors shallow features resolved below the geometry grid.
        // When viewing from below the water surface keep the visible bed.
        let camera_above = -dot(input.relative, radial) > depth;
        if water_pass || camera_above {
            let water_light = 0.38 + 0.9 * max(0.0, dot(sun, radial));
            return vec4(linear(color_water(depth, kind)) * water_light, 1.0);
        }
        rgb = mix(vec3(0.52, 0.48, 0.37), vec3(0.12, 0.16, 0.19), smoothstep(0.0, 2400.0, depth));
    } else {
        let slope = length(cross(surface_normal, radial));
        rgb = color_land(input.height, slope, macro_fields(face, input.uv))
            * land_grain(input.relative, footprint);
    }
    return vec4(linear(rgb) * illumination, 1.0);
}
