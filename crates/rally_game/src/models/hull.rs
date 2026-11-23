//! Continuous car hull: one lofted surface from nose to tail whose cross-section includes the
//! greenhouse, so bonnet → windscreen → roof → rear deck flow as a single skin. Wheel arches are
//! cut into the sill line (semicircular openings) and the flanks flare out smoothly around them.
//! Glass quads go to a separate translucent mesh. Query helpers (`point`, `top_y`, `side_x`)
//! let details attach exactly to the surface.
use super::parts::MeshExt;
use crate::render::mesh::{rgb, rgba, shade, MeshBuilder};
use macroquad::math::Vec3;

/// Lengthwise key section of the lower body
#[derive(Clone, Copy, Debug)]
pub struct Key {
    pub z: f32,
    /// sill / underside height
    pub yb: f32,
    /// shoulder (belt / bonnet edge) height
    pub yt: f32,
    /// half width of the flank
    pub w: f32,
    /// shoulder rounding radius
    pub sh: f32,
    /// extra height of the top centre (sculpted bonnet / deck)
    pub crown: f32,
}

pub const fn key(z: f32, yb: f32, yt: f32, w: f32, sh: f32, crown: f32) -> Key {
    Key { z, yb, yt, w, sh, crown }
}

/// Greenhouse: windscreen base z0 → roof z1..z2 → rear window base z3
#[derive(Clone, Copy, Debug)]
pub struct Cabin {
    pub z0: f32,
    pub z1: f32,
    pub z2: f32,
    pub z3: f32,
    pub yr: f32,
    pub wr: f32,
    /// roof-rail rounding
    pub rail: f32,
    pub crown: f32,
    /// side windows run between these z (pillars outside)
    pub win: (f32, f32),
    /// optional B pillar z
    pub b_pillar: Option<f32>,
    /// rear "window" is a solid painted deck (mid/rear-engine cars)
    pub rear_solid: bool,
    /// cockpit opening (open cars): top quads over the cabin are left open
    pub open: bool,
}

