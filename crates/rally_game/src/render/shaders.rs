//! GLSL ES 1.00 shader sources (desktop GL + GLES2 compatible).
//!
//! World shader conventions:
//! * vertex colour alpha < 255 in OPAQUE meshes = wind-sway weight (see `mesh::sway`)
//! * alpha output depends on `params3.y` (out mode): 0 = opaque to screen (1.0),
//!   1 = opaque to the HDR-ish scene target (alpha = bloom/emissive mask), 2 = blended (real alpha)

/// Precision header for fragment shaders: highp where available (needed for shadow depth).
macro_rules! frag_header {
    () => {
        "#version 100\n#ifdef GL_FRAGMENT_PRECISION_HIGH\nprecision highp float;\n#else\nprecision mediump float;\n#endif\n"
    };
}

/// Shared wind function (vertex shaders)
macro_rules! wind_fn {
    () => {
        r#"
vec3 wind_offset(vec3 w, float weight, float t) {
    float ph = dot(w.xz, vec2(0.13, 0.09));
    float gust = 0.65 + 0.35 * sin(t * 0.45 + w.x * 0.011 + w.z * 0.007);
    float a = sin(t * 1.9 + ph) + 0.35 * sin(t * 4.3 + ph * 2.7);
    float b = cos(t * 1.5 + ph * 1.3) * 0.6 + 0.25 * sin(t * 3.7 + ph * 1.9);
    return vec3(a * 0.9 + 0.5, -abs(a) * 0.15, b) * weight * gust * 0.11;
}
"#
    };
}

pub const WORLD_VS: &str = concat!(
    "#version 100\n",
    r#"
attribute vec3 in_pos;
attribute vec3 in_normal;
attribute vec4 in_color;
uniform mat4 view_proj;
uniform mat4 model;
uniform mat4 shadow_mat;
uniform vec4 params;   // x fog density, y unlit, z time, w rim
uniform vec4 params2;  // x gloss, y sway enabled, z shadow strength, w ao top (local y, 0 = off)
uniform vec4 params3;  // x depth bias, y out mode, z shadow texel (uv), w ao bottom (local y)
uniform vec4 params4;  // x shadow texel (world m), y shadow depth bias
varying vec4 v_color;
varying vec3 v_normal;
varying vec3 v_world;
varying vec3 v_shadow;
varying float v_ao;
"#,
    wind_fn!(),
    r#"
void main() {
    vec4 c = in_color / 255.0;
    vec4 w = model * vec4(in_pos, 1.0);
    vec3 n = normalize((model * vec4(in_normal, 0.0)).xyz);
    if (params2.y > 0.5) {
        if (in_color.a < 254.5) {
            float weight = (255.0 - in_color.a) / 200.0;
            w.xyz += wind_offset(w.xyz, weight, params.z);
            // foliage: bend normals towards the sky so thin blades/leaves light softly
            n = normalize(mix(n, vec3(0.0, 1.0, 0.0), 0.35 + 0.45 * weight));
        }
        c.a = 1.0;
    }
    v_world = w.xyz;
    v_normal = n;
    v_color = c;
    v_ao = 1.0;
    if (params2.w != 0.0) {
        float t = clamp((in_pos.y - params3.w) / (params2.w - params3.w), 0.0, 1.0);
        v_ao = mix(0.62, 1.0, t * (2.0 - t));
    }
    if (params2.z > 0.0) {
        // normal-offset to fight acne, scaled by shadow texel size
        vec3 off = n * params4.x * 2.0;
        vec4 s = shadow_mat * vec4(w.xyz + off, 1.0);
        v_shadow = s.xyz * 0.5 + 0.5;
    } else {
        v_shadow = vec3(0.0);
    }
    gl_Position = view_proj * w;
    gl_Position.z -= params3.x * gl_Position.w;
}
"#
);

