//! Campaign screens: chapter map + stage cards, stage intro, stage results, leaderboard.
//! Data comes from `campaign_data` (adapter over rally_core::campaign + the local store).
use crate::campaign_data::*;
use crate::ui::*;
use crate::{audio, hud, models, settings, App, Screen, ASSISTS};
use macroquad::prelude::*;
use rally_core::campaign::{CampaignKind, Chapter, EventKind, Medal, MedalTimes, StageDef};
use rally_core::race::format_time;
use rally_core::track::Track;
use rally_core::vehicle::car_specs;
use std::sync::mpsc::{channel, Receiver};
use std::sync::Arc;

/// A finished stage run for the results screen
#[derive(Clone)]
pub struct UiResult {
    pub r: crate::campaign_mode::StageResult,
    /// Splits of the best run *before* this one (for the splits table)
    pub best_splits: Vec<f32>,
    pub medal_times: Option<MedalTimes>,
}

pub struct CampState {
    /// Rally stages or the circuit season (`campaign_data::set_kind` follows it)
    pub kind: CampaignKind,
    pub chapter: usize,
    pub stage: usize,
    /// Stage being driven (set when a campaign run starts)
    pub active: Option<StageDef>,
    pub car: usize,
    pub paint: usize,
    pub assists: usize,
    intro_id: String,
    intro_track: Option<Arc<Track>>,
    intro_rx: Option<Receiver<(String, Arc<Track>)>>,
    /// Background reference run (medal times) for a stage not timed yet
    ref_id: String,
    ref_rx: Option<Receiver<(String, Option<f32>)>>,
    /// Best run before the current attempt (time, splits)
    prev_best: Option<(f32, Vec<f32>)>,
    pub result: Option<UiResult>,
    pub lb_tab: usize,
    pub lb_filter: usize,
    pub lb_from: Screen,
}

impl CampState {
    pub fn new() -> CampState {
        CampState {
            kind: CampaignKind::Rally,
            chapter: 0,
            stage: 0,
            active: None,
            car: 2,
            paint: 0,
            assists: settings::get().assists,
            intro_id: String::new(),
            intro_track: None,
            intro_rx: None,
            ref_id: String::new(),
            ref_rx: None,
            prev_best: None,
            result: None,
            lb_tab: std::env::var("COZY_LB_TAB").ok().and_then(|v| v.parse().ok()).unwrap_or(0).min(1),
            lb_filter: 0,
            lb_from: Screen::Campaign,
        }
    }

    /// Switch campaign (Rally / Circuit): back to its first chapter, a car of its class
    pub fn set_kind(&mut self, k: CampaignKind) {
        self.kind = k;
        crate::campaign_data::set_kind(k);
        self.chapter = 0;
        self.stage = 0;
        if car_specs().get(self.car).is_none_or(|c| c.class != k.car_class()) {
            self.car = rally_core::campaign::class_cars(k.car_class()).first().copied().unwrap_or(0) as usize;
        }
    }

    pub fn selected(&self) -> Option<StageDef> {
        chapters().get(self.chapter).and_then(|c| c.stages.get(self.stage)).cloned()
    }

    /// Queue the selected stage with campaign_mode (config is picked up by finish_loading)
    pub fn begin(&mut self) -> bool {
        let Some(def) = self.selected() else { return false };
        self.prev_best = best_run(&def.id);
        let n = car_specs().len();
        let a = ASSISTS[self.assists.min(ASSISTS.len() - 1)];
        let ok = crate::campaign_mode::start_stage(&def.id, self.car.min(n - 1), self.paint % models::PAINTS.len(), (a.0, a.1)).is_some();
        if ok {
            self.active = Some(def);
        }
        ok
    }

    /// (best time, best splits, medal targets) for the stage HUD
    pub fn hud_reference(&self) -> (Option<f32>, Vec<f32>, Option<MedalTimes>) {
        let Some(def) = &self.active else { return (None, Vec::new(), None) };
        let mt = stage_view(&views(), &def.id).and_then(|v| v.medal_times);
        match &self.prev_best {
            Some((t, sp)) => (Some(*t), sp.clone(), mt),
            None => (None, Vec::new(), mt),
        }
    }

    /// Medal targets need a headless reference run (1-4 s) the first time: do it off-thread
    fn request_reference(&mut self, def: &StageDef, cached: bool) {
        if cached || self.ref_id == def.id {
            return;
        }
        self.ref_id = def.id.clone();
        let (tx, rx) = channel();
        let d = def.clone();
        std::thread::spawn(move || {
            let r = rally_core::campaign_run::reference_run(&d).map(|r| r.time);
            let _ = tx.send((d.id.clone(), r));
        });
        self.ref_rx = Some(rx);
    }

    fn poll_reference(&mut self) {
        if let Some(rx) = &self.ref_rx {
            if let Ok((id, t)) = rx.try_recv() {
                if let Some(t) = t {
                    crate::campaign_mode::cache_reference(&id, t);
                }
                self.ref_rx = None;
            }
        }
    }

    /// Generate the selected stage's track in the background (map + forecast on the intro)
    fn request_intro_track(&mut self, def: &StageDef) {
        if self.intro_id == def.id {
            return;
        }
        self.intro_id = def.id.clone();
        self.intro_track = None;
        let (tx, rx) = channel();
        let params = def.params.clone();
        let id = def.id.clone();
        std::thread::spawn(move || {
            let t = Arc::new(Track::generate(&params));
            let _ = tx.send((id, t));
        });
        self.intro_rx = Some(rx);
    }

    fn poll_intro_track(&mut self) {
        if let Some(rx) = &self.intro_rx {
            if let Ok((id, t)) = rx.try_recv() {
                if id == self.intro_id {
                    self.intro_track = Some(t);
                }
                self.intro_rx = None;
            }
        }
    }
}

// ---------------------------------------------------------------- drawing helpers

pub fn medal_color(m: Medal) -> Color {
    match m {
        Medal::None => Color::new(0.85, 0.82, 0.8, 1.0),
        Medal::Bronze => Color::new(0.88, 0.60, 0.42, 1.0),
        Medal::Silver => Color::new(0.80, 0.83, 0.90, 1.0),
        Medal::Gold => Color::new(1.0, 0.80, 0.30, 1.0),
        Medal::Platinum => Color::new(0.80, 0.92, 1.0, 1.0),
    }
}

pub fn medal_name(m: Medal) -> &'static str {
    match m {
        Medal::None => "No medal",
        Medal::Bronze => "Bronze",
        Medal::Silver => "Silver",
        Medal::Gold => "Gold",
        Medal::Platinum => "Platinum",
    }
}

fn star(cx: f32, cy: f32, r: f32, color: Color) {
    let pts: Vec<Vec2> = (0..10)
        .map(|i| {
            let a = -std::f32::consts::FRAC_PI_2 + i as f32 * std::f32::consts::PI / 5.0;
            let rr = if i % 2 == 0 { r } else { r * 0.45 };
            vec2(cx + a.cos() * rr, cy + a.sin() * rr)
        })
        .collect();
    for i in 0..10 {
        draw_triangle(vec2(cx, cy), pts[i], pts[(i + 1) % 10], color);
    }
}

