//! Ambient biome life around each local player: petals & butterflies, falling leaves,
//! snowfall, dust motes & tumbleweeds, sea sparkle & gulls, and fireflies near trees.
use super::particle::{Ground, Particle, Shape};
use super::{Fx, Prio};
use macroquad::math::Vec3;
use rally_core::biome::Biome;
use rally_core::track::{PropKind, Track};
use rally_core::world::World;

/// Ambient particles further than this from every player are removed
pub const KILL_RADIUS: f32 = 48.0;
const SPAWN_RADIUS: f32 = 30.0;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
enum Group {
    Petal = 1,
    Butterfly,
    Leaf,
    Firefly,
    Snow,
    Mote,
    Tumbleweed,
    Sparkle,
    Gull,
    Diamond,
}

const GROUPS: usize = 11;

fn population(b: Biome) -> &'static [(Group, u16)] {
    match b {
        Biome::Meadow => &[(Group::Petal, 110), (Group::Butterfly, 10), (Group::Firefly, 6), (Group::Sparkle, 14)],
        Biome::Autumn => &[(Group::Leaf, 150), (Group::Firefly, 16), (Group::Butterfly, 3), (Group::Sparkle, 10)],
        Biome::Snow => &[(Group::Snow, 360), (Group::Diamond, 40)],
        Biome::Desert => &[(Group::Mote, 70), (Group::Tumbleweed, 5), (Group::Butterfly, 2)],
        Biome::Coast => &[(Group::Sparkle, 120), (Group::Gull, 8), (Group::Petal, 30), (Group::Butterfly, 4)],
    }
}

fn near_tree(track: &Track, p: Vec3, r: f32) -> Option<Vec3> {
    let mut best: Option<(f32, Vec3)> = None;
    track.for_each_prop_near(p, r, |pr| {
        if matches!(pr.kind, PropKind::Tree | PropKind::Pine | PropKind::Bush | PropKind::Palm) {
            let d = pr.pos.distance_squared(p);
            if best.is_none_or(|b| d < b.0) {
                best = Some((d, pr.pos));
            }
        }
    });
    best.map(|b| b.1)
}

impl Fx {
    pub(super) fn update_ambient(&mut self, world: &World, _dt: f32) {
        if self.locals.is_empty() {
            return;
        }
        let nl = self.locals.len();
        if self.ambient_counts.len() != nl {
            self.ambient_counts = vec![[0; 16]; nl];
        }
        for c in self.ambient_counts.iter_mut() {
            *c = [0; 16];
        }
        // count ambient population by group, attributed to the nearest player
        for p in &self.parts {
            if !p.ambient {
                continue;
            }
            let mut best = (f32::MAX, 0);
            for (k, &ci) in self.locals.iter().enumerate() {
                let d = self.poses[ci].0.distance_squared(p.pos);
                if d < best.0 {
                    best = (d, k);
                }
            }
            self.ambient_counts[best.1][p.group as usize % GROUPS] += 1;
        }
        let track = world.track.clone();
        let pop = population(self.biome);
        for k in 0..nl {
            let ci = self.locals[k];
            let car = &world.cars[ci];
            let (pos, rot) = self.poses[ci];
            // centre the life in front of the car, where the chase camera looks
            let f = rot * Vec3::NEG_Z;
            let f = Vec3::new(f.x, 0.0, f.z).normalize_or(Vec3::X);
            let ahead = Vec3::new(car.vel.x, 0.0, car.vel.z) * 1.0;
            let centre = pos + f * 12.0 + ahead.clamp_length_max(18.0);
            for &(g, target) in pop {
                let have = self.ambient_counts[k][g as usize];
                if have >= target {
                    continue;
                }
                // refill gradually (fills up within a few frames at the start)
                let n = (target - have).min(target / 5 + 1);
                for _ in 0..n {
                    if !self.spawn_ambient(&track, g, centre, pos, f) {
                        break;
                    }
                }
                self.ambient_counts[k][g as usize] += n;
            }
        }
    }

    fn disc(&mut self, centre: Vec3, r0: f32, r1: f32) -> Vec3 {
        let d = self.rng.hdir();
        let r = (self.rng.r(r0 * r0, r1 * r1)).sqrt();
        centre + d * r
    }