pub const WORLD_FS: &str = concat!(
    frag_header!(),
    r#"
varying vec4 v_color;
varying vec3 v_normal;
varying vec3 v_world;
varying vec3 v_shadow;
varying float v_ao;
uniform sampler2D shadow_map;
uniform vec3 sun_dir;
uniform vec3 sun_color;
uniform vec3 sky_amb;
uniform vec3 ground_amb;
uniform vec3 fog_color;
uniform vec3 fog_sun;
uniform vec3 cam_pos;
uniform vec4 tint;
uniform vec4 params;
uniform vec4 params2;
uniform vec4 params3;
uniform vec4 params4;
uniform vec4 params5; // x snow cover, y wetness, z lightning flash, w overcast
uniform vec4 params6; // x light count, y night
uniform vec4 light_pos[10]; // xyz position, w range
uniform vec4 light_dir[10]; // xyz direction, w cos outer
uniform vec4 light_col[10]; // rgb colour * intensity, w cos inner

float unpack_depth(vec4 c) {
    return dot(c, vec4(1.0, 1.0 / 255.0, 1.0 / 65025.0, 1.0 / 16581375.0));
}

float shadow_tap(vec2 uv, float z) {
    return step(z, unpack_depth(texture2D(shadow_map, uv)));
}

// 4x4 texel box filter evaluated with bilinear weights from a 5x5 tap grid: soft, stable edges
float shadow_pcf(vec3 sc, float slope) {
    float texel = params3.z;
    float size = 1.0 / texel;
    vec2 t = sc.xy * size - 0.5;
    vec2 f = fract(t);
    vec2 base = (floor(t) + 0.5) * texel;
    float z = sc.z - params4.y * slope;
    float sum = 0.0;
    for (int j = 0; j < 5; j++) {
        float wy = j == 0 ? 1.0 - f.y : (j == 4 ? f.y : 1.0);
        for (int i = 0; i < 5; i++) {
            float wx = i == 0 ? 1.0 - f.x : (i == 4 ? f.x : 1.0);
            sum += wx * wy * shadow_tap(base + vec2(float(i) - 2.0, float(j) - 2.0) * texel, z);
        }
    }
    return sum / 16.0;
}

// 2x2 bilinear-weighted compare (4 taps): cheap filter for distant fragments
float shadow_bilinear(vec3 sc, float slope) {
    float texel = params3.z;
    vec2 t = sc.xy / texel - 0.5;
    vec2 f = fract(t);
    vec2 base = (floor(t) + 0.5) * texel;
    float z = sc.z - params4.y * slope;
    float a = shadow_tap(base, z);
    float b = shadow_tap(base + vec2(texel, 0.0), z);
    float c = shadow_tap(base + vec2(0.0, texel), z);
    float e = shadow_tap(base + vec2(texel, texel), z);
    return mix(mix(a, b, f.x), mix(c, e, f.x), f.y);
}

vec3 soft_clip(vec3 c) {
    // gentle highlight shoulder so bright surfaces (snow, sand) keep their shading
    vec3 x = max(c - 0.78, 0.0);
    return min(c, 0.78) + x / (1.0 + x * 4.5);
}

void main() {
    vec3 n = normalize(v_normal);
    vec3 base = v_color.rgb * tint.rgb;
    bool road = params4.z > 0.5;
    float wet_gloss = 0.0;
    if (params.y < 0.5) {
        // fresh snow settles on upward faces (roads get a thinner, packed layer)
        if (params5.x > 0.0) {
            float up = smoothstep(0.45, 0.92, v_normal.y);
            base = mix(base, vec3(0.94, 0.96, 1.0), params5.x * up * (road ? 0.45 : 0.85));
        }
        // rain: everything darker and a little shiny (roads carry baked wetness/puddles)
        if (params5.y > 0.0 && !road) {
            base *= 1.0 - 0.2 * params5.y;
            wet_gloss = params5.y * 0.3 * smoothstep(0.3, 0.9, v_normal.y);
        }
    }
    vec3 to_cam = cam_pos - v_world;
    float dist = length(to_cam);
    vec3 view = to_cam / max(dist, 1e-4);
    // two-sided lighting for thin geometry, but keep authored normals (grass = up) intact
    if (!gl_FrontFacing && dot(n, view) < 0.0) { n = -n; }
    float d = dot(n, sun_dir);
    float wrap = clamp((d + 0.25) / 1.25, 0.0, 1.0);
    wrap = mix(wrap, smoothstep(0.15, 0.6, wrap), 0.35);
    float sh = 1.0;
    if (params2.z > 0.0 && wrap > 0.0) {
        vec3 sc = v_shadow;
        vec2 edge = min(sc.xy, 1.0 - sc.xy);
        float inside = clamp(min(edge.x, edge.y) * 14.0, 0.0, 1.0);
        if (inside > 0.0 && sc.z < 1.0) {
            float slope = 1.0 + 3.0 * (1.0 - clamp(d, 0.0, 1.0));
            // full soft filter up close; far away a 2x2 bilinear tap is indistinguishable
            float s;
            if (dist < 45.0) {
                s = shadow_pcf(sc, slope);
            } else {
                s = shadow_bilinear(sc, slope * 1.5);
            }
            sh = mix(1.0, s, inside * params2.z);
        }
    }
    // faces turned away from the sun count as shadowed so the terminator matches cast shadows
    float direct = wrap * sh;
    float hemi = n.y * 0.5 + 0.5;
    vec3 amb = mix(ground_amb, sky_amb, hemi) * mix(0.7, 1.12, hemi) * v_ao;
    // shadowed areas pick up a gentle cool sky tint, never black
    amb *= mix(vec3(1.02, 1.04, 1.13), vec3(1.0), sh);
    // soft sky fill from the side opposite the sun lifts self-shadowed faces
    vec3 fill_dir = normalize(vec3(-sun_dir.x, 0.35, -sun_dir.z));
    float fill = max(dot(n, fill_dir), 0.0) * 0.22;
    vec3 lit = base * (amb + sky_amb * fill * v_ao + sun_color * direct * 0.85 * mix(1.0, v_ao, 0.5));
    // specular glint (car paint, water-ish props)
    float spec = 0.0;
    float gloss = max(params2.x, wet_gloss);
    if (road) {
        gloss = (1.0 - v_color.a) * 1.25;
    }
    if (gloss > 0.0) {
        vec3 h = normalize(sun_dir + view);
        float nh = max(dot(n, h), 0.0);
        spec = pow(nh, 24.0 + gloss * 90.0) * gloss * sh * step(0.0, d);
        float fres = pow(1.0 - clamp(dot(n, view), 0.0, 1.0), 4.0);
        lit += sun_color * spec * 1.4 + sky_amb * fres * gloss * 0.35;
    }
    // headlights (night): soft-edged cones, smooth distance falloff, wrapped diffuse for the
    // low-poly faces and a glint on glossy paint and the rubbered / wet road
    if (params6.x > 0.5) {
        vec3 lamp = vec3(0.0);
        vec3 lamp_spec = vec3(0.0);
        for (int i = 0; i < 10; i++) {
            if (float(i) >= params6.x) { break; }
            vec3 L = light_pos[i].xyz - v_world;
            float ld = length(L);
            vec3 l = L / max(ld, 1e-3);
            float cone = smoothstep(light_dir[i].w, light_col[i].w, dot(-l, light_dir[i].xyz));
            float fall = clamp(1.0 - ld / light_pos[i].w, 0.0, 1.0);
            fall *= fall;
            // no blinding hot spot right in front of the bumper
            fall *= smoothstep(0.0, 3.0, ld);
            float ndl = clamp(dot(n, l) * 0.7 + 0.3, 0.0, 1.0);
            vec3 c = light_col[i].rgb * cone * fall;
            lamp += c * ndl;
            if (gloss > 0.0) {
                vec3 hl = normalize(l + view);
                lamp_spec += c * pow(max(dot(n, hl), 0.0), 20.0 + gloss * 80.0) * gloss;
            }
        }
        lit += base * lamp * v_ao + lamp_spec * 1.2;
    }
    float rim = pow(1.0 - clamp(dot(n, view), 0.0, 1.0), 3.0) * params.w;
    lit += rim * mix(sky_amb, sun_color, 0.6) * 0.3 * (0.5 + 0.5 * sh);
    lit = soft_clip(lit);
    vec3 col = mix(lit, base, params.y);
    // aerial perspective: fog warms up towards the sun
    float f = 1.0 - exp(-pow(dist * params.x, 1.5));
    if (params4.w > 0.0) { f = min(f, params4.w); }
    float toward_sun = pow(max(dot(-view, sun_dir), 0.0), 6.0);
    vec3 fogc = mix(fog_color, fog_sun, toward_sun);
    col = mix(col, fogc, clamp(f, 0.0, 1.0));
    float a = 1.0;
    if (params3.y > 1.5) {
        a = v_color.a * tint.a;
    } else if (params3.y > 0.5) {
        float emissive = params.y * clamp((max(tint.r, max(tint.g, tint.b)) - 1.0) * 2.5, 0.0, 1.0);
        a = clamp(emissive + max(spec - 0.15, 0.0) * 0.5, 0.0, 1.0) * (1.0 - f);
    }
    gl_FragColor = vec4(col, a);
}
"#
);