/// A medal with a little ribbon. `Medal::None` draws an empty dashed slot.
pub fn draw_medal(cx: f32, cy: f32, r: f32, m: Medal, alpha: f32) {
    if m == Medal::None {
        for i in 0..12 {
            let a0 = i as f32 / 12.0 * std::f32::consts::TAU;
            arc(cx, cy, r * 0.82, r, a0, a0 + 0.3, with_alpha(INK, 0.18 * alpha));
        }
        return;
    }
    let c = medal_color(m);
    // ribbon
    draw_triangle(vec2(cx - r * 0.55, cy - r * 1.55), vec2(cx - r * 0.05, cy - r * 1.55), vec2(cx - r * 0.1, cy - r * 0.4), with_alpha(SKY, alpha));
    draw_triangle(vec2(cx + r * 0.55, cy - r * 1.55), vec2(cx + r * 0.05, cy - r * 1.55), vec2(cx + r * 0.1, cy - r * 0.4), with_alpha(PEACH, alpha));
    circle(cx, cy + r * 0.12, r, Color::new(0.2, 0.12, 0.25, 0.2 * alpha));
    circle(cx, cy, r, with_alpha(shade(c, 0.82), alpha));
    circle(cx, cy, r * 0.8, with_alpha(c, alpha));
    star(cx, cy, r * 0.48, with_alpha(lighten(c, 0.55), alpha));
    if m == Medal::Platinum {
        arc(cx, cy, r * 0.84, r * 0.92, -2.4, -1.2, with_alpha(LAVENDER, alpha));
    }
}

fn biome_color(ch: &Chapter) -> Color {
    use rally_core::biome::Biome;
    match ch.biome {
        Biome::Meadow => MINT,
        Biome::Autumn => PEACH,
        Biome::Coast => SKY,
        Biome::Desert => BUTTER,
        Biome::Snow => LAVENDER,
        Biome::Forest => MINT,
        Biome::Taiga => SKY,
    }
}

fn padlock(cx: f32, cy: f32, r: f32, color: Color) {
    arc(cx, cy - r * 0.25, r * 0.38, r * 0.55, std::f32::consts::PI, std::f32::consts::TAU, color);
    rounded_rect(cx - r * 0.7, cy - r * 0.3, r * 1.4, r * 1.05, r * 0.2, color);
    circle(cx, cy + r * 0.15, r * 0.16, CREAM);
}

/// Info chip: rounded pill with optional icon. Returns width.
fn chip(x: f32, cy: f32, label: &str, size: f32, bg: Color, icon: Option<WxKind>) -> f32 {
    let s = scale();
    let tw = text_size(label, size).x;
    let iw = if icon.is_some() { size * 1.3 } else { 0.0 };
    let w = tw + iw + 22.0 * s;
    let h = size * 1.7;
    rounded_rect(x, cy - h * 0.5, w, h, h * 0.5, bg);
    if let Some(k) = icon {
        hud::weather_icon(k, x + 10.0 * s + iw * 0.45, cy, size * 0.68);
    }
    text(label, x + 11.0 * s + iw, mid_baseline(cy, size), size, INK);
    w
}

fn km(def: &StageDef) -> String {
    if def.laps > 1 {
        format!("{} laps  -  {:.1} km", def.laps, stage_length_m(def) / 1000.0)
    } else {
        format!("{:.1} km", stage_length_m(def) / 1000.0)
    }
}

/// Info chips of an event card (after the distance): surface + difficulty for rally stages,
/// format (race / hot laps) + night for circuit events
fn event_chips(def: &StageDef) -> Vec<(String, Color)> {
    match def.kind {
        EventKind::Race { rivals, .. } => {
            let mut v = vec![(format!("Race  -  {rivals} rivals"), with_alpha(PEACH, 0.55))];
            if def.params.night {
                v.push(("Night".to_string(), with_alpha(LAVENDER, 0.6)));
            }
            v
        }
        EventKind::TimeTrial if def.params.discipline == rally_core::track::Discipline::Circuit => {
            let car = car_specs().get(def.cars.first().copied().unwrap_or(0) as usize).map(|c| c.name).unwrap_or("");
            let mut v = vec![(format!("Hot laps  -  {car}"), with_alpha(MINT, 0.55))];
            if def.params.night {
                v.push(("Night".to_string(), with_alpha(LAVENDER, 0.6)));
            }
            v
        }
        EventKind::TimeTrial => vec![(surface_name(def).to_string(), with_alpha(MINT, 0.55)), (difficulty_name(def.params.difficulty).to_string(), with_alpha(LAVENDER, 0.55))],
    }
}

/// What a medal needs in a race event
fn race_target(md: Medal, margin: f32) -> String {
    match md {
        Medal::Platinum => format!("win by {margin:.0}s"),
        Medal::Gold => "win".into(),
        Medal::Silver => "2nd".into(),
        _ => "3rd".into(),
    }
}

fn surface_name(def: &StageDef) -> &'static str {
    let i = rally_core::surfacemap::SURFACE_MODES.iter().position(|m| *m == def.params.surface).unwrap_or(0);
    ["Biome mix", "Gravel", "Tarmac", "Snow", "Mixed"][i.min(4)]
}

/// Small track silhouette (setup screen card): road line, start (and finish) dot. `alpha`
/// fades a stale preview while the new one is built.
pub(crate) fn draw_mini_map(track: &Track, x: f32, y: f32, w: f32, h: f32, alpha: f32) {
    let s = scale();
    let looped = track.is_loop();
    let i0 = track.start_index();
    let i1 = if looped { track.samples.len() } else { track.finish_index() };
    if i1 <= i0 + 2 {
        return;
    }
    let pts: Vec<Vec2> = (i0..=i1).step_by(3).map(|i| i % track.samples.len()).map(|i| vec2(track.samples[i].pos.x, track.samples[i].pos.z)).collect();
    let (mut min, mut max) = (vec2(f32::MAX, f32::MAX), vec2(f32::MIN, f32::MIN));
    for p in &pts {
        min = min.min(*p);
        max = max.max(*p);
    }
    let ext = max - min;
    let pad = 12.0 * s;
    let sc = ((w - 2.0 * pad) / ext.x.max(1.0)).min((h - 2.0 * pad) / ext.y.max(1.0));
    let off = vec2(x + (w - ext.x * sc) * 0.5, y + (h - ext.y * sc) * 0.5);
    let to = |p: Vec2| off + (p - min) * sc;
    for (wd, col) in [(5.0 * s, with_alpha(INK, 0.55 * alpha)), (2.6 * s, with_alpha(Color::new(1.0, 0.96, 0.88, 1.0), alpha))] {
        for i in 1..pts.len() {
            let (a, b) = (to(pts[i - 1]), to(pts[i]));
            draw_line(a.x, a.y, b.x, b.y, wd, col);
            circle(b.x, b.y, wd * 0.5, col);
        }
    }
    let a = to(pts[0]);
    circle(a.x, a.y, 5.0 * s, with_alpha(MINT, alpha));
    if !looped {
        let b = to(pts[pts.len() - 1]);
        circle(b.x, b.y, 5.0 * s, with_alpha(PEACH, alpha));
    }
}

