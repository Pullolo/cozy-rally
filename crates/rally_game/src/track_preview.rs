//! Track preview for the race setup screens: the track the current options will build,
//! generated on a background thread (the same `Track::generate` the race uses) so the map can
//! follow the options without stalling the menu.
use rally_core::track::{Track, TrackParams};
use std::sync::mpsc::{channel, Receiver};
use std::sync::Arc;

#[derive(Default)]
pub struct TrackPreview {
    /// Last finished preview and the params it was built from
    done: Option<(TrackParams, Arc<Track>)>,
    /// Build in flight
    rx: Option<Receiver<(TrackParams, Arc<Track>)>>,
}

impl TrackPreview {
    /// The preview for `params` (night only changes the lighting, so it is ignored). Returns the
    /// newest finished track and whether it matches `params` (false while a rebuild runs: the
    /// old map is still shown). Starts a rebuild when needed; one build at a time, so quickly
    /// cycling through seeds only builds the latest choice once the running build is done.
    pub fn get(&mut self, params: &TrackParams) -> Option<(Arc<Track>, bool)> {
        let key = TrackParams { night: false, ..params.clone() };
        if let Some(rx) = &self.rx {
            if let Ok(done) = rx.try_recv() {
                self.done = Some(done);
                self.rx = None;
            }
        }
        let fresh = self.done.as_ref().is_some_and(|(p, _)| *p == key);
        if !fresh && self.rx.is_none() {
            let (tx, rx) = channel();
            std::thread::spawn(move || {
                let t = Arc::new(Track::generate(&key));
                let _ = tx.send((key, t));
            });
            self.rx = Some(rx);
        }
        self.done.as_ref().map(|(_, t)| (t.clone(), fresh))
    }
}