/// Shadow-map depth pass: packs ortho depth into RGBA8 (portable, no depth textures needed).
pub const SHADOW_VS: &str = concat!(
    "#version 100\n",
    r#"
attribute vec3 in_pos;
attribute vec3 in_normal;
attribute vec4 in_color;
uniform mat4 view_proj;
uniform mat4 model;
uniform vec4 params; // z time, y sway enabled
varying float v_depth;
"#,
    wind_fn!(),
    r#"
void main() {
    vec4 w = model * vec4(in_pos, 1.0);
    if (params.y > 0.5 && in_color.a < 254.5) {
        w.xyz += wind_offset(w.xyz, (255.0 - in_color.a) / 200.0, params.z);
    }
    gl_Position = view_proj * w;
    v_depth = gl_Position.z * 0.5 + 0.5;
}
"#
);

pub const SHADOW_FS: &str = concat!(
    frag_header!(),
    r#"
varying float v_depth;
void main() {
    vec4 enc = vec4(1.0, 255.0, 65025.0, 16581375.0) * clamp(v_depth, 0.0, 0.99999);
    enc = fract(enc);
    enc -= enc.yzww * vec4(1.0 / 255.0, 1.0 / 255.0, 1.0 / 255.0, 0.0);
    gl_FragColor = enc;
}
"#
);