/// Top-down stage silhouette from start to finish, tinted by weather, with split markers
pub(crate) fn draw_stage_map(track: &Track, x: f32, y: f32, w: f32, h: f32, reveal: f32) {
    let s = scale();
    // circuits: the whole loop, closed back to the start line
    let looped = track.is_loop();
    let i0 = track.start_index();
    let i1 = if looped { track.samples.len() } else { track.finish_index() };
    if i1 <= i0 + 2 {
        return;
    }
    let pts: Vec<(Vec2, f32)> = (i0..=i1).step_by(2).map(|i| i % track.samples.len()).map(|i| (vec2(track.samples[i].pos.x, track.samples[i].pos.z), track.samples[i].s)).collect();
    let mut min = vec2(f32::MAX, f32::MAX);
    let mut max = vec2(f32::MIN, f32::MIN);
    for (p, _) in &pts {
        min = min.min(*p);
        max = max.max(*p);
    }
    let ext = max - min;
    let sc = ((w - 60.0 * s) / ext.x.max(1.0)).min((h - 60.0 * s) / ext.y.max(1.0));
    let off = vec2(x + (w - ext.x * sc) * 0.5, y + (h - ext.y * sc) * 0.5);
    let to = |p: Vec2| off + (p - min) * sc;
    let n = ((pts.len() as f32) * reveal.clamp(0.0, 1.0)) as usize;
    for (wd, col) in [(12.0 * s, with_alpha(INK, 0.55)), (7.0 * s, Color::new(1.0, 0.96, 0.88, 1.0))] {
        for i in 1..n {
            let a = to(pts[i - 1].0);
            let b = to(pts[i].0);
            draw_line(a.x, a.y, b.x, b.y, wd, col);
            circle(b.x, b.y, wd * 0.5, col);
        }
    }
    // weather tint along the road
    for i in 1..n {
        let kind = classify(&track.weather_at(pts[i].1));
        if kind != WxKind::Clear && kind != WxKind::Cloudy {
            let a = to(pts[i - 1].0);
            let b = to(pts[i].0);
            draw_line(a.x, a.y, b.x, b.y, 5.0 * s, hud::weather_color(kind));
        }
    }
    // splits
    for (k, sp) in track.splits.iter().enumerate() {
        let (p, _, _, _) = track.frame_at(*sp);
        let q = to(vec2(p.x, p.z));
        if (sp - track.start_s) / track.stage_length() > reveal {
            continue;
        }
        circle(q.x, q.y, 11.0 * s, INK);
        circle(q.x, q.y, 9.0 * s, BUTTER);
        text_mid(&format!("{}", k + 1), q.x, q.y, 12.0 * s, INK);
    }
    let a = to(pts[0].0);
    circle(a.x, a.y, 14.0 * s, INK);
    circle(a.x, a.y, 11.5 * s, MINT);
    text_mid(if looped { "S" } else { "A" }, a.x, a.y, 14.0 * s, INK);
    if reveal >= 0.999 && !looped {
        let b = to(pts[pts.len() - 1].0);
        circle(b.x, b.y, 14.0 * s, INK);
        circle(b.x, b.y, 11.5 * s, PEACH);
        text_mid("B", b.x, b.y, 14.0 * s, INK);
    }
}

/// Elevation profile + weather timeline under the map
pub(crate) fn draw_profile(track: &Track, x: f32, y: f32, w: f32, h: f32) {
    let s = scale();
    let i0 = track.start_index();
    let i1 = if track.is_loop() { track.samples.len() - 1 } else { track.finish_index().max(i0 + 2) };
    let hs: Vec<f32> = (i0..=i1).map(|i| track.samples[i].pos.y).collect();
    let lo = hs.iter().cloned().fold(f32::MAX, f32::min);
    let hi = hs.iter().cloned().fold(f32::MIN, f32::max).max(lo + 1.0);
    let cols = 80;
    for c in 0..cols {
        let f = c as f32 / cols as f32;
        let i = ((f * (hs.len() - 1) as f32) as usize).min(hs.len() - 1);
        let v = (hs[i] - lo) / (hi - lo);
        let bh = (h * 0.25 + v * h * 0.55).max(2.0);
        let sval = if track.is_loop() { f * track.length } else { track.start_s + f * track.stage_length() };
        let kind = classify(&track.weather_at(sval));
        let col = if kind == WxKind::Clear { with_alpha(MINT, 0.8) } else { mix(with_alpha(MINT, 0.8), hud::weather_color(kind), 0.7) };
        draw_rectangle(x + f * w, y + h - bh, w / cols as f32 + 0.6, bh, col);
    }
    // event icons
    for (s0, s1, e) in &track.weather_events {
        let f = (((s0 + s1) * 0.5 - track.start_s) / track.stage_length()).clamp(0.02, 0.98);
        let cx = x + f * w;
        let kind = event_kind(*e);
        circle(cx, y - 4.0 * s, 15.0 * s, with_alpha(hud::weather_color(kind), 0.4));
        hud::weather_icon(kind, cx, y - 4.0 * s, 11.0 * s);
    }
    text("Start", x, y + h + 18.0 * s, 15.0 * s, INK_SOFT);
    text_mid("elevation + weather", x + w * 0.5, y + h + 13.0 * s, 14.0 * s, INK_SOFT);
    text_right("Finish", x + w, y + h + 18.0 * s, 15.0 * s, INK_SOFT);
}

// ---------------------------------------------------------------- screens

