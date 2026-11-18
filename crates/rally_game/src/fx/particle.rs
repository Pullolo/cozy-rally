//! Particle data + simulation. One flat array of plain structs, swap-removed when dead.
use macroquad::math::Vec3;
use rally_core::track::Track;

/// How a particle is drawn.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Shape {
    /// Soft round cloud (dust, smoke, powder, foam). Lit, sorted.
    Puff,
    /// Tiny tumbling triangle (pebbles, snow clumps, sand grains).
    Chunk,
    /// Tumbling rectangle (confetti, grass bits, straw).
    Flake,
    /// Tumbling diamond (leaves, petals).
    Leaf,
    /// Small round droplet (water).
    Drop,
    /// Unlit soft glow dot (fireflies, shift pops).
    Glow,
    /// Unlit little 5-point star (bonks).
    Star,
    /// Unlit velocity-stretched streak (sparks).
    Spark,
    /// Unlit 4-point twinkle (snow/ice/sea sparkle, dust motes).
    Twinkle,
    Butterfly,
    Gull,
    Tumbleweed,
}

impl Shape {
    pub fn glows(self) -> bool {
        matches!(self, Shape::Glow | Shape::Star | Shape::Spark | Shape::Twinkle)
    }
}

/// What happens when a particle reaches the ground.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Ground {
    /// Ignore the ground.
    Ignore,
    /// Vanish on contact.
    Die,
    /// Keep the centre above ground (clouds).
    Hover,
    /// Bounce a couple of times then settle (pebbles).
    Bounce,
    /// Lie flat on the ground (or water) and fade (leaves, confetti).
    Settle,
    /// Keep hopping (tumbleweeds).
    Hop,
}

#[derive(Clone, Copy, Debug)]
pub struct Particle {
    pub pos: Vec3,
    pub vel: Vec3,
    pub age: f32,
    pub life: f32,
    pub size0: f32,
    pub size1: f32,
    pub color: [f32; 3],
    pub alpha: f32,
    pub shape: Shape,
    pub ground: Ground,
    /// Billboard roll angle, tumble angle, or wing phase
    pub rot: f32,
    pub spin: f32,
    /// Flake axis yaw or creature heading
    pub yaw: f32,
    /// Air drag (1/s) towards `wind * wind_k`
    pub drag: f32,
    /// Gravity multiplier (negative = buoyant)
    pub grav: f32,
    pub wind_k: f32,
    /// Sideways sway amplitude (m/s) for fluttering things
    pub flutter: f32,
    pub seed: f32,
    /// Height/width ratio of flakes
    pub aspect: f32,
    /// Fade-in time (s)
    pub fade_in: f32,
    pub ambient: bool,
    /// Ambient group id (for per-biome population control)
    pub group: u8,
    pub resting: bool,
    /// Anchor for creatures (wander centre)
    pub home: Vec3,
}

impl Default for Particle {
    fn default() -> Self {
        Particle {
            pos: Vec3::ZERO,
            vel: Vec3::ZERO,
            age: 0.0,
            life: 1.0,
            size0: 0.5,
            size1: 0.5,
            color: [1.0; 3],
            alpha: 1.0,
            shape: Shape::Puff,
            ground: Ground::Ignore,
            rot: 0.0,
            spin: 0.0,
            yaw: 0.0,
            drag: 1.0,
            grav: 0.0,
            wind_k: 0.0,
            flutter: 0.0,
            seed: 0.0,
            aspect: 1.0,
            fade_in: 0.0,
            ambient: false,
            group: 0,
            resting: false,
            home: Vec3::ZERO,
        }
    }
}

impl Particle {
    #[inline]
    pub fn t(&self) -> f32 {
        (self.age / self.life).clamp(0.0, 1.0)
    }

    /// Current size (ease-out growth for clouds)
    #[inline]
    pub fn size(&self) -> f32 {
        let t = self.t();
        let e = 1.0 - (1.0 - t) * (1.0 - t);
        self.size0 + (self.size1 - self.size0) * e
    }

    /// Current opacity including fade in/out
    #[inline]
    pub fn opacity(&self) -> f32 {
        let t = self.t();
        let fin = if self.fade_in > 0.0 { (self.age / self.fade_in).min(1.0) } else { 1.0 };
        let fout = match self.shape {
            // clouds thin out over their whole life
            Shape::Puff => (1.0 - t) * (1.0 - t) * (1.0 + t),
            _ => ((self.life - self.age) / (self.life * 0.25).clamp(0.05, 1.5)).clamp(0.0, 1.0),
        };
        self.alpha * fin * fout
    }

    #[inline]
    pub fn dead(&self) -> bool {
        self.age >= self.life
    }
}

pub struct SimCtx<'a> {
    pub track: &'a Track,
    pub wind: Vec3,
}

/// Cheap deterministic hash noise in [-1, 1]
#[inline]
fn wobble(x: f32) -> f32 {
    (x * 1.7).sin() * 0.6 + (x * 3.1 + 1.3).sin() * 0.4
}