/// Sky dome: per-pixel gradient, sun disc + glow, soft horizon haze.
pub const SKY_VS: &str = r#"#version 100
attribute vec3 in_pos;
attribute vec3 in_normal;
attribute vec4 in_color;
uniform mat4 view_proj;
uniform mat4 model;
varying vec3 v_dir;
void main() {
    v_dir = in_pos;
    vec4 p = view_proj * (model * vec4(in_pos, 1.0));
    gl_Position = p.xyww; // at the far plane
    gl_Position.z = p.w * 0.99999;
}
"#;

pub const SKY_FS: &str = concat!(
    frag_header!(),
    r#"
varying vec3 v_dir;
uniform vec3 sky_top;
uniform vec3 sky_horizon;
uniform vec3 fog_color;
uniform vec3 sun_dir;
uniform vec3 sun_color;
uniform vec4 params; // x out mode, y sun size, z glow strength, w time
uniform vec4 params2; // x overcast, y lightning flash, z precipitation, w night

float hash(vec2 p) { return fract(sin(dot(p, vec2(127.1, 311.7))) * 43758.5453); }
float vnoise(vec2 p) {
    vec2 i = floor(p);
    vec2 f = fract(p);
    f = f * f * (3.0 - 2.0 * f);
    return mix(mix(hash(i), hash(i + vec2(1.0, 0.0)), f.x), mix(hash(i + vec2(0.0, 1.0)), hash(i + vec2(1.0, 1.0)), f.x), f.y);
}

void main() {
    vec3 d = normalize(v_dir);
    float y = d.y;
    float t = pow(clamp(y, 0.0, 1.0), 0.55);
    vec3 col = mix(sky_horizon, sky_top, t);
    // warm band right at the horizon, cool haze below it
    col = mix(col, fog_color, smoothstep(0.06, -0.04, y));
    float s = max(dot(d, sun_dir), 0.0);
    float glow = pow(s, 10.0) * 0.16 + pow(s, 64.0) * 0.3;
    col += sun_color * glow * params.z * vec3(1.0, 0.92, 0.8);
    float night = params2.w;
    // the moon is a little bigger than the sun disc, pale and softly mottled
    float size = params.y * (1.0 + 0.6 * night);
    float disc = smoothstep(1.0 - size * 1.25, 1.0 - size, s) * params.z;
    vec3 disc_col = mix(vec3(1.0, 0.98, 0.9), vec3(0.90, 0.93, 1.0) * (0.86 + 0.14 * vnoise(d.xz * 700.0 + d.y * 300.0)), night);
    if (night > 0.01 && y > 0.0) {
        // stars on a grid projected onto a dome; a few bright ones twinkle
        vec2 g = d.xz / (y + 0.3) * 60.0;
        vec2 cell = floor(g);
        float h = hash(cell);
        vec2 off = vec2(hash(cell + 1.7), hash(cell + 3.1)) - 0.5;
        float r = length(fract(g) - 0.5 - off * 0.6);
        float bright = clamp((h - 0.975) / 0.025, 0.0, 1.0);
        float tw = 0.65 + 0.35 * sin(params.w * (1.5 + h * 3.0) + h * 90.0);
        float star = step(0.975, h) * smoothstep(0.09 + 0.06 * bright, 0.0, r) * (0.35 + bright * 0.9) * tw;
        // a faint milky way band of dust and tiny stars
        float band = exp(-pow(dot(d, normalize(vec3(0.55, 0.25, -0.8))) * 3.2, 2.0));
        float dust = vnoise(d.xz / (y + 0.3) * 9.0) * 0.6 + vnoise(d.xz / (y + 0.3) * 23.0) * 0.4;
        float fade = smoothstep(0.02, 0.28, y) * (1.0 - min(1.0, params2.x * 1.3));
        col += (vec3(0.85, 0.9, 1.0) * star + vec3(0.10, 0.11, 0.18) * band * dust) * fade * night;
    }
    col = mix(col, disc_col, disc);
    // overcast / rain cloud deck: soft billows on a plane above, drifting with time
    float oc = params2.x;
    if (oc > 0.01 && y > -0.05) {
        vec2 uv = d.xz / (max(y, 0.0) + 0.09) * 0.55 + vec2(params.w * 0.012, params.w * 0.005);
        float n = vnoise(uv) * 0.5 + vnoise(uv * 2.1 + 3.7) * 0.28 + vnoise(uv * 4.3 - 1.3) * 0.14 + vnoise(uv * 8.9) * 0.08;
        float th = mix(0.78, 0.12, oc);
        float cover = smoothstep(th - 0.12, th + 0.22, n) * smoothstep(-0.05, 0.12, y);
        // lit tops, heavy grey bellies when it pours
        vec3 light = mix(vec3(0.93, 0.93, 0.94), vec3(0.66, 0.68, 0.72), params2.z);
        vec3 dark = mix(vec3(0.66, 0.68, 0.73), vec3(0.34, 0.36, 0.41), params2.z);
        vec3 cloud = mix(dark, light, smoothstep(0.35, 0.95, n));
        cloud = mix(cloud, sky_horizon, (1.0 - smoothstep(0.0, 0.3, y)) * 0.65);
        cloud *= mix(1.0, 0.09, night);
        cloud += vec3(0.55, 0.57, 0.68) * params2.y * (0.4 + 0.6 * n);
        col = mix(col, cloud, cover * min(1.0, 0.35 + oc));
    }
    col += vec3(0.25, 0.26, 0.32) * params2.y;
    float a = 1.0;
    if (params.x > 0.5) {
        a = clamp(disc + pow(s, 24.0) * 0.55, 0.0, 1.0);
    }
    gl_FragColor = vec4(col, a);
}
"#
);