impl App {
    pub(crate) fn campaign_screen(&mut self) {
        self.menu_backdrop();
        screen_wash(Color::new(1.0, 0.96, 0.9, 0.35), Color::new(1.0, 0.94, 0.88, 0.5));
        let s = scale();
        let sw = screen_width();
        let sh = screen_height();
        let t = self.screen_t;
        let chs = chapters();
        let vs = views();
        let golds = golds(&vs);
        // header
        let m = 24.0 * s;
        text_outlined("Campaign", m + 6.0 * s, m + 46.0 * s, 46.0 * s, CREAM);
        let kind = self.camp.kind;
        let sub = match kind {
            CampaignKind::Rally => "Time-trial stages from A to B. Earn golds to open new chapters.",
            CampaignKind::Circuit => "Races and hot laps on the circuits. Win for gold, and keep an eye on the sky.",
        };
        text(sub, m + 8.0 * s, m + 76.0 * s, 18.0 * s, INK);
        let tally = format!("{golds}");
        let label = if golds == 1 { "gold medal" } else { "gold medals" };
        let tw = text_size(&tally, 26.0 * s).x + text_size(label, 16.0 * s).x + 80.0 * s;
        rounded_rect(sw - m - tw, m + 8.0 * s, tw, 50.0 * s, 25.0 * s, with_alpha(CREAM, 0.95));
        draw_medal(sw - m - tw + 28.0 * s, m + 37.0 * s, 12.0 * s, Medal::Gold, 1.0);
        text(&tally, sw - m - tw + 50.0 * s, mid_baseline(m + 33.0 * s, 26.0 * s), 26.0 * s, INK);
        text_right(label, sw - m - 18.0 * s, mid_baseline(m + 33.0 * s, 16.0 * s), 16.0 * s, INK_SOFT);

        let n_ch = chs.len();
        self.focus.begin(n_ch + 3 + 1 + 1);
        // chapter cards (left)
        let lx = m;
        let lw = (sw * 0.30).clamp(300.0 * s, 380.0 * s);
        let top = m + 104.0 * s;
        let back_h = 50.0 * s;
        let ch_h = ((sh - top - back_h - 2.0 * m) / n_ch as f32 - 10.0 * s).min(96.0 * s);
        let mut enter_stages = false;
        for (i, ch) in chs.iter().enumerate() {
            let k = ease_back(((t - 0.05 * i as f32) / 0.35).clamp(0.0, 1.0));
            let y = top + i as f32 * (ch_h + 10.0 * s);
            let x = lx - (1.0 - k) * 40.0 * s;
            let id = self.focus.peek_next();
            let (hl, act) = focus_item(&mut self.focus, x, y, lw, ch_h);
            if self.focus.index == id && self.camp.chapter != i {
                self.camp.chapter = i;
                self.camp.stage = 0;
            }
            if act {
                enter_stages = true;
            }
            let unlocked = unlocked(&vs, i);
            let sel = self.camp.chapter == i;
            let col = biome_color(ch);
            let base = if unlocked { col } else { mix(col, Color::new(0.8, 0.78, 0.8, 1.0), 0.7) };
            soft_shadow(x, y, lw, ch_h, 16.0 * s, 0.7);
            rounded_rect_gradient(x, y, lw, ch_h, 16.0 * s, lighten(base, 0.45), lighten(base, 0.15));
            if sel || hl > 0.01 {
                rounded_rect_outline(x - 3.0 * s, y - 3.0 * s, lw + 6.0 * s, ch_h + 6.0 * s, 19.0 * s, 3.0 * s, Color::new(1.0, 1.0, 1.0, if sel { 1.0 } else { hl }));
            }
            let r = ch_h * 0.3;
            circle(x + 16.0 * s + r, y + ch_h * 0.5, r, with_alpha(CREAM, 0.9));
            if unlocked {
                text_mid(&format!("{}", i + 1), x + 16.0 * s + r, y + ch_h * 0.5, r * 1.1, INK);
            } else {
                padlock(x + 16.0 * s + r, y + ch_h * 0.5, r * 0.55, INK_SOFT);
            }
            let tx = x + 30.0 * s + r * 2.0;
            text(&ch.name, tx, y + ch_h * 0.42, 22.0 * s, if unlocked { INK } else { INK_SOFT });
            if unlocked {
                for (j, st) in ch.stages.iter().enumerate() {
                    let md = stage_view(&vs, &st.id).map(|v| v.medal).unwrap_or(Medal::None);
                    let _ = j;
                    draw_medal(tx + 10.0 * s + j as f32 * 26.0 * s, y + ch_h * 0.72, 8.5 * s, md, 1.0);
                }
            } else {
                text(&format!("Needs {} golds", ch.unlock_golds), tx, y + ch_h * 0.75, 16.0 * s, INK_SOFT);
            }
        }
        // stage cards (right)
        let rx = lx + lw + 24.0 * s;
        let rw = sw - rx - m;
        let ch = &chs[self.camp.chapter.min(n_ch - 1)];
        let unlocked = unlocked(&vs, self.camp.chapter.min(n_ch - 1));
        let hy = top;
        text_outlined(&ch.name, rx + 4.0 * s, hy + 30.0 * s, 32.0 * s, CREAM);
        let things = if kind == CampaignKind::Circuit { "events" } else { "stages" };
        let sub = if unlocked { format!("Chapter {}  -  {} {things}", self.camp.chapter + 1, ch.stages.len()) } else { format!("Locked  -  earn {} golds to open ({} so far)", ch.unlock_golds, golds) };
        text(&sub, rx + 6.0 * s, hy + 56.0 * s, 17.0 * s, INK);
        let st_top = hy + 72.0 * s;
        let n_st = ch.stages.len().max(1);
        let st_h = ((sh - st_top - m) / n_st as f32 - 12.0 * s).min(160.0 * s);
        let mut open_stage = None;
        for (j, st) in ch.stages.iter().enumerate() {
            let k = ease_out(((t - 0.15 - 0.07 * j as f32) / 0.35).clamp(0.0, 1.0));
            let y = st_top + j as f32 * (st_h + 12.0 * s) + (1.0 - k) * 30.0 * s;
            let id = self.focus.peek_next();
            let (hl, act) = focus_item(&mut self.focus, rx, y, rw, st_h);
            if self.focus.index == id {
                self.camp.stage = j;
            }
            if act && unlocked {
                open_stage = Some(j);
            }
            let a = if unlocked { 1.0 } else { 0.55 };
            soft_shadow(rx, y, rw, st_h, 18.0 * s, a);
            rounded_rect_gradient(rx, y, rw, st_h, 18.0 * s, with_alpha(Color::new(1.0, 0.99, 0.96, 1.0), 0.96 * a + 0.04), with_alpha(CREAM, 0.96 * a + 0.04));
            rounded_rect(rx, y, 10.0 * s, st_h, 5.0 * s, biome_color(ch));
            if hl > 0.01 {
                rounded_rect_outline(rx - 3.0 * s, y - 3.0 * s, rw + 6.0 * s, st_h + 6.0 * s, 21.0 * s, 3.0 * s, Color::new(1.0, 1.0, 1.0, hl));
            }
            let px = rx + 28.0 * s;
            text(&format!("{}.{}", self.camp.chapter + 1, j + 1), px, y + 40.0 * s, 18.0 * s, INK_SOFT);
            text(&st.name, px + 44.0 * s, y + 40.0 * s, 28.0 * s, INK);
            text(&st.blurb, px, y + 68.0 * s, 17.0 * s, INK_SOFT);
            let cy = y + st_h - 32.0 * s;
            let mut cx = px;
            let (wx, hint) = forecast(st);
            cx += chip(cx, cy, &km(st), 16.0 * s, with_alpha(BUTTER, 0.6), None) + 8.0 * s;
            for (label, col) in event_chips(st) {
                cx += chip(cx, cy, &label, 16.0 * s, col, None) + 8.0 * s;
            }
            chip(cx, cy, &hint, 16.0 * s, with_alpha(SKY, 0.45), Some(wx));
            // medal + times on the right
            let mx = rx + rw - 70.0 * s;
            let v = stage_view(&vs, &st.id);
            let medal = v.map(|v| v.medal).unwrap_or(Medal::None);
            draw_medal(mx, y + st_h * 0.5 + 6.0 * s, 26.0 * s, medal, a);
            let best = v.and_then(|v| v.best);
            let gold = if st.is_race() { "Win".to_string() } else { v.and_then(|v| v.medal_times).map(|m| format_time(m.gold)).unwrap_or_else(|| "\u{2014}".into()) };
            let tx = mx - 50.0 * s;
            text_right(&format!("Best  {}", best.map(format_time).unwrap_or_else(|| "--:--".into())), tx, y + 46.0 * s, 18.0 * s, INK);
            text_right(&format!("Gold  {gold}"), tx, y + 70.0 * s, 16.0 * s, shade(BUTTER, 0.7));
            if !unlocked {
                padlock(rx + rw * 0.5, y + st_h * 0.5, 16.0 * s, with_alpha(INK, 0.5));
            }
        }
        let back = button_back(&mut self.focus, "Back", lx, sh - m - back_h, lw, back_h, CREAM);
        // Rally / Circuit season switch (top right, left of the gold tally)
        {
            let tab_w = 128.0 * s;
            let th = 44.0 * s;
            let tx0 = sw - m - tw - 2.0 * tab_w - 26.0 * s;
            let ty = m + 11.0 * s;
            let (hl, act) = focus_item(&mut self.focus, tx0, ty, tab_w * 2.0 + 8.0 * s, th);
            let focused = hl > 0.5;
            let mut pick = None;
            if act {
                let (mx, _) = mouse_position();
                pick = Some(if is_mouse_button_pressed(MouseButton::Left) { if mx < tx0 + tab_w + 4.0 * s { 0 } else { 1 } } else { 1 - (kind as usize) });
            } else if focused && (self.focus.left || self.focus.right) {
                pick = Some(1 - (kind as usize));
            }
            for (i, k) in CampaignKind::ALL.iter().enumerate() {
                let x = tx0 + i as f32 * (tab_w + 8.0 * s);
                let on = *k == kind;
                soft_shadow(x, ty, tab_w, th, th * 0.5, 0.5);
                rounded_rect(x, ty, tab_w, th, th * 0.5, if on { if i == 0 { MINT } else { SKY } } else { with_alpha(CREAM, 0.92) });
                if on && hl > 0.01 {
                    rounded_rect_outline(x - 2.5 * s, ty - 2.5 * s, tab_w + 5.0 * s, th + 5.0 * s, th * 0.5 + 2.5 * s, 2.5 * s, Color::new(1.0, 1.0, 1.0, hl));
                }
                text_mid(k.name(), x + tab_w * 0.5, ty + th * 0.5, 20.0 * s, if on { INK } else { INK_SOFT });
            }
            if let Some(i) = pick {
                if CampaignKind::ALL[i] != kind {
                    self.camp.set_kind(CampaignKind::ALL[i]);
                    audio::ui_sound(audio::UiSound::Move);
                }
            }
        }
        if enter_stages && unlocked {
            self.focus.index = n_ch;
        }
        if let Some(j) = open_stage {
            self.camp.stage = j;
            self.goto(Screen::StageIntro);
        } else if back || self.focus.back {
            if self.focus.back {
                audio::ui_sound(audio::UiSound::Back);
            }
            self.goto(Screen::Title);
        }
    }