pub fn step(p: &mut Particle, dt: f32, cx: &SimCtx) {
    p.age += dt;
    match p.shape {
        Shape::Butterfly => return step_butterfly(p, dt, cx),
        Shape::Gull => return step_gull(p, dt, cx),
        Shape::Glow if p.ambient => return step_firefly(p, dt, cx),
        _ => {}
    }
    if p.resting {
        return;
    }
    let target = cx.wind * p.wind_k;
    let k = 1.0 - (-p.drag * dt).exp();
    p.vel += (target - p.vel) * k;
    p.vel.y -= 9.81 * p.grav * dt;
    if p.flutter > 0.0 {
        let ph = p.age * 2.3 + p.seed * 40.0;
        p.pos += Vec3::new(ph.cos(), 0.0, (ph * 0.77).sin()) * (p.flutter * dt);
    }
    p.pos += p.vel * dt;
    p.rot += p.spin * dt;
    if p.shape == Shape::Tumbleweed {
        let hs = Vec3::new(p.vel.x, 0.0, p.vel.z).length();
        p.rot -= hs / p.size0.max(0.1) * dt;
    }
    if p.ground == Ground::Ignore {
        return;
    }
    let tr = cx.track;
    match p.ground {
        Ground::Hover => {
            let h = tr.visual_height(p.pos.x, p.pos.z) + p.size() * 0.3;
            if p.pos.y < h {
                p.pos.y += (h - p.pos.y) * (1.0 - (-8.0 * dt).exp());
                p.vel.y = p.vel.y.max(0.0);
            }
        }
        Ground::Die => {
            if p.pos.y < tr.visual_height(p.pos.x, p.pos.z) {
                p.age = p.life;
            }
        }
        Ground::Bounce => {
            let h = tr.visual_height(p.pos.x, p.pos.z) + p.size0 * 0.4;
            if p.pos.y < h {
                p.pos.y = h;
                if p.vel.y < -1.2 {
                    p.vel.y *= -0.3;
                    p.vel.x *= 0.55;
                    p.vel.z *= 0.55;
                    p.spin *= 0.6;
                } else {
                    p.vel = Vec3::ZERO;
                    p.resting = true;
                    p.life = p.life.min(p.age + 1.2);
                }
            }
        }
        Ground::Settle => {
            let h = tr.visual_height(p.pos.x, p.pos.z) + 0.1;
            if p.pos.y < h {
                p.pos.y = h;
                p.vel = Vec3::ZERO;
                p.resting = true;
                // lie flat
                p.rot = std::f32::consts::FRAC_PI_2;
                p.spin = 0.0;
                p.life = p.life.min(p.age + 2.5);
            }
        }
        Ground::Hop => {
            let h = tr.visual_height(p.pos.x, p.pos.z) + p.size0;
            if p.pos.y < h {
                p.pos.y = h;
                let r = wobble(p.age * 13.0 + p.seed * 50.0) * 0.5 + 0.5;
                p.vel.y = 1.0 + r * 2.4;
            }
        }
        Ground::Ignore => {}
    }
}

fn step_butterfly(p: &mut Particle, dt: f32, cx: &SimCtx) {
    let s = p.seed * 100.0;
    // meander, but drift back home if wandering off
    let mut turn = wobble(p.age * 0.9 + s) * 2.2;
    let to_home = p.home - p.pos;
    let d = Vec3::new(to_home.x, 0.0, to_home.z);
    if d.length() > 7.0 {
        let want = d.z.atan2(d.x);
        let mut diff = want - p.yaw;
        while diff > std::f32::consts::PI {
            diff -= std::f32::consts::TAU;
        }
        while diff < -std::f32::consts::PI {
            diff += std::f32::consts::TAU;
        }
        turn += diff.signum() * 1.6;
    }
    p.yaw += turn * dt;
    let g = cx.track.visual_height(p.pos.x, p.pos.z);
    let want_h = g + 0.7 + 1.1 * (0.5 + 0.5 * (p.age * 0.6 + s).sin());
    let speed = 1.4 + 0.5 * wobble(p.age * 0.5 + s * 2.0);
    p.vel = Vec3::new(p.yaw.cos() * speed, (want_h - p.pos.y) * 1.5 + (p.age * 9.0 + s).sin() * 0.5, p.yaw.sin() * speed) + cx.wind * 0.25;
    p.pos += p.vel * dt;
    p.rot += dt * (13.0 + p.seed * 4.0);
}

fn step_gull(p: &mut Particle, dt: f32, cx: &SimCtx) {
    // lazy circles around `home`, bank with the turn
    let s = p.seed * 100.0;
    p.yaw += p.spin * dt;
    let speed = 6.5;
    let g = cx.track.visual_height(p.pos.x, p.pos.z);
    let want_h = (p.home.y).max(g + 5.0) + (p.age * 0.4 + s).sin() * 1.2;
    p.vel = Vec3::new(p.yaw.cos() * speed, (want_h - p.pos.y) * 0.6, p.yaw.sin() * speed);
    p.pos += (p.vel + cx.wind * 0.4) * dt;
    // flap in bursts, glide in between
    let burst = (p.age * 0.5 + s).sin() > 0.3;
    p.rot += dt * if burst { 9.0 } else { 0.0 };
    if !burst {
        p.rot += ((p.rot / std::f32::consts::TAU).round() * std::f32::consts::TAU - p.rot) * (1.0 - (-4.0 * dt).exp());
    }
}

fn step_firefly(p: &mut Particle, dt: f32, cx: &SimCtx) {
    let s = p.seed * 100.0;
    let wander = Vec3::new(wobble(p.age * 0.8 + s), wobble(p.age * 1.1 + s * 3.0) * 0.5, wobble(p.age * 0.7 + s * 7.0)) * 0.7;
    let pull = (p.home - p.pos) * 0.25;
    p.vel += ((wander + pull) - p.vel) * (1.0 - (-2.0 * dt).exp());
    p.pos += p.vel * dt;
    let g = cx.track.visual_height(p.pos.x, p.pos.z) + 0.3;
    if p.pos.y < g {
        p.pos.y = g;
    }
}