/// Opaque stylised water: vertex waves, fresnel sky tint, depth colour, shoreline foam, sparkles.
/// Vertex colour alpha = water depth (0..4 m mapped to 0..255).
pub const WATER_VS: &str = r#"#version 100
attribute vec3 in_pos;
attribute vec3 in_normal;
attribute vec4 in_color;
uniform mat4 view_proj;
uniform vec4 params; // x fog, y out mode, z time, w wave amp
varying vec4 v_color;
varying vec3 v_world;
varying vec3 v_normal;
void main() {
    vec3 p = in_pos;
    float t = params.z;
    float depth = in_color.a / 255.0 * 4.0;
    float amp = params.w * clamp(depth * 0.8, 0.15, 1.0);
    float w1 = sin(p.x * 0.21 + t * 1.3) * cos(p.z * 0.17 + t * 0.9);
    float w2 = sin((p.x + p.z) * 0.37 - t * 1.9) * 0.5;
    p.y += (w1 + w2) * amp;
    // analytic normal
    float dx = 0.21 * cos(p.x * 0.21 + t * 1.3) * cos(p.z * 0.17 + t * 0.9) + 0.37 * 0.5 * cos((p.x + p.z) * 0.37 - t * 1.9);
    float dz = -0.17 * sin(p.x * 0.21 + t * 1.3) * sin(p.z * 0.17 + t * 0.9) + 0.37 * 0.5 * cos((p.x + p.z) * 0.37 - t * 1.9);
    v_normal = normalize(vec3(-dx * amp, 1.0, -dz * amp));
    v_world = p;
    v_color = in_color / 255.0;
    gl_Position = view_proj * vec4(p, 1.0);
}
"#;