/// Wheel arch: opening radius `r` around (z, yc), flank flares out by `flare`
#[derive(Clone, Copy, Debug)]
pub struct Arch {
    pub z: f32,
    pub yc: f32,
    pub r: f32,
    pub flare: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct Livery {
    pub paint: [f32; 3],
    /// sill / lower chamfer colour
    pub lower: [f32; 3],
    /// colour of the lower flank band (kind 2), None = paint
    pub side_band: Option<[f32; 3]>,
    /// colour of the top-centre band over bonnet/roof/deck (kind 7), None = paint
    pub top_band: Option<[f32; 3]>,
    pub roof: [f32; 3],
    /// pillar / window-frame colour
    pub pillar: [f32; 3],
}

pub struct Hull {
    pub keys: Vec<Key>,
    pub cabin: Option<Cabin>,
    pub arches: Vec<Arch>,
    pub nose_r: f32,
    pub tail_r: f32,
}

/// Evaluated cross-section parameters at z
#[derive(Clone, Copy, Debug)]
pub struct Sec {
    pub z: f32,
    /// sill height before the arch cut
    pub yb: f32,
    pub ybot: f32,
    pub yt: f32,
    /// smooth fender rise over the wheel arches (added to the shoulder line)
    pub hump: f32,
    pub w: f32,
    pub sh: f32,
    pub crown: f32,
    pub flare: f32,
    pub yr: f32,
    pub wr: f32,
    pub h: f32,
    pub rail: f32,
    pub rcrown: f32,
    /// shrink applied at the rounded nose/tail
    pub shrink: f32,
}

const HALF: usize = 9;
pub const RING: usize = 2 * HALF - 2;

fn cr(p0: f32, p1: f32, p2: f32, p3: f32, t: f32) -> f32 {
    let t2 = t * t;
    let t3 = t2 * t;
    0.5 * ((2.0 * p1) + (-p0 + p2) * t + (2.0 * p0 - 5.0 * p1 + 4.0 * p2 - p3) * t2 + (-p0 + 3.0 * p1 - 3.0 * p2 + p3) * t3)
}

/// Split a (possibly non-planar) quad along the diagonal that folds it least, so facets
/// stay consistent instead of zig-zagging.
fn flat_quad(m: &mut MeshBuilder, a: Vec3, b: Vec3, c: Vec3, d: Vec3, col: [u8; 4]) {
    let n = |p: Vec3, q: Vec3, r: Vec3| (q - p).cross(r - p).normalize_or_zero();
    let fold_ac = n(a, b, c).dot(n(a, c, d));
    let fold_bd = n(a, b, d).dot(n(b, c, d));
    // keep one consistent diagonal unless the other folds clearly less
    if fold_ac >= fold_bd - 0.02 {
        m.tri(a, b, c, col);
        m.tri(a, c, d, col);
    } else {
        m.tri(a, b, d, col);
        m.tri(b, c, d, col);
    }
}

fn ease(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

impl Hull {
    pub fn front(&self) -> f32 {
        self.keys[0].z - self.nose_r
    }
    pub fn rear(&self) -> f32 {
        self.keys.last().unwrap().z + self.tail_r
    }

    /// Smooth (Catmull-Rom) interpolation of the key sections
    fn key_at(&self, z: f32) -> Key {
        let k = &self.keys;
        let n = k.len();
        if z <= k[0].z {
            return Key { z, ..k[0] };
        }
        if z >= k[n - 1].z {
            return Key { z, ..k[n - 1] };
        }
        let mut i = 0;
        while i + 1 < n && k[i + 1].z < z {
            i += 1;
        }
        let t = (z - k[i].z) / (k[i + 1].z - k[i].z).max(1e-5);
        let g = |j: isize| k[(j.clamp(0, n as isize - 1)) as usize];
        let (a, b, c, d) = (g(i as isize - 1), g(i as isize), g(i as isize + 1), g(i as isize + 2));
        let f = |sel: fn(&Key) -> f32| cr(sel(&a), sel(&b), sel(&c), sel(&d), t);
        Key { z, yb: f(|k| k.yb), yt: f(|k| k.yt), w: f(|k| k.w), sh: f(|k| k.sh).max(0.02), crown: f(|k| k.crown) }
    }

    pub fn sec(&self, z: f32) -> Sec {
        let (zf, zr) = (self.keys[0].z, self.keys.last().unwrap().z);
        let kz = z.clamp(zf, zr);
        let k = self.key_at(kz);
        // rounded nose / tail
        let shrink = if z < zf {
            let d = (zf - z) / self.nose_r.max(1e-3);
            self.nose_r * (1.0 - (1.0 - d.min(1.0) * d.min(1.0)).max(0.0).sqrt())
        } else if z > zr {
            let d = (z - zr) / self.tail_r.max(1e-3);
            self.tail_r * (1.0 - (1.0 - d.min(1.0) * d.min(1.0)).max(0.0).sqrt())
        } else {
            0.0
        };
        let mut ybot = k.yb;
        let mut flare = 0.0f32;
        let mut hump = 0.0f32;
        for a in &self.arches {
            let dz = (z - a.z).abs();
            if dz < a.r {
                ybot = ybot.max(a.yc + (a.r * a.r - dz * dz).sqrt() * 0.98 - 0.02);
            }
            // fender crown: a smooth bell along z, so the shoulder never follows the steep arch edge
            let t = dz / (a.r * 1.45);
            if t < 1.0 {
                let bell = (1.0 - t * t) * (1.0 - t * t);
                hump = hump.max((a.yc + a.r + 0.08 - k.yt).max(0.0) * bell);
            }
            let span = a.r * 1.75;
            flare = flare.max(a.flare * ease(1.0 - (dz / span).min(1.0)));
        }
        let mut s = Sec { z, yb: k.yb, ybot, yt: k.yt, hump, w: k.w, sh: k.sh, crown: k.crown, flare, yr: k.yt, wr: k.w - k.sh, h: 0.0, rail: 0.05, rcrown: 0.0, shrink };
        if let Some(c) = self.cabin {
            let t = if z <= c.z0 || z >= c.z3 {
                0.0
            } else if z < c.z1 {
                (z - c.z0) / (c.z1 - c.z0)
            } else if z <= c.z2 {
                1.0
            } else {
                (c.z3 - z) / (c.z3 - c.z2)
            };
            let t = (t * std::f32::consts::FRAC_PI_2).sin();
            let mid = if z > c.z1 && z < c.z2 { ((z - c.z1) / (c.z2 - c.z1) * std::f32::consts::PI).sin() * 0.02 } else { 0.0 };
            s.yr = k.yt + (c.yr - k.yt) * t + mid;
            s.wr = c.wr;
            s.h = t;
            s.rail = c.rail;
            s.rcrown = c.crown;
        }
        // apply the end rounding: pull everything in
        let d = s.shrink;
        if d > 0.0 {
            s.ybot += d * 0.45;
            s.yb += d * 0.45;
            s.yt -= d * 0.7;
            s.yr -= d * 0.7;
            s.w -= d;
            s.wr = (s.wr - d).max(0.02);
            s.flare *= 1.0 - (d / 0.2).min(1.0);
            s.sh = (s.sh - d * 0.4).max(0.02);
            s.crown *= 0.7;
        }
        s
    }

    /// 9 half-ring points (x >= 0) for a section: underside centre → top centre
    pub fn half(s: &Sec) -> [Vec3; HALF] {
        let z = s.z;
        // Heights are laid out from the unraised sill and only pushed upward (never reordered)
        // where the arch cut climbs past them: the section can't fold over itself, and the
        // fender rises smoothly with the arch instead of kinking.
        let ybot = s.ybot;
        // how far the arch cut has climbed into the flank (0 = not at all)
        let lift = ((ybot - s.yb) / (s.yt - s.yb).max(0.05)).clamp(0.0, 1.0);
        let cb = (0.07f32).min((s.yt - s.yb) * 0.3).max(0.03) * (1.0 - 0.6 * lift);
        let y2 = ybot + cb;
        let y3 = (s.yb + (s.yt - s.yb) * 0.55).max(y2 + 0.02);
        let y4 = (s.yt + s.hump - s.sh * 0.55).max(y3 + 0.02);
        let yt_side = (s.yt + s.hump).max(y4 + 0.025);
        let wl = s.w + s.flare;
        let wm = s.w + s.flare * 0.85;
        let ws = s.w + s.flare * 0.45;
        let ytop_c = s.yt + s.crown;
        let p = |x: f32, y: f32| Vec3::new(x.max(0.0), y, z);
        // the underside may rise over the arches but never through the bonnet centre
        let p0 = p(0.0, ybot.min(ytop_c - 0.06));
        let p1 = p(wl - cb, ybot);
        let p2 = p(wl, y2);
        let p3 = p(wm, y3);
        let p4 = p(ws - s.sh * 0.35, y4);
        let p5 = p(ws - s.sh, yt_side);
        // bonnet/deck top vs cabin top, blended by cabin factor h
        let bx = (ws - s.sh).max(0.05);
        let b6 = (bx * 0.62, yt_side + (ytop_c - yt_side) * 0.55);
        let b7 = (bx * 0.3, yt_side + (ytop_c - yt_side) * 0.9);
        let b8 = (0.0, ytop_c);
        let c6 = ((s.wr + s.rail).min(bx), s.yr - s.rail);
        let c7 = (s.wr * 0.45, s.yr + s.rcrown * 0.8);
        let c8 = (0.0, s.yr + s.rcrown);
        let h = s.h;
        // the glasshouse widens to the roof-rail line right at the cowl/deck so the pillars rise
        // from the belt corners (heights still follow h smoothly)
        let hx = (h / 0.12).min(1.0);
        let l = |a: (f32, f32), b: (f32, f32), fx: f32| p(a.0 + (b.0 - a.0) * fx, a.1 + (b.1 - a.1) * h);
        [p0, p1, p2, p3, p4, p5, l(b6, c6, hx), l(b7, c7, h), l(b8, c8, h)]
    }

    pub fn ring(s: &Sec) -> Vec<Vec3> {
        let hp = Self::half(s);
        let mut v: Vec<Vec3> = hp.to_vec();
        for k in (1..HALF - 1).rev() {
            let q = hp[k];
            v.push(Vec3::new(-q.x, q.y, q.z));
        }
        v
    }

    /// Sample positions along the length: keys subdivided, arches resolved, ends rounded.
    fn stations(&self) -> Vec<f32> {
        let mut zs: Vec<f32> = Vec::new();
        let f = self.front();
        let r = self.rear();
        for t in [1.0f32, 0.82, 0.55, 0.25] {
            zs.push(self.keys[0].z - self.nose_r * t);
        }
        for w in self.keys.windows(2) {
            for k in 0..3 {
                zs.push(w[0].z + (w[1].z - w[0].z) * k as f32 / 3.0);
            }
        }
        zs.push(self.keys.last().unwrap().z);
        for t in [0.25f32, 0.55, 0.82, 1.0] {
            zs.push(self.keys.last().unwrap().z + self.tail_r * t);
        }
        for a in &self.arches {
            for u in [-1.15f32, -1.0, -0.8, -0.5, -0.18, 0.18, 0.5, 0.8, 1.0, 1.15] {
                let z = a.z + u * a.r;
                if z > f && z < r {
                    zs.push(z);
                }
            }
        }
        if let Some(c) = self.cabin {
            for z in [c.z0, c.z0 + (c.z1 - c.z0) * 0.33, c.z0 + (c.z1 - c.z0) * 0.66, c.z1, c.z2, c.z2 + (c.z3 - c.z2) * 0.33, c.z2 + (c.z3 - c.z2) * 0.66, c.z3, c.win.0, c.win.1] {
                zs.push(z);
            }
            if let Some(b) = c.b_pillar {
                zs.push(b - 0.05);
                zs.push(b + 0.05);
            }
        }
        zs.sort_by(|a, b| a.partial_cmp(b).unwrap());
        zs.dedup_by(|a, b| (*a - *b).abs() < 0.045);
        zs
    }

    /// Build the hull: painted skin into `m`, glass into `g`.
    pub fn build(&self, m: &mut MeshBuilder, g: &mut MeshBuilder, liv: &Livery) {
        let zs = self.stations();
        let secs: Vec<Sec> = zs.iter().map(|&z| self.sec(z)).collect();
        let rings: Vec<Vec<Vec3>> = secs.iter().map(Self::ring).collect();
        let glass_c = |c: [f32; 3]| rgba(c, 0.42);
        const GLASS: [f32; 3] = [0.58, 0.77, 0.88];
        const GLINT: [f32; 3] = [0.86, 0.95, 0.99];
        for si in 0..rings.len() - 1 {
            let (a, b) = (&rings[si], &rings[si + 1]);
            let (sa, sb) = (&secs[si], &secs[si + 1]);
            let zc = (sa.z + sb.z) * 0.5;
            let hmin = sa.h.min(sb.h);
            let hmax = sa.h.max(sb.h);
            // region: 0 none, 1 windscreen, 2 roof, 3 rear window
            let region = match self.cabin {
                Some(c) if hmax > 0.02 => {
                    if zc < c.z1 {
                        1
                    } else if zc <= c.z2 {
                        2
                    } else {
                        3
                    }
                }
                _ => 0,
            };
            for k in 0..RING {
                let j = (k + 1) % RING;
                let kind = if k < HALF - 1 { k } else { RING - 1 - k };
                let (p0, p1, p2, p3) = (a[k], a[j], b[j], b[k]);
                // (glass?, colour)
                let (is_glass, col): (bool, [f32; 3]) = match (region, kind) {
                    (_, 0) => (false, [0.21, 0.2, 0.23]),
                    (_, 1) => (false, liv.lower),
                    (_, 2) => (false, liv.side_band.unwrap_or(liv.paint)),
                    (_, 3) | (_, 4) => (false, liv.paint),
                    (0, 5) | (0, 6) => (false, liv.paint),
                    (0, _) => (false, liv.top_band.unwrap_or(liv.paint)),
                    (1, 5) if hmin < 0.08 => (false, liv.pillar),
                    (1, 5) => (true, GLASS),
                    (_, 5) => {
                        let c = self.cabin.unwrap();
                        let in_win = zc > c.win.0 && zc < c.win.1 && hmin > 0.55;
                        let in_b = c.b_pillar.map_or(false, |bz| (zc - bz).abs() < 0.05);
                        if in_win && !in_b {
                            (true, GLASS)
                        } else {
                            (false, liv.pillar)
                        }
                    }
                    (1, _) => (true, if kind == 7 { GLINT } else { GLASS }),
                    (3, _) if !self.cabin.unwrap().rear_solid => (true, GLASS),
                    (3, 7) => (false, liv.top_band.unwrap_or(liv.roof)),
                    (3, _) => (false, liv.roof),
                    (2, 7) => (false, liv.top_band.unwrap_or(liv.roof)),
                    _ => (false, liv.roof),
                };
                let open_top = self.cabin.map_or(false, |c| c.open) && region != 0 && kind >= 6;
                if open_top {
                    continue;
                }
                let target: &mut MeshBuilder = if is_glass { &mut *g } else { &mut *m };
                let c = if is_glass { glass_c(col) } else { rgb(col) };
                if p0.distance_squared(p1) < 1e-9 {
                    target.tri(p0, p2, p3, c);
                } else if p2.distance_squared(p3) < 1e-9 {
                    target.tri(p0, p1, p2, c);
                } else {
                    flat_quad(target, p0, p1, p2, p3, c);
                }
            }
        }
        // caps at the nose and tail
        let cap = |m: &mut MeshBuilder, r: &Vec<Vec3>, flip: bool| {
            let cen = r.iter().copied().sum::<Vec3>() / r.len() as f32;
            for i in 0..r.len() {
                let (p, q) = (r[i], r[(i + 1) % r.len()]);
                if flip {
                    m.tri(cen, q, p, rgb(shade(liv.paint, 0.97)));
                } else {
                    m.tri(cen, p, q, rgb(shade(liv.paint, 0.97)));
                }
            }
        };
        // slim A pillars running along the windscreen edge (ring point 6), attached to the skin
        if let Some(c) = self.cabin {
            if !c.open {
                let pts: Vec<(Vec3, Vec3)> = zs
                    .iter()
                    .zip(secs.iter())
                    .filter(|(z, sc)| **z >= c.z0 && **z <= c.z1 + 1e-3 && sc.h > 0.1)
                    .map(|(_, sc)| {
                        let hp = Self::half(sc);
                        (hp[6], (hp[6] - hp[5]).cross(Vec3::Z).normalize_or_zero())
                    })
                    .collect();
                for sx in [-1.0f32, 1.0] {
                    for w in pts.windows(2) {
                        let f = |(p, n): (Vec3, Vec3)| {
                            let n = if n.y < 0.0 { -n } else { n };
                            Vec3::new(p.x * sx, p.y, p.z) + Vec3::new(n.x * sx, n.y, 0.0) * 0.012
                        };
                        m.beam(f(w[0]), f(w[1]), 0.05, rgb(liv.pillar));
                    }
                }
            }
        }
        cap(m, &rings[0], false);
        cap(m, rings.last().unwrap(), true);
        let _ = MeshExt::mark(m);
    }

    /// Point on the hull surface at z, along the half ring (u in 0..8, fractional), on side
    /// sx (+1 right / -1 left), with outward normal.
    pub fn point(&self, z: f32, sx: f32, u: f32) -> (Vec3, Vec3) {
        let s = self.sec(z);
        let hp = Self::half(&s);
        let u = u.clamp(0.0, (HALF - 1) as f32);
        let i = (u.floor() as usize).min(HALF - 2);
        let t = u - i as f32;
        let p = hp[i].lerp(hp[i + 1], t);
        let tan = (hp[i + 1] - hp[i]).normalize_or_zero();
        // outward normal in the section plane (rotate tangent by -90° about Z for the right half)
        let n = Vec3::new(tan.y, -tan.x, 0.0).normalize_or_zero();
        (Vec3::new(p.x * sx, p.y, p.z), Vec3::new(n.x * sx, n.y, n.z))
    }

    /// Like `point` but with the true 3D surface normal (accounts for the lengthwise slope at
    /// the nose and tail).
    pub fn point3(&self, z: f32, sx: f32, u: f32) -> (Vec3, Vec3) {
        let (p, n2) = self.point(z, sx, u);
        let (a, _) = self.point(z - 0.02, sx, u);
        let (b, _) = self.point(z + 0.02, sx, u);
        let (c, _) = self.point(z, sx, (u - 0.06).max(0.0));
        let (d, _) = self.point(z, sx, (u + 0.06).min((HALF - 1) as f32));
        let mut n = (d - c).cross(b - a).normalize_or_zero();
        if n.dot(n2) < 0.0 {
            n = -n;
        }
        if n == Vec3::ZERO {
            n = n2;
        }
        (p, n)
    }

    /// Thin strip following the surface at z from u0 to u1, lifted off the skin (door seams,
    /// stripes, channels)
    pub fn strip(&self, m: &mut MeshBuilder, z: f32, sx: f32, u0: f32, u1: f32, half_w: f32, lift: f32, col: [u8; 4]) {
        // facets between stations deviate from the smooth section by a few mm: stay clear
        let lift = lift.max(0.025);
        let steps = ((u1 - u0).abs() / 0.3).ceil().max(1.0) as usize;
        for k in 0..steps {
            let ua = u0 + (u1 - u0) * k as f32 / steps as f32;
            let ub = u0 + (u1 - u0) * (k + 1) as f32 / steps as f32;
            let (pa, na) = self.point3(z, sx, ua);
            let (pb, nb) = self.point3(z, sx, ub);
            let (pa, pb) = (pa + na * lift, pb + nb * lift);
            let d = Vec3::Z * half_w;
            m.quad(pa - d, pa + d, pb + d, pb - d, col);
        }
    }

    /// Strip running lengthwise at ring position u from z0 to z1 (aero channels, pinstripes)
    pub fn long_strip(&self, m: &mut MeshBuilder, sx: f32, u: f32, z0: f32, z1: f32, half_w: f32, lift: f32, col: [u8; 4]) {
        let lift = lift.max(0.025);
        let steps = ((z1 - z0) / 0.15).ceil().max(1.0) as usize;
        for k in 0..steps {
            let za = z0 + (z1 - z0) * k as f32 / steps as f32;
            let zb = z0 + (z1 - z0) * (k + 1) as f32 / steps as f32;
            let (pa, na) = self.point3(za, sx, u);
            let (pb, nb) = self.point3(zb, sx, u);
            let (ta, _) = self.point3(za, sx, u + 0.15);
            let (tb, _) = self.point3(zb, sx, u + 0.15);
            let (ea, eb) = ((ta - pa).normalize_or_zero() * half_w, (tb - pb).normalize_or_zero() * half_w);
            let (pa, pb) = (pa + na * lift, pb + nb * lift);
            m.quad(pa - ea, pb - eb, pb + eb, pa + ea, col);
        }
    }

    /// Height of the top surface at lateral |x| (searching the top half of the ring)
    pub fn top_y(&self, z: f32, x: f32) -> f32 {
        let s = self.sec(z);
        let hp = Self::half(&s);
        let x = x.abs();
        for i in (4..HALF - 1).rev() {
            let (a, b) = (hp[i], hp[i + 1]);
            let (lo, hi) = (b.x.min(a.x), b.x.max(a.x));
            if x >= lo && x <= hi {
                let t = if (a.x - b.x).abs() < 1e-5 { 0.0 } else { (x - a.x) / (b.x - a.x) };
                return a.y + (b.y - a.y) * t;
            }
        }
        hp[HALF - 1].y
    }

    /// x of the flank at height y on the lower half (max over the side points)
    pub fn side_x(&self, z: f32, y: f32) -> f32 {
        let s = self.sec(z);
        let hp = Self::half(&s);
        for i in 1..6 {
            let (a, b) = (hp[i], hp[i + 1]);
            if (y >= a.y && y <= b.y) || (y <= a.y && y >= b.y) {
                let t = if (b.y - a.y).abs() < 1e-5 { 0.0 } else { (y - a.y) / (b.y - a.y) };
                return a.x + (b.x - a.x) * t;
            }
        }
        hp[3].x
    }
}