    pub(crate) fn stage_intro(&mut self) {
        self.menu_backdrop();
        screen_wash(Color::new(1.0, 0.96, 0.9, 0.35), Color::new(1.0, 0.94, 0.88, 0.5));
        let Some(def) = self.camp.selected() else {
            self.switch_now(Screen::Campaign);
            return;
        };
        self.camp.request_intro_track(&def);
        self.camp.poll_intro_track();
        self.camp.poll_reference();
        let vs = views();
        let view = stage_view(&vs, &def.id);
        let mt = view.and_then(|v| v.medal_times);
        // races award medals by place: no reference run needed
        self.camp.request_reference(&def, mt.is_some() || def.is_race());
        let s = scale();
        let sw = screen_width();
        let sh = screen_height();
        let m = 24.0 * s;
        let appear = ease_back((self.screen_t / 0.4).min(1.0));
        let lw = (sw * 0.46).max(500.0 * s).min(sw - 2.0 * m);
        let lx = m - (1.0 - appear) * 40.0 * s;
        let h = sh - 2.0 * m;
        panel(lx, m, lw, h);
        let px = lx + 26.0 * s;
        let chs = chapters();
        let ch = &chs[self.camp.chapter];
        let what = if def.params.discipline == rally_core::track::Discipline::Circuit { "Event" } else { "Stage" };
        text(&format!("{}  -  {what} {}", ch.name, self.camp.stage + 1), px, m + 34.0 * s, 17.0 * s, INK_SOFT);
        text(&def.name, px, m + 76.0 * s, 42.0 * s, INK);
        text(&def.blurb, px, m + 104.0 * s, 17.0 * s, INK_SOFT);
        let cy = m + 136.0 * s;
        let mut cx = px;
        cx += chip(cx, cy, &km(&def), 16.0 * s, with_alpha(BUTTER, 0.6), None) + 8.0 * s;
        for (label, col) in event_chips(&def) {
            cx += chip(cx, cy, &label, 16.0 * s, col, None) + 8.0 * s;
        }
        let (wx, hint) = forecast(&def);
        chip(px, cy + 36.0 * s, &format!("Forecast: {hint}"), 16.0 * s, with_alpha(SKY, 0.45), Some(wx));
        // medal targets
        let best = view.and_then(|v| v.best);
        let earned = view.map(|v| v.medal).unwrap_or(Medal::None);
        let my = cy + 70.0 * s;
        let rows = [Medal::Platinum, Medal::Gold, Medal::Silver, Medal::Bronze].map(|md| {
            let t = mt.map(|m| match md {
                Medal::Platinum => m.platinum,
                Medal::Gold => m.gold,
                Medal::Silver => m.silver,
                _ => m.bronze,
            });
            (md, t)
        });
        let colw = (lw - 52.0 * s) * 0.5;
        for (i, (md, tm)) in rows.iter().enumerate() {
            let rx = px + (i % 2) as f32 * colw;
            let ry = my + (i / 2) as f32 * 40.0 * s;
            let got = earned >= *md;
            rounded_rect(rx, ry, colw - 10.0 * s, 34.0 * s, 17.0 * s, if got { with_alpha(medal_color(*md), 0.45) } else { with_alpha(CREAM_DEEP, 0.7) });
            draw_medal(rx + 18.0 * s, ry + 19.0 * s, 9.0 * s, *md, 1.0);
            text(medal_name(*md), rx + 36.0 * s, mid_baseline(ry + 17.0 * s, 16.0 * s), 16.0 * s, INK);
            match tm {
                _ if def.is_race() => text_right(&race_target(*md, def.platinum_margin()), rx + colw - 22.0 * s, mid_baseline(ry + 17.0 * s, 16.0 * s), 16.0 * s, INK),
                Some(tm) => text_right(&format_time(*tm), rx + colw - 22.0 * s, mid_baseline(ry + 17.0 * s, 16.0 * s), 16.0 * s, INK),
                None => {
                    let a = get_time() as f32 * 5.0 + i as f32;
                    arc(rx + colw - 34.0 * s, ry + 17.0 * s, 5.0 * s, 8.0 * s, a, a + 4.0, with_alpha(INK, 0.4));
                }
            }
        }
        let by0 = my + 104.0 * s;
        text(&format!("Your best:  {}", best.map(format_time).unwrap_or_else(|| "not driven yet".into())), px, by0, 18.0 * s, INK);
        if mt.is_none() && !def.is_race() {
            text_right("timing the reference run...", px + lw - 52.0 * s, by0, 15.0 * s, INK_SOFT);
        }
        // car / paint / assists
        let specs = car_specs();
        let allowed: Vec<usize> = (0..specs.len()).filter(|c| def.cars.is_empty() || def.cars.contains(&(*c as u8))).collect();
        if !allowed.contains(&self.camp.car) {
            self.camp.car = allowed[0];
        }
        self.focus.begin(6);
        let rh = 44.0 * s;
        let step = rh + 8.0 * s;
        let mut ry = by0 + 18.0 * s;
        let rw = lw - 52.0 * s;
        let pos = allowed.iter().position(|c| *c == self.camp.car).unwrap_or(0);
        let d = selector(&mut self.focus, "Car", specs[self.camp.car].name, px, ry, rw, rh, PEACH);
        self.camp.car = allowed[((pos as i32 + d).rem_euclid(allowed.len() as i32)) as usize];
        ry += step;
        let paint = self.camp.paint % models::PAINTS.len();
        let d = selector_swatch(&mut self.focus, "Paint", crate::paint_name(paint), Some(rgb3(models::PAINTS[paint])), px, ry, rw, rh, PEACH);
        self.camp.paint = ((paint as i32 + d).rem_euclid(models::PAINTS.len() as i32)) as usize;
        ry += step;
        let d = selector(&mut self.focus, "Assists", ASSISTS[self.camp.assists].0, px, ry, rw, rh, LAVENDER);
        self.camp.assists = ((self.camp.assists as i32 + d).rem_euclid(ASSISTS.len() as i32)) as usize;
        let bh = 52.0 * s;
        let byb = m + h - bh - 22.0 * s;
        let bw = (rw - 20.0 * s) / 3.0;
        let back = button_back(&mut self.focus, "Back", px, byb, bw, bh, CREAM);
        let lb = button(&mut self.focus, "Leaderboard", px + bw + 10.0 * s, byb, bw, bh, SKY);
        let go = button(&mut self.focus, "Start!", px + 2.0 * (bw + 10.0 * s), byb, bw, bh, PEACH);
        // map panel (right)
        let mx = lx + lw + m;
        let mw = sw - mx - m;
        if mw > 200.0 * s {
            let mx = mx + (1.0 - appear) * 60.0 * s;
            panel(mx, m, mw, h);
            text(if def.params.layout == rally_core::track::LayoutKind::Loop { "Circuit map" } else { "Stage map" }, mx + 22.0 * s, m + 34.0 * s, 18.0 * s, INK_SOFT);
            let prof_h = 70.0 * s;
            let map_h = h - prof_h - 110.0 * s;
            rounded_rect(mx + 16.0 * s, m + 48.0 * s, mw - 32.0 * s, map_h, 16.0 * s, with_alpha(lighten(biome_color(ch), 0.55), 0.8));
            match &self.camp.intro_track {
                Some(tr) => {
                    let reveal = ((self.screen_t - 0.2) / 1.2).clamp(0.0, 1.0);
                    draw_stage_map(tr, mx + 16.0 * s, m + 48.0 * s, mw - 32.0 * s, map_h, ease_out(reveal));
                    draw_profile(tr, mx + 30.0 * s, m + 48.0 * s + map_h + 30.0 * s, mw - 60.0 * s, prof_h);
                }
                None => {
                    let c = vec2(mx + mw * 0.5, m + 48.0 * s + map_h * 0.5);
                    let a = get_time() as f32 * 4.0;
                    arc(c.x, c.y, 16.0 * s, 22.0 * s, a, a + 4.2, with_alpha(INK, 0.4));
                    text_mid("Surveying the stage...", c.x, c.y + 44.0 * s, 18.0 * s, INK_SOFT);
                }
            }
        }
        if go && self.camp.begin() {
            settings::update(|st| st.assists = self.camp.assists);
            self.start_race();
        } else if lb {
            self.camp.lb_from = Screen::StageIntro;
            self.goto(Screen::Leaderboard);
        } else if back || self.focus.back {
            if self.focus.back {
                audio::ui_sound(audio::UiSound::Back);
            }
            self.goto(Screen::Campaign);
        }
    }