pub const WATER_FS: &str = concat!(
    frag_header!(),
    r#"
varying vec4 v_color;
varying vec3 v_world;
varying vec3 v_normal;
uniform vec3 sun_dir;
uniform vec3 sun_color;
uniform vec3 sky_color;
uniform vec3 fog_color;
uniform vec3 fog_sun;
uniform vec3 cam_pos;
uniform vec3 shallow;
uniform vec4 params;

float hash(vec2 p) { return fract(sin(dot(p, vec2(127.1, 311.7))) * 43758.5453); }
float vnoise(vec2 p) {
    vec2 i = floor(p);
    vec2 f = fract(p);
    f = f * f * (3.0 - 2.0 * f);
    return mix(mix(hash(i), hash(i + vec2(1.0, 0.0)), f.x), mix(hash(i + vec2(0.0, 1.0)), hash(i + vec2(1.0, 1.0)), f.x), f.y);
}

void main() {
    float t = params.z;
    float depth = v_color.a * 4.0;
    // small ripples perturb the normal
    vec2 q = v_world.xz;
    float r1 = vnoise(q * 0.55 + vec2(t * 0.35, t * 0.21));
    float r2 = vnoise(q * 1.3 - vec2(t * 0.27, -t * 0.4));
    vec3 n = normalize(v_normal + vec3(r1 - 0.5, 0.0, r2 - 0.5) * 0.22);
    vec3 to_cam = cam_pos - v_world;
    float dist = length(to_cam);
    vec3 v = to_cam / dist;
    float ndv = clamp(dot(n, v), 0.0, 1.0);
    float fres = 0.08 + 0.72 * pow(1.0 - ndv, 4.0);
    vec3 deep = v_color.rgb;
    vec3 body = mix(shallow, deep, smoothstep(0.0, 2.6, depth));
    vec3 lit = body * (0.62 + 0.45 * max(dot(n, sun_dir), 0.0) * sun_color);
    vec3 col = mix(lit, sky_color, fres);
    // sun sparkle
    vec3 h = normalize(sun_dir + v);
    float nh = max(dot(n, h), 0.0);
    float glint = pow(nh, 220.0) * 2.2;
    float sparkle = step(0.82, vnoise(q * 3.1 + t * 1.7)) * pow(nh, 30.0) * 0.9;
    float spec = glint + sparkle;
    col += sun_color * spec;
    // shoreline foam: animated band where the water gets shallow
    float band = 0.25 + 0.12 * sin(t * 1.4 + vnoise(q * 0.4) * 6.0);
    float foam = smoothstep(band + 0.25, band, depth) * smoothstep(0.35, 0.65, vnoise(q * 1.7 + vec2(t * 0.2, 0.0)) + 0.35);
    foam = max(foam, smoothstep(0.12, 0.0, depth));
    col = mix(col, vec3(0.97, 0.98, 1.0), foam * 0.85);
    float f = 1.0 - exp(-pow(dist * params.x, 1.5));
    float toward_sun = pow(max(dot(-v, sun_dir), 0.0), 6.0);
    col = mix(col, mix(fog_color, fog_sun, toward_sun), clamp(f, 0.0, 1.0));
    float a = 1.0;
    if (params.y > 0.5) {
        a = clamp(spec * 0.9, 0.0, 1.0) * (1.0 - f);
    }
    gl_FragColor = vec4(min(col, vec3(1.0)), a);
}
"#
);