    fn spawn_ambient(&mut self, track: &Track, g: Group, centre: Vec3, car_pos: Vec3, f: Vec3) -> bool {
        let mut p = Particle { ambient: true, group: g as u8, seed: self.rng.f(), fade_in: 1.2, ..Default::default() };
        match g {
            Group::Petal => {
                let at = self.disc(centre, 2.0, SPAWN_RADIUS);
                let gh = track.visual_height(at.x, at.z);
                p.pos = Vec3::new(at.x, gh + self.rng.r(0.6, 6.0), at.z);
                p.vel = self.wind;
                p.life = self.rng.r(9.0, 15.0);
                p.size0 = self.rng.r(0.13, 0.18);
                p.size1 = p.size0;
                p.color = if self.biome == Biome::Coast {
                    self.rng.pick(&[[1.0, 0.92, 0.5], [1.0, 1.0, 0.95], [1.0, 0.85, 0.6]])
                } else {
                    self.rng.pick(&[[1.0, 0.78, 0.84], [1.0, 0.96, 0.97], [1.0, 0.68, 0.76], [1.0, 0.95, 0.75]])
                };
                p.shape = Shape::Leaf;
                p.ground = Ground::Settle;
                p.rot = self.rng.r(0.0, 6.28);
                p.spin = self.rng.r(-4.0, 4.0);
                p.yaw = self.rng.r(0.0, 6.28);
                p.drag = 2.0;
                p.grav = 0.05;
                p.wind_k = 1.0;
                p.flutter = 0.8;
                p.aspect = 0.75;
            }
            Group::Leaf => {
                let at = self.disc(centre, 2.0, SPAWN_RADIUS);
                let gh = track.visual_height(at.x, at.z);
                // half the leaves fall around trees, the rest drift over the road
                let tree = if self.rng.f() < 0.5 { near_tree(track, at, 7.0) } else { None };
                let (x, z, h) = match tree {
                    Some(t) => (t.x + self.rng.r(-2.0, 2.0), t.z + self.rng.r(-2.0, 2.0), self.rng.r(2.5, 6.0)),
                    None => (at.x, at.z, self.rng.r(3.0, 9.0)),
                };
                p.pos = Vec3::new(x, gh + h, z);
                p.vel = self.wind;
                p.life = self.rng.r(12.0, 18.0);
                p.size0 = self.rng.r(0.18, 0.26);
                p.size1 = p.size0;
                let c = self.rng.pick(&track.style.leaves);
                let k = self.rng.r(0.9, 1.1);
                p.color = [c[0] * k, c[1] * k, c[2] * k];
                p.shape = Shape::Leaf;
                p.ground = Ground::Settle;
                p.rot = self.rng.r(0.0, 6.28);
                p.spin = self.rng.r(-3.5, 3.5);
                p.yaw = self.rng.r(0.0, 6.28);
                p.drag = 1.6;
                p.grav = 0.1;
                p.wind_k = 1.0;
                p.flutter = 1.3;
                p.aspect = 0.62;
            }
            Group::Butterfly => {
                let at = self.disc(centre, 2.0, SPAWN_RADIUS * 0.45);
                let gh = track.visual_height(at.x, at.z);
                if track.water_level.is_some_and(|w| gh < w + 0.2) {
                    return true;
                }
                p.pos = Vec3::new(at.x, gh + self.rng.r(0.9, 2.2), at.z);
                p.home = p.pos;
                p.life = self.rng.r(25.0, 40.0);
                p.size0 = self.rng.r(0.34, 0.42);
                p.size1 = p.size0;
                p.color = self.rng.pick(&[[1.0, 0.86, 0.36], [0.62, 0.80, 1.0], [1.0, 0.72, 0.82], [1.0, 0.99, 0.95], [1.0, 0.64, 0.38], [0.78, 0.66, 1.0]]);
                p.shape = Shape::Butterfly;
                p.yaw = self.rng.r(0.0, 6.28);
                p.vel = Vec3::new(p.yaw.cos(), 0.0, p.yaw.sin());
                p.rot = self.rng.r(0.0, 6.28);
                p.fade_in = 1.5;
            }
            Group::Firefly => {
                let at = self.disc(centre, 3.0, SPAWN_RADIUS);
                let Some(t) = near_tree(track, at, 9.0) else { return true };
                let off = self.rng.hdir() * self.rng.r(0.8, 3.0);
                let gh = track.visual_height(t.x + off.x, t.z + off.z);
                p.pos = Vec3::new(t.x + off.x, gh + self.rng.r(0.5, 2.2), t.z + off.z);
                p.home = p.pos;
                p.life = self.rng.r(18.0, 30.0);
                p.size0 = 0.26;
                p.size1 = 0.26;
                p.color = [0.88, 1.0, 0.45];
                p.alpha = 0.95;
                p.shape = Shape::Glow;
                p.fade_in = 2.0;
            }
            Group::Snow => {
                let at = self.disc(centre, 0.0, SPAWN_RADIUS * 0.75);
                let gh = track.visual_height(at.x, at.z);
                p.pos = Vec3::new(at.x, gh.max(car_pos.y - 2.0) + self.rng.r(0.5, 13.0), at.z);
                p.vel = self.wind + Vec3::Y * -0.8;
                p.life = 25.0;
                p.size0 = self.rng.r(0.04, 0.075);
                p.size1 = p.size0;
                p.color = [1.0, 1.0, 1.0];
                p.alpha = 0.95;
                p.shape = Shape::Drop;
                p.ground = Ground::Die;
                p.drag = 1.2;
                p.grav = 0.09;
                p.wind_k = 1.0;
                p.flutter = 0.5;
            }
            Group::Diamond | Group::Mote => {
                let at = self.disc(centre, 1.0, SPAWN_RADIUS * 0.7);
                let gh = track.visual_height(at.x, at.z);
                p.pos = Vec3::new(at.x, gh + self.rng.r(0.3, 4.5), at.z);
                p.vel = self.wind * 0.6;
                p.life = self.rng.r(5.0, 9.0);
                p.size0 = self.rng.r(0.05, 0.09);
                p.size1 = p.size0;
                p.color = if g == Group::Mote { [1.0, 0.94, 0.78] } else { [0.88, 0.95, 1.0] };
                p.alpha = 0.7;
                p.shape = Shape::Twinkle;
                p.drag = 1.0;
                p.wind_k = 0.6;
                p.flutter = 0.35;
                p.rot = self.rng.r(0.0, 1.0);
            }
            Group::Tumbleweed => {
                let wdir = self.wind.normalize_or(Vec3::X);
                let side = Vec3::new(-wdir.z, 0.0, wdir.x);
                // start upwind of a spot in front of the car so they roll across the view
                let at = centre + side * self.rng.r(-10.0, 10.0) + self.rng.hdir() * self.rng.r(0.0, 10.0) - wdir * self.rng.r(8.0, 18.0);
                let gh = track.visual_height(at.x, at.z);
                p.size0 = self.rng.r(0.55, 0.85);
                p.size1 = p.size0;
                p.pos = Vec3::new(at.x, gh + p.size0 + 0.5, at.z);
                p.vel = self.wind;
                p.life = self.rng.r(16.0, 22.0);
                p.color = [0.80, 0.64, 0.42];
                p.alpha = 1.0;
                p.shape = Shape::Tumbleweed;
                p.ground = Ground::Hop;
                p.drag = 0.7;
                p.grav = 1.0;
                p.wind_k = 1.8;
                p.fade_in = 1.0;
            }
            Group::Sparkle => {
                let Some(wl) = track.water_level else { return true };
                // try a few spots for open water
                let mut found = None;
                for _ in 0..6 {
                    let at = self.disc(centre, 2.0, SPAWN_RADIUS * 1.5);
                    if track.terrain.height(at.x, at.z) < wl - 0.3 {
                        found = Some(at);
                        break;
                    }
                }
                let Some(at) = found else { return true };
                p.pos = Vec3::new(at.x, wl + self.rng.r(0.05, 0.3), at.z);
                p.life = self.rng.r(0.6, 1.6);
                p.size0 = self.rng.r(0.32, 0.48);
                p.size1 = p.size0 * 0.6;
                p.color = [1.0, 1.0, 0.97];
                p.alpha = 0.95;
                p.shape = Shape::Twinkle;
                p.rot = self.rng.r(0.0, 1.0);
                p.fade_in = 0.2;
            }
            Group::Gull => {
                let at = self.disc(centre + f * 10.0, 4.0, SPAWN_RADIUS);
                let gh = track.visual_height(at.x, at.z);
                p.home = Vec3::new(at.x, gh.max(track.water_level.unwrap_or(gh)) + self.rng.r(6.0, 11.0), at.z);
                p.pos = p.home;
                p.life = self.rng.r(35.0, 50.0);
                p.size0 = self.rng.r(1.7, 2.0);
                p.size1 = p.size0;
                p.color = [0.98, 0.98, 0.96];
                p.shape = Shape::Gull;
                p.yaw = self.rng.r(0.0, 6.28);
                p.spin = self.rng.r(0.22, 0.42) * if self.rng.f() < 0.5 { -1.0 } else { 1.0 };
                p.vel = Vec3::new(p.yaw.cos(), 0.0, p.yaw.sin());
                p.rot = self.rng.r(0.0, 6.28);
                p.fade_in = 2.0;
            }
        }
        self.spawn(p, Prio::Ambient)
    }
}
