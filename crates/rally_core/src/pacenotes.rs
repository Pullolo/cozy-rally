//! Rally pace notes derived from the track centre line.
//!
//! Notes follow real rally conventions: corners are graded 1 (slowest) .. 6 (fastest) from the
//! speed a reference car can carry through them — curvature, corner angle (how much of the road
//! width a racing line can use), banking/camber and surface grip all count, not raw radius. Very
//! tight 180° turns are "hairpin", compact 90° junction corners are "square", near-flat direction
//! changes are "kink". Modifiers (tightens, opens, long, crest, over crest, jump, don't cut,
//! caution, narrows, into, surface changes...) and rounded distance calls complete the note.
//!
//! The data model is UI-agnostic and serialisable so a co-driver mode (another player reading
//! the notes aloud on their own screen, or over the network) can reuse it as-is.
use crate::surface::Surface;
use crate::track::{Track, SAMPLE_SPACING, SHOULDER};
use serde::{Deserialize, Serialize};

const G: f32 = 9.81;
/// Grip of the reference car (tyre grip x safety factor) used to grade corners.
pub const REF_GRIP: f32 = 1.0;
/// Upper speed bound for each grade 1..=6 in km/h (reference car). Above the last = flat.
pub const GRADE_KMH: [f32; 6] = [48.0, 62.0, 78.0, 96.0, 118.0, 145.0];
/// Distance calls used when reading the gap to the next note (metres)
pub const DISTANCE_CALLS: [u32; 15] = [30, 50, 80, 100, 120, 150, 200, 250, 300, 350, 400, 500, 600, 700, 800];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum NoteKind {
    /// Numbered corner 1..6
    Corner,
    /// Very slow ~180° turn
    Hairpin,
    /// Compact ~90° junction-style corner
    Square,
    /// Small, near-flat change of direction
    Kink,
    /// Crest on a straight (no corner attached)
    Crest,
    /// Jump take-off
    Jump,
    /// Hazard on a straight: surface change, narrowing, compression
    Hazard,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Direction {
    Left,
    Right,
    /// For notes that are not corners
    None,
}

impl Direction {
    pub fn word(self) -> &'static str {
        match self {
            Direction::Left => "left",
            Direction::Right => "right",
            Direction::None => "",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Modifier {
    /// Corner turns through a lot of angle
    Long,
    /// Gets tighter towards the exit (see `exit_severity`)
    Tightens,
    /// Opens up towards the exit
    Opens,
    /// Blind crest right before the corner
    Crest,
    /// Corner is taken over a crest
    OverCrest,
    /// A jump leads into this note
    Jump,
    /// Dip / compression in the corner
    Compression,
    /// Road leans the wrong way
    OffCamber,
    /// Obstacles on the inside: keep off the apex
    DontCut,
    /// Road gets narrower
    Narrows,
    /// Surface changes to this one
    SurfaceChange(Surface),
    Caution,
    DoubleCaution,
    /// No gap to the next note
    Into,
    /// The surface change is a short patch (ice patch, mud, sand drift), not a new section
    Patch,
}

impl Modifier {
    pub fn word(self) -> String {
        match self {
            Modifier::Long => "long".into(),
            Modifier::Tightens => "tightens".into(),
            Modifier::Opens => "opens".into(),
            Modifier::Crest => "crest".into(),
            Modifier::OverCrest => "over crest".into(),
            Modifier::Jump => "jump".into(),
            Modifier::Compression => "compression".into(),
            Modifier::OffCamber => "off camber".into(),
            Modifier::DontCut => "don't cut".into(),
            Modifier::Narrows => "narrows".into(),
            Modifier::SurfaceChange(s) => format!("onto {}", surface_word(s)),
            Modifier::Caution => "caution".into(),
            Modifier::DoubleCaution => "double caution".into(),
            Modifier::Into => "into".into(),
            Modifier::Patch => "patch".into(),
        }
    }
}

pub fn surface_word(s: Surface) -> &'static str {
    match s {
        Surface::Asphalt => "tarmac",
        Surface::Gravel => "gravel",
        Surface::Dirt => "dirt",
        Surface::Grass => "grass",
        Surface::Sand => "sand",
        Surface::Snow => "snow",
        Surface::Ice => "ice",
        Surface::Mud => "mud",
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PaceNote {
    /// Arc length where the note starts (turn-in / hazard start), metres from the start line
    pub s_start: f32,
    /// Arc length where it ends (exit). May be < s_start if it wraps over the line.
    pub s_end: f32,
    pub kind: NoteKind,
    pub direction: Direction,
    /// 1 (slowest) ..= 6 (fastest); hairpins are 1, kinks 6, non-corner notes 0
    pub severity: u8,
    /// For tightening/opening corners: grade at the exit
    pub exit_severity: Option<u8>,
    pub modifiers: Vec<Modifier>,
    /// Rounded distance call to the next note (metres); 0 = "into"
    pub distance_to_next: u32,
    /// Total heading change (degrees, unsigned)
    pub angle_deg: f32,
    /// Tightest radius on the centre line (m)
    pub min_radius: f32,
    /// Estimated speed of the reference car (km/h)
    pub speed_kmh: f32,
    /// Spoken form, e.g. "caution crest left 4 tightens 2 100"
    pub text: String,
}

impl PaceNote {
    pub fn has(&self, m: Modifier) -> bool {
        self.modifiers.contains(&m)
    }

    pub fn is_corner(&self) -> bool {
        matches!(self.kind, NoteKind::Corner | NoteKind::Hairpin | NoteKind::Square | NoteKind::Kink)
    }

    pub fn surface_change(&self) -> Option<Surface> {
        self.modifiers.iter().find_map(|m| if let Modifier::SurfaceChange(s) = m { Some(*s) } else { None })
    }

    /// The note without its link: "caution crest left 4 tightens 2 don't cut"
    pub fn call(&self) -> String {
        let mut w: Vec<String> = Vec::new();
        if self.has(Modifier::DoubleCaution) {
            w.push("double caution".into());
        } else if self.has(Modifier::Caution) {
            w.push("caution".into());
        }
        if self.has(Modifier::Crest) && self.kind != NoteKind::Crest {
            w.push("crest".into());
        }
        let dir = self.direction.word();
        match self.kind {
            NoteKind::Corner => w.push(format!("{dir} {}", self.severity)),
            NoteKind::Hairpin => w.push(format!("hairpin {dir}")),
            NoteKind::Square => w.push(format!("square {dir}")),
            NoteKind::Kink => w.push(format!("kink {dir}")),
            NoteKind::Crest => w.push("crest".into()),
            NoteKind::Jump => w.push("jump".into()),
            NoteKind::Hazard => {}
        }
        for m in &self.modifiers {
            match m {
                Modifier::SurfaceChange(sf) if self.has(Modifier::Patch) => w.push(match sf {
                    Surface::Sand => "sand drift".into(),
                    other => format!("{} patch", surface_word(*other)),
                }),
                Modifier::Long | Modifier::OverCrest | Modifier::Compression | Modifier::OffCamber | Modifier::Narrows | Modifier::DontCut | Modifier::SurfaceChange(_) => w.push(m.word()),
                Modifier::Tightens | Modifier::Opens => match self.exit_severity {
                    Some(e) if e.abs_diff(self.severity) >= 2 => w.push(format!("{} {e}", m.word())),
                    _ => w.push(m.word()),
                },
                _ => {}
            }
        }
        w.retain(|s| !s.is_empty());
        w.join(" ")
    }

    /// Spoken form including the link to the next note: "left 4 tightens into" / "right 2 100"
    pub fn text(&self) -> String {
        let c = self.call();
        if self.has(Modifier::Into) || self.distance_to_next == 0 {
            format!("{c} into")
        } else {
            format!("{c} {}", self.distance_to_next)
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PaceNotes {
    pub track_length: f32,
    pub notes: Vec<PaceNote>,
}

/// One entry of `upcoming`
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Upcoming {
    pub index: usize,
    /// Metres until the note starts (negative while inside it)
    pub distance: f32,
    /// 0..1 progress through the note while inside it
    pub progress: f32,
}

impl PaceNotes {
    /// Whole stage read out in order
    pub fn stage_text(&self) -> String {
        self.notes.iter().map(|n| n.text()).collect::<Vec<_>>().join(", ")
    }

    pub fn len(&self) -> usize {
        self.notes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.notes.is_empty()
    }
}

/// The next `n` notes for a car at arc length `s` (the note it is inside first, if any).
pub fn upcoming(notes: &PaceNotes, s: f32, n: usize) -> Vec<Upcoming> {
    let len = notes.track_length;
    let mut v: Vec<Upcoming> = notes
        .notes
        .iter()
        .enumerate()
        .map(|(i, note)| {
            let to_start = (note.s_start - s).rem_euclid(len);
            let to_end = (note.s_end - s).rem_euclid(len);
            let span = (note.s_end - note.s_start).rem_euclid(len);
            if to_end < span && to_start > to_end {
                // inside
                let into = span - to_end;
                Upcoming { index: i, distance: -into, progress: (into / span.max(0.1)).clamp(0.0, 1.0) }
            } else {
                Upcoming { index: i, distance: to_start, progress: 0.0 }
            }
        })
        .collect();
    v.sort_by(|a, b| a.distance.partial_cmp(&b.distance).unwrap_or(std::cmp::Ordering::Equal));
    v.truncate(n);
    v
}

/// Grade (1..=6, 7 = flat) for a speed in km/h
pub fn grade_for_speed(kmh: f32) -> u8 {
    GRADE_KMH.iter().position(|&g| kmh < g).map(|p| p as u8 + 1).unwrap_or(7)
}

/// Max steady-state cornering speed (m/s) for curvature `kappa` (1/m), bank angle towards the
/// inside of the turn (rad, negative = off-camber) and friction coefficient `mu`.
pub fn corner_speed(kappa: f32, bank_into: f32, mu: f32) -> f32 {
    let k = kappa.abs().max(1e-5);
    let tb = bank_into.tan();
    let num = (mu + tb).max(0.05);
    let den = (1.0 - mu * tb).max(0.2);
    (G / k * num / den).sqrt().min(60.0)
}

/// Per-sample reference speed (m/s) along the centre line, from curvature averaged over 8 m,
/// banking and per-sample surface grip. Useful as an AI speed hint.
pub fn reference_speeds(track: &Track) -> Vec<f32> {
    let n = track.samples.len();
    let k = avg_abs_curv(track, 2);
    (0..n)
        .map(|i| {
            let smp = &track.samples[i];
            let bank_into = smp.bank * smp.curvature.signum();
            corner_speed(k[i], bank_into, REF_GRIP * smp.surface.props().grip)
        })
        .collect()
}

fn avg_abs_curv(track: &Track, r: isize) -> Vec<f32> {
    let n = track.samples.len();
    (0..n)
        .map(|i| {
            let mut s = 0.0;
            for k in -r..=r {
                s += track.samples[(i as isize + k).rem_euclid(n as isize) as usize].curvature;
            }
            (s / (2 * r + 1) as f32).abs()
        })
        .collect()
}

fn round_call(gap: f32) -> u32 {
    if gap < 15.0 {
        return 0;
    }
    let g = gap * 1.08;
    let mut best = DISTANCE_CALLS[0];
    for &c in DISTANCE_CALLS.iter() {
        if c as f32 <= g {
            best = c;
        }
    }
    best
}

struct Run {
    i0: usize,
    /// inclusive end (may exceed n: unwrapped)
    i1: usize,
    sign: f32,
}

/// Build the pace notes for a track. Deterministic (pure function of the track).
pub fn notes_for(track: &Track) -> PaceNotes {
    let n = track.samples.len();
    let len = track.length;
    let ds = SAMPLE_SPACING;
    let at = |i: isize| &track.samples[i.rem_euclid(n as isize) as usize];
    let kavg = avg_abs_curv(track, 2);
    let ksig: Vec<f32> = (0..n).map(|i| track.samples[i].curvature).collect();

    // ---- 1. corner runs: |k| above threshold with constant sign. Start at a straight sample.
    let k_on = 1.0 / 320.0;
    let start = (0..n).find(|&i| ksig[i].abs() < k_on).unwrap_or(0);
    let mut runs: Vec<Run> = Vec::new();
    let mut cur: Option<Run> = None;
    for o in 0..=n {
        let i = start + o;
        let k = ksig[i % n];
        let on = k.abs() >= k_on && o < n;
        match cur.as_mut() {
            Some(r) if on && k.signum() == r.sign => r.i1 = i,
            _ => {
                if let Some(r) = cur.take() {
                    runs.push(r);
                }
                if on {
                    cur = Some(Run { i0: i, i1: i, sign: k.signum() });
                }
            }
        }
    }
    // merge same-sign runs separated by a tiny gap
    let mut merged: Vec<Run> = Vec::new();
    for r in runs {
        if let Some(last) = merged.last_mut() {
            if last.sign == r.sign && (r.i0 - last.i1) as f32 * ds < 10.0 {
                last.i1 = r.i1;
                continue;
            }
        }
        merged.push(r);
    }
    // split long runs at deep curvature valleys (two corners in the same direction)
    let mut split: Vec<Run> = Vec::new();
    for r in merged {
        let mut stack = vec![r];
        while let Some(r) = stack.pop() {
            let mut cut = None;
            if (r.i1 - r.i0) as f32 * ds > 40.0 {
                let mut best = f32::MAX;
                for i in r.i0 + 6..r.i1.saturating_sub(6) {
                    let k = kavg[i % n];
                    let left = (r.i0..i).map(|j| kavg[j % n]).fold(0.0, f32::max);
                    let right = (i + 1..=r.i1).map(|j| kavg[j % n]).fold(0.0, f32::max);
                    if k < 0.3 * left.min(right) && k < best {
                        best = k;
                        cut = Some(i);
                    }
                }
            }
            match cut {
                Some(c) => {
                    stack.push(Run { i0: c + 1, i1: r.i1, sign: r.sign });
                    stack.push(Run { i0: r.i0, i1: c, sign: r.sign });
                }
                None => split.push(r),
            }
        }
    }

    // ---- 2. corner notes
    let mut notes: Vec<PaceNote> = Vec::new();
    for r in &split {
        let angle: f32 = (r.i0..=r.i1).map(|i| ksig[i % n].abs() * ds).sum();
        let angle_deg = angle.to_degrees();
        if angle_deg < 8.0 {
            continue;
        }
        let peak = (r.i0..=r.i1).map(|i| kavg[i % n]).fold(0.0, f32::max);
        // trim to the part that really turns
        let thr = (0.25 * peak).max(k_on);
        let mut a = r.i0;
        while a < r.i1 && kavg[a % n] < thr {
            a += 1;
        }
        let mut b = r.i1;
        while b > a && kavg[b % n] < thr {
            b -= 1;
        }
        let hw_min = (a..=b).map(|i| track.samples[i % n].half_width).fold(f32::MAX, f32::min);
        let mu = REF_GRIP * (a.saturating_sub(5)..=b).map(|i| track.samples[i % n].surface.props().grip).fold(f32::MAX, f32::min);
        let bank_into = (a..=b).map(|i| track.samples[i % n].bank).sum::<f32>() / (b - a + 1) as f32 * r.sign;
        let w_use = (2.0 * hw_min - 2.2).max(1.0);
        let line_gain = 0.55 * w_use / (1.0 - (angle.min(std::f32::consts::PI) * 0.5).cos()).max(1e-3);
        let speed_for = |k: f32| corner_speed(1.0 / (1.0 / k.max(1e-5) + line_gain), bank_into, mu) * 3.6;
        let v = speed_for(peak);
        let mut sev = grade_for_speed(v);
        let min_radius = 1.0 / peak;
        let span_len = (b - a + 1) as f32 * ds;

        // tightens / opens: compare entry and exit thirds
        let third = ((b - a) / 3).max(1);
        let k_in = (a..=a + third).map(|i| kavg[i % n]).fold(0.0, f32::max);
        let k_out = (b.saturating_sub(third).max(a)..=b).map(|i| kavg[i % n]).fold(0.0, f32::max);
        let (sev_in, sev_out) = (grade_for_speed(speed_for(k_in)).min(6), grade_for_speed(speed_for(k_out)).min(6));
        let mut mods = Vec::new();
        let mut exit_severity = None;

        let kind = if angle_deg >= 135.0 && sev <= 2 {
            sev = 1;
            NoteKind::Hairpin
        } else if (68.0..=115.0).contains(&angle_deg) && min_radius <= 24.0 && span_len <= 1.9 * min_radius * angle {
            sev = sev.min(3);
            NoteKind::Square
        } else if sev >= 7 && angle_deg < 35.0 || (sev >= 6 && angle_deg < 14.0) {
            sev = 6;
            NoteKind::Kink
        } else {
            sev = sev.min(6);
            NoteKind::Corner
        };
        if kind == NoteKind::Corner {
            if k_out > 1.5 * k_in && sev_out < sev_in {
                mods.push(Modifier::Tightens);
                sev = sev_in;
                exit_severity = Some(sev_out);
            } else if k_in > 1.5 * k_out && sev_in < sev_out {
                mods.push(Modifier::Opens);
                sev = sev_in;
                exit_severity = Some(sev_out);
            }
            if angle_deg >= 100.0 || span_len > 75.0 {
                mods.insert(0, Modifier::Long);
            }
        }
        if bank_into < -0.025 && kind != NoteKind::Kink {
            mods.push(Modifier::OffCamber);
        }
        notes.push(PaceNote {
            s_start: (a % n) as f32 * ds,
            s_end: ((b % n) as f32 * ds + ds).rem_euclid(len),
            kind,
            direction: if r.sign > 0.0 { Direction::Left } else { Direction::Right },
            severity: sev,
            exit_severity,
            modifiers: mods,
            distance_to_next: 0,
            angle_deg,
            min_radius,
            speed_kmh: v.min(200.0),
            text: String::new(),
        });
    }
    notes.sort_by(|a, b| a.s_start.partial_cmp(&b.s_start).unwrap());

    // helper: which corner note (if any) covers / follows arc length s within a window
    let fwd = |a: f32, b: f32| (b - a).rem_euclid(len);
    // ---- 3. vertical features: crests, compressions, jumps
    let h = |i: isize| at(i).pos.y;
    let w = 7isize; // +-14 m
    // candidates, then non-maximum suppression (plateaus give runs of equal maxima)
    let mut cands: Vec<(usize, f32)> = Vec::new();
    for i in 0..n as isize {
        let c = (h(i - w) - 2.0 * h(i) + h(i + w)) / ((2 * w) as f32 * ds).powi(2) * 4.0;
        let prom = h(i) - 0.5 * (h(i - 2 * w) + h(i + 2 * w));
        let local_max = (1..=w).all(|k| h(i) >= h(i - k) && h(i) >= h(i + k));
        let local_min = (1..=w).all(|k| h(i) <= h(i - k) && h(i) <= h(i + k));
        if (local_max && prom > 0.9 && c < -0.004) || (local_min && prom < -0.9 && c > 0.004) {
            cands.push((i as usize, prom));
        }
    }
    cands.sort_by(|a, b| b.1.abs().partial_cmp(&a.1.abs()).unwrap());
    let mut crests: Vec<(usize, bool)> = Vec::new(); // (index, is_crest) ; false = compression
    for (i, prom) in cands {
        if crests.iter().all(|&(j, _)| {
            let d = (i as isize - j as isize).unsigned_abs();
            d.min(n - d) as f32 * ds > 30.0
        }) {
            crests.push((i, prom > 0.0));
        }
    }
    crests.sort();
    let jump_set: Vec<usize> = track.jumps.clone();
    let mut extra: Vec<PaceNote> = Vec::new();
    let blank = |s0: f32, s1: f32, kind: NoteKind, mods: Vec<Modifier>| PaceNote {
        s_start: s0.rem_euclid(len),
        s_end: s1.rem_euclid(len),
        kind,
        direction: Direction::None,
        severity: 0,
        exit_severity: None,
        modifiers: mods,
        distance_to_next: 0,
        angle_deg: 0.0,
        min_radius: 0.0,
        speed_kmh: 0.0,
        text: String::new(),
    };
    for &(i, is_crest) in &crests {
        let s = i as f32 * ds;
        if jump_set.iter().any(|&j| fwd(j as f32 * ds, s) < 30.0 || fwd(s, j as f32 * ds) < 20.0) {
            continue;
        }
        // attached to a corner?
        let mut attached = false;
        for nt in notes.iter_mut().filter(|nt| nt.kind != NoteKind::Kink || !is_crest) {
            let to_start = fwd(s, nt.s_start);
            let span = fwd(nt.s_start, nt.s_end);
            let inside = fwd(nt.s_start, s) <= span;
            if is_crest {
                if !inside && to_start <= 35.0 {
                    nt.modifiers.push(Modifier::Crest);
                    attached = true;
                    break;
                } else if inside && fwd(nt.s_start, s) > 3.0 {
                    nt.modifiers.push(Modifier::OverCrest);
                    attached = true;
                    break;
                }
            } else if inside || to_start <= 10.0 {
                nt.modifiers.push(Modifier::Compression);
                attached = true;
                break;
            }
        }
        if !attached {
            if is_crest {
                extra.push(blank(s - 6.0, s + 6.0, NoteKind::Crest, vec![]));
            } else {
                extra.push(blank(s - 6.0, s + 6.0, NoteKind::Hazard, vec![Modifier::Compression]));
            }
        }
    }
    for &j in &jump_set {
        let s = j as f32 * ds;
        extra.push(blank(s - 4.0, s + 4.0, NoteKind::Jump, vec![]));
    }

    // ---- 4. surface changes and narrowing
    // surface changes, called where the transition / patch *starts*
    let map = &track.surface_map;
    let mut events: Vec<(f32, f32, Surface, Surface, bool)> = Vec::new(); // (s, length, from, to, patch)
    for tr in &map.transitions {
        events.push((tr.s, tr.len, tr.from, tr.to, false));
    }
    for p in &map.patches {
        let base = map.section_mix(p.s, 0.0);
        let base = if base.t >= 0.5 { base.b } else { base.a };
        events.push((p.s_start(len), p.half_len * 2.0, base, p.surface, true));
    }
    for (s, run, from, to, patch) in events {
        let slippery = to == Surface::Ice || to.props().grip < from.props().grip - 0.12;
        let mut mods = vec![Modifier::SurfaceChange(to)];
        if patch {
            mods.push(Modifier::Patch);
        }
        // inside a corner: attach to it; otherwise its own note at the start of the change
        let target = notes.iter().position(|nt| {
            let span = fwd(nt.s_start, nt.s_end);
            fwd(nt.s_start, s) <= span
        });
        match target {
            Some(t) => notes[t].modifiers.extend(mods),
            None => {
                if slippery {
                    mods.insert(0, Modifier::Caution);
                }
                extra.push(blank(s, s + run.min(40.0), NoteKind::Hazard, mods));
            }
        }
    }
    for nt in notes.iter_mut().filter(|nt| matches!(nt.kind, NoteKind::Corner | NoteKind::Kink)) {
        let a = (nt.s_start / ds) as isize;
        let before = (15..=25).map(|k| at(a - k).half_width).sum::<f32>() / 11.0;
        let span = (fwd(nt.s_start, nt.s_end) / ds) as isize;
        let inside = (0..=span).map(|k| at(a + k).half_width).fold(f32::MAX, f32::min);
        if before - inside > 1.2 {
            nt.modifiers.push(Modifier::Narrows);
        }
        // a dip inside a corner that already has a crest call is noise
        if nt.has(Modifier::Crest) || nt.has(Modifier::OverCrest) {
            nt.modifiers.retain(|m| *m != Modifier::Compression);
        }
    }
    // ---- 5. don't cut: solid obstacles on the inside near the apex
    for nt in notes.iter_mut().filter(|nt| matches!(nt.kind, NoteKind::Corner | NoteKind::Hairpin | NoteKind::Square) && nt.severity <= 4) {
        let a = (nt.s_start / ds) as isize;
        let span = (fwd(nt.s_start, nt.s_end) / ds) as isize;
        let mut hit = false;
        for k in (0..=span).step_by(2) {
            let smp = at(a + k);
            let inside = if nt.direction == Direction::Left { -1.0 } else { 1.0 };
            let p = smp.pos + smp.right * inside * (smp.half_width + SHOULDER * 0.5 + 1.0);
            track.for_each_prop_near(p, 4.0, |pr| {
                if pr.radius > 0.25 && glam::Vec2::new(pr.pos.x - p.x, pr.pos.z - p.z).length() < 2.5 + pr.radius {
                    hit = true;
                }
            });
            if hit {
                break;
            }
        }
        if hit {
            nt.modifiers.push(Modifier::DontCut);
        }
    }

    // ---- 6. merge, order, cautions, links
    notes.extend(extra);
    notes.sort_by(|a, b| a.s_start.partial_cmp(&b.s_start).unwrap());
    // jumps attach to the next corner if it follows right away
    let m = notes.len();
    for k in 0..m {
        if notes[k].kind == NoteKind::Jump {
            let nx = (k + 1) % m;
            if notes[nx].is_corner() && fwd(notes[k].s_end, notes[nx].s_start) < 45.0 {
                notes[nx].modifiers.push(Modifier::Jump);
            }
        }
    }
    // cautions: how much braking is needed + blind / slippery / tightening factors
    let mut prev_exit_speed = 0.0f32;
    let mut prev_end = 0.0f32;
    // find the last corner before s=0 to seed the approach model
    if let Some(last) = notes.iter().rev().find(|nt| nt.is_corner()) {
        prev_exit_speed = last.speed_kmh / 3.6;
        prev_end = last.s_end;
    }
    for nt in notes.iter_mut() {
        if !nt.is_corner() {
            if nt.kind == NoteKind::Hazard && nt.surface_change() == Some(Surface::Ice) && !nt.has(Modifier::Caution) {
                nt.modifiers.insert(0, Modifier::Caution);
            }
            continue;
        }
        let gap = fwd(prev_end, nt.s_start);
        let v_app = (prev_exit_speed * prev_exit_speed + 2.0 * 4.5 * gap).sqrt().min(40.0);
        let v_c = nt.speed_kmh / 3.6;
        let ratio = v_app / v_c.max(1.0);
        let mut score = 0;
        if ratio > 2.4 {
            score += 1;
        }
        if ratio > 3.3 {
            score += 1;
        }
        if (nt.has(Modifier::Crest) || nt.has(Modifier::OverCrest)) && nt.severity <= 3 {
            score += 1;
        }
        if nt.has(Modifier::Tightens) && nt.exit_severity.unwrap_or(6) <= 2 {
            score += 1;
        }
        if matches!(nt.surface_change(), Some(Surface::Ice) | Some(Surface::Mud) | Some(Surface::Sand)) {
            score += 1;
        }
        if nt.has(Modifier::OffCamber) && nt.severity <= 3 {
            score += 1;
        }
        if nt.has(Modifier::Jump) && nt.severity <= 4 {
            score += 1;
        }
        if score >= 3 {
            nt.modifiers.insert(0, Modifier::DoubleCaution);
        } else if score == 2 {
            nt.modifiers.insert(0, Modifier::Caution);
        }
        prev_exit_speed = if nt.has(Modifier::Opens) { v_c * 1.25 } else { v_c };
        prev_end = nt.s_end;
    }
    for k in 0..m {
        let nx = (k + 1) % m;
        let gap = fwd(notes[k].s_end, notes[nx].s_start);
        let gap = if gap > len * 0.9 { 0.0 } else { gap }; // overlapping notes
        let d = round_call(gap);
        notes[k].distance_to_next = d;
        if d == 0 {
            notes[k].modifiers.push(Modifier::Into);
        }
        // tidy: stable modifier order, no duplicates
        let mut seen = Vec::new();
        notes[k].modifiers.retain(|m| {
            if seen.contains(m) {
                false
            } else {
                seen.push(*m);
                true
            }
        });
    }
    for nt in notes.iter_mut() {
        nt.text = nt.text();
    }
    PaceNotes { track_length: len, notes }
}