// ---------------------------------------------------------------- post-processing

pub const POST_VS: &str = r#"#version 100
attribute vec2 in_pos;
uniform vec4 rect; // source uv rect (x, y, w, h)
varying vec2 v_uv;
varying vec2 v_local;
varying vec2 v_radial;
void main() {
    v_local = in_pos;
    v_radial = (in_pos - 0.5) * rect.zw;
    v_uv = rect.xy + in_pos * rect.zw;
    gl_Position = vec4(in_pos * 2.0 - 1.0, 0.0, 1.0);
}
"#;

/// Bright pass + 4-tap box downsample: bloom source = colour * mask (alpha), plus a soft luma knee.
pub const BRIGHT_FS: &str = concat!(
    frag_header!(),
    r#"
varying vec2 v_uv;
varying vec2 v_local;
uniform sampler2D tex;
uniform vec4 texel; // xy = source texel size, z = luma threshold, w = luma knee strength
vec3 pick(vec2 uv) {
    vec4 c = texture2D(tex, uv);
    float l = dot(c.rgb, vec3(0.299, 0.587, 0.114));
    float k = clamp((l - texel.z) / (1.0 - texel.z), 0.0, 1.0);
    return c.rgb * max(c.a, k * k * texel.w);
}
void main() {
    vec2 o = texel.xy;
    vec3 c = pick(v_uv + vec2(-o.x, -o.y)) + pick(v_uv + vec2(o.x, -o.y)) + pick(v_uv + vec2(-o.x, o.y)) + pick(v_uv + vec2(o.x, o.y));
    gl_FragColor = vec4(c * 0.25, 1.0);
}
"#
);

/// Plain 4-tap box downsample
pub const DOWN_FS: &str = concat!(
    frag_header!(),
    r#"
varying vec2 v_uv;
varying vec2 v_local;
uniform sampler2D tex;
uniform vec4 texel;
void main() {
    vec2 o = texel.xy;
    vec3 c = texture2D(tex, v_uv + vec2(-o.x, -o.y)).rgb + texture2D(tex, v_uv + vec2(o.x, -o.y)).rgb + texture2D(tex, v_uv + vec2(-o.x, o.y)).rgb + texture2D(tex, v_uv + vec2(o.x, o.y)).rgb;
    gl_FragColor = vec4(c * 0.25, 1.0);
}
"#
);

/// 9-tap gaussian using linear filtering (5 fetches). texel.xy = direction * texel size
pub const BLUR_FS: &str = concat!(
    frag_header!(),
    r#"
varying vec2 v_uv;
varying vec2 v_local;
uniform sampler2D tex;
uniform vec4 texel;
void main() {
    vec2 d = texel.xy;
    vec3 c = texture2D(tex, v_uv).rgb * 0.2270270270;
    c += texture2D(tex, v_uv + d * 1.3846153846).rgb * 0.3162162162;
    c += texture2D(tex, v_uv - d * 1.3846153846).rgb * 0.3162162162;
    c += texture2D(tex, v_uv + d * 3.2307692308).rgb * 0.0702702703;
    c += texture2D(tex, v_uv - d * 3.2307692308).rgb * 0.0702702703;
    gl_FragColor = vec4(c, 1.0);
}
"#
);