    /// Called when a campaign stage run ends: store it and show the stage results
    pub(crate) fn stage_over(&mut self) {
        let Some(def) = self.camp.active.clone() else { return };
        let Some(sess) = self.session.as_ref() else { return };
        let car = sess.players[0].car;
        // campaign_mode::frame recorded the run on the Finish event (progress, board, ghost)
        let Some(r) = crate::campaign_mode::last_result() else {
            self.goto(Screen::Campaign);
            return;
        };
        let medal_times = stage_view(&views(), &def.id).and_then(|v| v.medal_times);
        let best_splits = self.camp.prev_best.as_ref().map(|b| b.1.clone()).unwrap_or_default();
        self.camp.result = Some(UiResult { r, best_splits, medal_times });
        self.results_cine = crate::cinematic::Cinematic::orbit(car);
        self.results_cine.frame = vec2(0.42, -0.30);
        self.switch_now(Screen::StageResults);
        self.set_music(audio::Music::Results);
    }

    pub(crate) fn stage_results(&mut self) {
        let Some(sess) = self.session.as_mut() else {
            self.switch_now(Screen::Campaign);
            return;
        };
        let dt = get_frame_time().min(0.05);
        sess.update(dt, None);
        self.results_cine.update(sess, dt);
        let horizon = sess.scene.sky_horizon;
        clear_background(Color::new(horizon[0], horizon[1], horizon[2], 1.0));
        sess.draw_3d(&mut self.gpu, |_, _, _| {});
        let Some(ui_res) = self.camp.result.clone() else { return };
        let res = &ui_res.r;
        let def = self.camp.active.clone().unwrap_or_else(|| self.camp.selected().unwrap());
        let me = sess.players.first().map(|p| p.car).unwrap_or(0);
        let place = sess.world.race.racers.get(me).map(|r| r.place).unwrap_or(1);
        let field = sess.world.cars.len();
        let s = scale();
        let sw = screen_width();
        let sh = screen_height();
        let t = self.screen_t;
        let fade = ease_out((t / 0.5).min(1.0));
        rounded_rect_gradient(0.0, 0.0, sw, sh, 0.0, Color::new(0.24, 0.18, 0.3, 0.08 * fade), Color::new(0.24, 0.18, 0.3, 0.38 * fade));
        let first_run = res.previous_best.is_none();
        let record = res.new_best && !first_run;
        if record || res.medal >= Medal::Gold {
            crate::confetti(t);
        }
        let m = 24.0 * s;
        // left: time + medal
        let lw = (sw * 0.42).min(560.0 * s);
        let lh = sh - 2.0 * m - 76.0 * s;
        let k = ease_back((t / 0.4).min(1.0));
        let lx = m - (1.0 - k) * 40.0 * s;
        panel(lx, m, lw, lh);
        let cx = lx + lw * 0.5;
        text_mid(&def.name, cx, m + 40.0 * s, 26.0 * s, INK_SOFT);
        let done = if def.is_race() { format!("Finished P{place} of {field}") } else if def.laps > 1 { "Laps complete".to_string() } else { "Stage complete".to_string() };
        text_mid(&done, cx, m + 74.0 * s, 20.0 * s, INK_SOFT);
        let tp = pop((t - 0.2) * 1.2);
        text_sticker_centered(&format_time(res.time), cx, m + 140.0 * s, (64.0 * s * tp).max(1.0), BUTTER, 1.0);
        // medal drop
        let md_t = ((t - 0.7) / 0.6).clamp(0.0, 1.0);
        let my = m + 150.0 * s + (lh - 260.0 * s) * 0.5 + 40.0 * s;
        let drop = (1.0 - ease_back(md_t)) * -120.0 * s;
        if md_t > 0.0 {
            if res.medal != Medal::None {
                for i in 0..12 {
                    let a = i as f32 / 12.0 * std::f32::consts::TAU + get_time() as f32 * 0.4;
                    let r0 = 70.0 * s;
                    let r1 = 120.0 * s;
                    draw_triangle(vec2(cx + a.cos() * r0 * 0.6, my + a.sin() * r0 * 0.6), vec2(cx + (a - 0.12).cos() * r1, my + (a - 0.12).sin() * r1), vec2(cx + (a + 0.12).cos() * r1, my + (a + 0.12).sin() * r1), with_alpha(medal_color(res.medal), 0.25 * md_t));
                }
            }
            draw_medal(cx, my + drop, 52.0 * s, res.medal, md_t.min(1.0));
            let label = if res.medal == Medal::None { "No medal this time".to_string() } else { format!("{} medal!", medal_name(res.medal)) };
            text_mid(&label, cx, my + 84.0 * s, 26.0 * s * md_t.max(0.01), INK);
        }
        // previous best / delta / next target
        let iy = m + lh - 64.0 * s;
        if let Some(pb) = res.previous_best {
            let d = res.time - pb;
            let col = if d <= 0.0 { shade(MINT, 0.7) } else { BERRY };
            text_mid(&format!("Previous best {}   ({:+.2})", format_time(pb), d), cx, iy, 18.0 * s, col);
        }
        let next = ui_res.medal_times.and_then(|m| [(Medal::Bronze, m.bronze), (Medal::Silver, m.silver), (Medal::Gold, m.gold), (Medal::Platinum, m.platinum)].into_iter().find(|(md, _)| *md > res.medal));
        if def.is_race() {
            if let Some(md) = [Medal::Bronze, Medal::Silver, Medal::Gold, Medal::Platinum].into_iter().find(|md| *md > res.medal) {
                text_mid(&format!("{}: {}", medal_name(md), race_target(md, def.platinum_margin())), cx, iy + 26.0 * s, 16.0 * s, INK_SOFT);
            }
        } else if let Some((md, tm)) = next {
            text_mid(&format!("{} needs {}  ({:.1}s faster)", medal_name(md), format_time(tm), res.time - tm), cx, iy + 26.0 * s, 16.0 * s, INK_SOFT);
        }
        if (record || first_run) && t > 1.2 {
            let rk = ease_back(((t - 1.2) / 0.4).min(1.0));
            let (label, col) = if record { ("New record!", MINT) } else { ("First time! Benchmark set", SKY) };
            let rw = (text_size(label, 20.0 * s).x + 40.0 * s) * rk;
            rounded_rect(cx - rw * 0.5, m + 168.0 * s, rw, 34.0 * s, 17.0 * s, col);
            if rk > 0.7 {
                text_mid(label, cx, m + 185.0 * s, 20.0 * s, INK);
            }
        }
        // right: splits table
        let tx = lx + lw + m;
        let tw = sw - tx - m;
        let n = res.splits.len() + 1;
        let row_h = (34.0 * s).min((lh - 110.0 * s) / n as f32);
        let th = 70.0 * s + n as f32 * row_h + 20.0 * s;
        let slide = ease_out(((t - 0.3) / 0.5).clamp(0.0, 1.0));
        let tx = tx + (1.0 - slide) * 60.0 * s;
        panel(tx, m, tw, th);
        text("Splits", tx + 22.0 * s, m + 36.0 * s, 22.0 * s, INK);
        let cols = [tx + 22.0 * s, tx + tw * 0.38, tx + tw * 0.64, tx + tw - 22.0 * s];
        for (i, hd) in ["", "Time", "Best", "Delta"].iter().enumerate() {
            if i == 3 {
                text_right(hd, cols[3], m + 62.0 * s, 15.0 * s, INK_SOFT);
            } else {
                text(hd, cols[i], m + 62.0 * s, 15.0 * s, INK_SOFT);
            }
        }
        for i in 0..n {
            let ry = m + 72.0 * s + i as f32 * row_h;
            let ra = ((t - 0.5 - i as f32 * 0.08) / 0.3).clamp(0.0, 1.0);
            if ra <= 0.0 {
                continue;
            }
            let fin = i == n - 1;
            let mine = if fin { Some(res.time) } else { res.splits.get(i).copied() };
            let best = if fin { res.previous_best } else { ui_res.best_splits.get(i).copied() };
            if i % 2 == 1 {
                rounded_rect(tx + 10.0 * s, ry + 2.0 * s, tw - 20.0 * s, row_h - 4.0 * s, 10.0 * s, with_alpha(CREAM_DEEP, 0.5 * ra));
            }
            let mid = ry + row_h * 0.5;
            let fs = 18.0 * s;
            text(&if fin { "Finish".to_string() } else { format!("Split {}", i + 1) }, cols[0], mid_baseline(mid, fs), fs, with_alpha(INK, ra));
            text(&mine.map(format_time).unwrap_or_else(|| "-".into()), cols[1], mid_baseline(mid, fs), fs, with_alpha(INK, ra));
            text(&best.map(format_time).unwrap_or_else(|| "-".into()), cols[2], mid_baseline(mid, fs), fs, with_alpha(INK_SOFT, ra));
            if let (Some(a), Some(b)) = (mine, best) {
                let d = ((a - b) * 100.0).round() / 100.0;
                let col = if d < 0.0 { MINT } else if d > 0.0 { PEACH } else { CREAM_DEEP };
                let label = format!("{:+.2}", d);
                let w = text_size(&label, fs * 0.9).x + 18.0 * s;
                rounded_rect(cols[3] - w, mid - fs * 0.65, w, fs * 1.3, fs * 0.65, with_alpha(col, ra));
                text_right(&label, cols[3] - 9.0 * s, mid_baseline(mid, fs * 0.9), fs * 0.9, with_alpha(INK, ra));
            }
        }
        text(&format!("#{} on your board", res.rank_local), tx + 22.0 * s, m + th + 30.0 * s, 18.0 * s, CREAM);
        // buttons
        let bh = 52.0 * s;
        let by = sh - m - bh;
        let bw = (sw - 2.0 * m - 3.0 * 14.0 * s) / 4.0;
        self.focus.begin(4);
        let retry = button(&mut self.focus, "Retry", m, by, bw, bh, PEACH);
        let next_label = if def.params.discipline == rally_core::track::Discipline::Circuit { "Next event" } else { "Next stage" };
        let next_st = button(&mut self.focus, next_label, m + bw + 14.0 * s, by, bw, bh, MINT);
        let lb = button(&mut self.focus, "Leaderboard", m + 2.0 * (bw + 14.0 * s), by, bw, bh, SKY);
        let camp = button_back(&mut self.focus, "Campaign", m + 3.0 * (bw + 14.0 * s), by, bw, bh, BUTTER);
        if retry && self.camp.begin() {
            self.start_race();
        } else if next_st {
            let chs = chapters();
            let (mut c, mut st) = (self.camp.chapter, self.camp.stage + 1);
            if st >= chs[c].stages.len() {
                c += 1;
                st = 0;
            }
            if c < chs.len() && unlocked(&views(), c) {
                self.camp.chapter = c;
                self.camp.stage = st;
                self.goto(Screen::StageIntro);
            } else {
                self.goto(Screen::Campaign);
            }
        } else if lb {
            self.camp.lb_from = Screen::StageIntro;
            self.goto(Screen::Leaderboard);
        } else if camp || self.focus.back {
            self.goto(Screen::Campaign);
        }
    }