/// Final composite: optional FXAA, bloom, cozy colour grading, vignette.
pub const COMPOSITE_FS: &str = concat!(
    frag_header!(),
    r#"
varying vec2 v_uv;
varying vec2 v_local;
uniform sampler2D scene;
uniform sampler2D bloom1;
uniform sampler2D bloom2;
uniform vec4 texel;   // xy scene texel size, z fxaa on, w bloom strength
uniform vec3 lift;    // shadow tint
uniform vec3 gain;    // highlight tint
uniform vec4 grade;   // x saturation, y contrast, z vignette, w warmth
uniform vec4 extra;   // x aspect (w/h), y softness, z time, w exposure
uniform vec4 motion;  // x speed streak amount (0..1)
varying vec2 v_radial;

float luma(vec3 c) { return dot(c, vec3(0.299, 0.587, 0.114)); }

vec3 fxaa(vec2 uv) {
    vec2 px = texel.xy;
    vec3 rgbNW = texture2D(scene, uv + vec2(-1.0, -1.0) * px).rgb;
    vec3 rgbNE = texture2D(scene, uv + vec2(1.0, -1.0) * px).rgb;
    vec3 rgbSW = texture2D(scene, uv + vec2(-1.0, 1.0) * px).rgb;
    vec3 rgbSE = texture2D(scene, uv + vec2(1.0, 1.0) * px).rgb;
    vec3 rgbM = texture2D(scene, uv).rgb;
    float lNW = luma(rgbNW);
    float lNE = luma(rgbNE);
    float lSW = luma(rgbSW);
    float lSE = luma(rgbSE);
    float lM = luma(rgbM);
    float lMin = min(lM, min(min(lNW, lNE), min(lSW, lSE)));
    float lMax = max(lM, max(max(lNW, lNE), max(lSW, lSE)));
    if (lMax - lMin < max(0.04, lMax * 0.1)) {
        return rgbM;
    }
    vec2 dir = vec2(-((lNW + lNE) - (lSW + lSE)), ((lNW + lSW) - (lNE + lSE)));
    float reduce = max((lNW + lNE + lSW + lSE) * 0.03125, 1.0 / 128.0);
    float rcp = 1.0 / (min(abs(dir.x), abs(dir.y)) + reduce);
    dir = clamp(dir * rcp, -8.0, 8.0) * px;
    vec3 a = 0.5 * (texture2D(scene, uv + dir * (1.0 / 3.0 - 0.5)).rgb + texture2D(scene, uv + dir * (2.0 / 3.0 - 0.5)).rgb);
    vec3 b = a * 0.5 + 0.25 * (texture2D(scene, uv - dir * 0.5).rgb + texture2D(scene, uv + dir * 0.5).rgb);
    float lB = luma(b);
    if (lB < lMin || lB > lMax) {
        return a;
    }
    return b;
}

void main() {
    vec3 c;
    if (texel.z > 0.5) {
        c = fxaa(v_uv);
    } else {
        c = texture2D(scene, v_uv).rgb;
    }
    // speed streaks: gentle radial blur towards the edges at very high speed
    if (motion.x > 0.001) {
        vec2 p = v_local - 0.5;
        float edge = smoothstep(0.24, 0.7, length(p * vec2(extra.x, 1.0) / max(extra.x, 1.0) * 1.6)) * motion.x;
        if (edge > 0.002) {
            vec3 acc = c;
            for (int k = 1; k <= 6; k++) {
                acc += texture2D(scene, v_uv - v_radial * (0.0075 * float(k) * edge)).rgb;
            }
            c = mix(c, acc / 7.0, min(1.0, edge * 1.3));
        }
    }
    // bloom: a tight glow plus a wide, soft halation for a dreamy film-like feel
    vec3 b1 = texture2D(bloom1, v_uv).rgb;
    vec3 b2 = texture2D(bloom2, v_uv).rgb;
    c += (b1 * 0.7 + b2 * (0.6 + extra.y)) * texel.w;
    c *= extra.w;
    // soft shoulder so bloom never clips harshly
    vec3 x = max(c - 0.9, 0.0);
    c = min(c, 0.9) + x / (1.0 + x * 6.0);
    // colour grading: lift/gain split toning, warmth, saturation, gentle contrast
    float l = luma(c);
    c = c * mix(lift, gain, smoothstep(0.05, 0.95, l));
    c += vec3(0.02, 0.006, -0.018) * grade.w;
    l = luma(c);
    c = mix(vec3(l), c, grade.x);
    c = (c - 0.5) * grade.y + 0.5;
    // vignette per viewport, tinted warm-lavender rather than black
    vec2 p = v_local - 0.5;
    float vig = clamp(1.0 - dot(p, p) * grade.z * 2.0, 0.0, 1.0);
    c *= mix(vec3(0.78, 0.7, 0.76), vec3(1.0), vig);
    gl_FragColor = vec4(clamp(c, 0.0, 1.0), 1.0);
}
"#
);