    pub(crate) fn leaderboard_screen(&mut self) {
        self.menu_backdrop();
        screen_wash(Color::new(1.0, 0.96, 0.9, 0.35), Color::new(1.0, 0.94, 0.88, 0.5));
        let s = scale();
        let sw = screen_width();
        let sh = screen_height();
        let w = (980.0 * s).min(sw - 40.0 * s);
        let h = sh - 48.0 * s;
        let k = ease_back((self.screen_t / 0.35).min(1.0));
        let x = (sw - w) * 0.5;
        let y = 24.0 * s + (1.0 - k) * 30.0 * s;
        panel(x, y, w, h);
        text("Leaderboard", x + 28.0 * s, y + 50.0 * s, 38.0 * s, INK);
        let all: Vec<(usize, usize, StageDef)> = chapters().into_iter().enumerate().flat_map(|(ci, c)| c.stages.into_iter().enumerate().map(move |(si, d)| (ci, si, d))).collect();
        let cur = all.iter().position(|(c, st, _)| *c == self.camp.chapter && *st == self.camp.stage).unwrap_or(0);
        self.focus.begin(4);
        let rh = 44.0 * s;
        let colw = (w - 3.0 * 28.0 * s) * 0.5;
        let d = selector(&mut self.focus, "Stage", &all[cur].2.name, x + 28.0 * s, y + 76.0 * s, colw, rh, BUTTER);
        if d != 0 {
            let n = ((cur as i32 + d).rem_euclid(all.len() as i32)) as usize;
            self.camp.chapter = all[n].0;
            self.camp.stage = all[n].1;
        }
        let def = all[((cur as i32 + d).rem_euclid(all.len() as i32)) as usize].2.clone();
        let filters = ["All assists", "Cozy", "Normal", "Sport", "Sim"];
        let d = selector(&mut self.focus, "Show", filters[self.camp.lb_filter], x + 56.0 * s + colw, y + 76.0 * s, colw, rh, LAVENDER);
        self.camp.lb_filter = ((self.camp.lb_filter as i32 + d).rem_euclid(filters.len() as i32)) as usize;
        // tabs
        let ty = y + 132.0 * s;
        let tab_w = 150.0 * s;
        let (hl, act) = focus_item(&mut self.focus, x + 28.0 * s, ty, tab_w * 2.0 + 8.0 * s, 40.0 * s);
        let tabs_focused = hl > 0.5;
        if act || (tabs_focused && (self.focus.left || self.focus.right)) {
            // clicking picks the tab under the cursor; keys toggle
            let (mx, _) = mouse_position();
            self.camp.lb_tab = if act && is_mouse_button_pressed(MouseButton::Left) { if mx < x + 28.0 * s + tab_w + 4.0 * s { 0 } else { 1 } } else { 1 - self.camp.lb_tab };
            audio::ui_sound(audio::UiSound::Move);
        }
        for (i, label) in ["Local", "Global"].iter().enumerate() {
            let tx = x + 28.0 * s + i as f32 * (tab_w + 8.0 * s);
            let on = self.camp.lb_tab == i;
            rounded_rect(tx, ty, tab_w, 40.0 * s, 20.0 * s, if on { SKY } else { with_alpha(CREAM_DEEP, 0.9) });
            if on && hl > 0.01 {
                rounded_rect_outline(tx - 2.5 * s, ty - 2.5 * s, tab_w + 5.0 * s, 45.0 * s, 22.5 * s, 2.5 * s, Color::new(1.0, 1.0, 1.0, hl));
            }
            text_mid(label, tx + tab_w * 0.5, ty + 20.0 * s, 19.0 * s, if on { INK } else { INK_SOFT });
        }
        let medals = stage_view(&views(), &def.id).and_then(|v| v.medal_times);
        if let Some(md) = medals {
            text_right(&format!("Gold {}   -   Platinum {}", format_time(md.gold), format_time(md.platinum)), x + w - 28.0 * s, ty + 26.0 * s, 16.0 * s, INK_SOFT);
        }
        // table
        let top = ty + 60.0 * s;
        let cols = [x + 40.0 * s, x + 100.0 * s, x + w * 0.38, x + w * 0.56, x + w * 0.74, x + w - 40.0 * s];
        for (i, hd) in ["#", "Driver", "Car", "Assists", "Time", "Gap"].iter().enumerate() {
            if i == 5 {
                text_right(hd, cols[5], top, 15.0 * s, INK_SOFT);
            } else {
                text(hd, cols[i], top, 15.0 * s, INK_SOFT);
            }
        }
        let row_h = 36.0 * s;
        let max_rows = (((y + h - 90.0 * s) - (top + 12.0 * s)) / row_h).floor().max(1.0) as usize;
        let filter = if self.camp.lb_filter == 0 { None } else { Some(filters[self.camp.lb_filter]) };
        let specs = car_specs();
        let car_name = |id: u8| specs.iter().find(|c| c.id == id).map(|c| c.name).unwrap_or("Blossom");
        let draw_row = |i: usize, rank: usize, name: &str, car: &str, assists: &str, time: f32, gap: Option<f32>, highlight: bool, alpha: f32| {
            let ry = top + 12.0 * s + i as f32 * row_h;
            if highlight {
                rounded_rect(x + 24.0 * s, ry, w - 48.0 * s, row_h - 4.0 * s, 12.0 * s, with_alpha(BUTTER, 0.6 * alpha));
            } else if i % 2 == 1 {
                rounded_rect(x + 24.0 * s, ry, w - 48.0 * s, row_h - 4.0 * s, 12.0 * s, with_alpha(CREAM_DEEP, 0.55 * alpha));
            }
            let mid = ry + (row_h - 4.0 * s) * 0.5;
            let fs = 18.0 * s;
            let medal = medals.map(|m| m.medal_for(time)).unwrap_or(Medal::None);
            if (1..=3).contains(&rank) {
                circle(cols[0] + 8.0 * s, mid, 12.0 * s, with_alpha([BUTTER, Color::new(0.86, 0.88, 0.95, 1.0), PEACH][rank - 1], alpha));
            }
            text_mid(&if rank == 0 { "-".to_string() } else { format!("{rank}") }, cols[0] + 8.0 * s, mid, fs * 0.85, with_alpha(INK, alpha));
            text(name, cols[1], mid_baseline(mid, fs), fs, with_alpha(INK, alpha));
            text(car, cols[2], mid_baseline(mid, fs * 0.9), fs * 0.9, with_alpha(INK_SOFT, alpha));
            text(assists, cols[3], mid_baseline(mid, fs * 0.9), fs * 0.9, with_alpha(INK_SOFT, alpha));
            draw_medal(cols[4] - 14.0 * s, mid, 8.0 * s, medal, alpha);
            text(&format_time(time), cols[4], mid_baseline(mid, fs), fs, with_alpha(INK, alpha));
            text_right(&gap.map(|g| format!("+{g:.2}")).unwrap_or_else(|| "-".into()), cols[5], mid_baseline(mid, fs * 0.9), fs * 0.9, with_alpha(INK_SOFT, alpha));
        };
        if self.camp.lb_tab == 0 {
            let rows = crate::campaign_mode::leaderboard(&def.id, filter, max_rows);
            let latest = rows.iter().map(|r| r.timestamp).max();
            if rows.is_empty() {
                text_mid("No times yet - be the first!", x + w * 0.5, top + 90.0 * s, 22.0 * s, INK_SOFT);
            }
            let lead = rows.first().map(|r| r.time_ms);
            for (i, e) in rows.iter().enumerate() {
                let gap = lead.filter(|_| i > 0).map(|l| (e.time_ms - l) as f32 / 1000.0);
                let ra = ((self.screen_t - 0.2 - i as f32 * 0.04) / 0.25).clamp(0.0, 1.0);
                draw_row(i, i + 1, &e.player, car_name(e.car), &e.assists, e.time_ms as f32 / 1000.0, gap, Some(e.timestamp) == latest, ra);
            }
        } else {
            // Global: a preview of how online boards will look, clearly marked
            let sample = [("mossy_tyres", 0u8, "Sport", 0.955), ("rallyfox", 2, "Sim", 0.97), ("pebble.drives", 1, "Normal", 0.99), ("hedgehog_gp", 3, "Cozy", 1.02), ("lumi", 2, "Normal", 1.04), ("sunny_sideways", 0, "Cozy", 1.07)];
            let gold = medals.map(|m| m.gold).unwrap_or(stage_length_m(&def) / 18.0);
            for (i, (n, c, a, f)) in sample.iter().enumerate().take(max_rows.saturating_sub(1)) {
                draw_row(i, i + 1, n, car_name(*c), a, gold * f, if i == 0 { None } else { Some(gold * (f - sample[0].3)) }, false, 0.45);
            }
            if let Some(b) = crate::campaign_mode::leaderboard(&def.id, None, 1).first() {
                let i = sample.len().min(max_rows.saturating_sub(1));
                draw_row(i, 0, "You (local best, not ranked yet)", car_name(b.car), &b.assists, b.time_ms as f32 / 1000.0, None, true, 1.0);
            }
            let bw = 520.0 * s;
            let bx = x + (w - bw) * 0.5;
            let byy = top + 90.0 * s;
            soft_shadow(bx, byy, bw, 92.0 * s, 20.0 * s, 1.0);
            rounded_rect_gradient(bx, byy, bw, 92.0 * s, 20.0 * s, lighten(LAVENDER, 0.5), lighten(LAVENDER, 0.2));
            text_mid("Global leaderboards are coming soon!", x + w * 0.5, byy + 34.0 * s, 24.0 * s, INK);
            text_mid("Race friends and rally fans everywhere - this is a preview.", x + w * 0.5, byy + 64.0 * s, 16.0 * s, INK_SOFT);
        }
        let back = button_back(&mut self.focus, "Back", x + w * 0.5 - 120.0 * s, y + h - 70.0 * s, 240.0 * s, 50.0 * s, PEACH);
        if back || self.focus.back {
            if self.focus.back {
                audio::ui_sound(audio::UiSound::Back);
            }
            let to = self.camp.lb_from;
            self.goto(to);
        }
    }
}
