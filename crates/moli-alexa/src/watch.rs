//! Whether a session that says « PLAYING » still plays. Amazon keeps a
//! session « PLAYING » long after the speaker went silent (« Alexa, stop »
//! does not always end it), and the Alexa app shows it so. Moli believes
//! such a session only while it moves (its progress advances, or its track
//! changes from one reading to the next), or while the speaker's own state
//! says it plays too; a session that stands still shows the speaker paused,
//! then idle after [`IDLE_AFTER`].
//!
//! Multi-room groups are not watched: their members' state comes from the
//! group (see `media::through_groups`).

use std::collections::HashMap;
use std::time::{Duration, Instant};

use crate::media::{IDLE, PAUSED, PLAYING, Playing};

/// A session still this long is the speaker gone idle.
pub(crate) const IDLE_AFTER: Duration = Duration::from_secs(10 * 60);
/// Two readings closer than this tell nothing (the one 3 s after an order
/// may not have moved yet): the verdict of the last comparison holds.
const SPAN: Duration = Duration::from_secs(10);
/// An order from Moli speaks for the session seen this soon after it.
const ORDER_HOLDS: Duration = Duration::from_secs(2 * 60);
/// Progress that moved less than this between two readings stood still.
const TOLERANCE: f64 = 1.0;

/// What two readings of a playing session say.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Motion {
    /// First seen: nothing to compare with yet.
    New,
    /// First seen after an order from Moli: believed.
    Ordered,
    /// Its progress advanced, or its track changed.
    Moves,
    /// Its progress stood still.
    Still,
    /// It gives no progress to compare.
    NoProgress,
}

#[derive(Debug)]
struct Seen {
    track: String,
    progress: Option<f64>,
    /// When `progress` was read.
    at: Instant,
    /// When it last moved (or was first seen, or confirmed by the speaker).
    moved: Instant,
    last: Motion,
}

/// The playing sessions seen at the last readings, by serial.
#[derive(Debug, Default)]
pub(crate) struct Watch {
    seen: HashMap<String, Seen>,
    /// Speakers Moli just gave a play or pause order to, and when.
    ordered: HashMap<String, Instant>,
}

impl Watch {
    /// How the session `p` of `serial` moved since the last reading, and how
    /// long it has stood still.
    pub(crate) fn judge(&mut self, serial: &str, p: &Playing, now: Instant) -> (Motion, Duration) {
        let track = if p.track.is_empty() {
            format!("{}\u{1f}{}", p.title, p.artist)
        } else {
            p.track.clone()
        };
        let fresh = |last| Seen {
            track: track.clone(),
            progress: p.progress,
            at: now,
            moved: now,
            last,
        };
        let Some(s) = self.seen.get_mut(serial) else {
            let ordered = self.ordered.remove(serial);
            let first = if ordered.is_some_and(|t| now.saturating_duration_since(t) < ORDER_HOLDS) {
                Motion::Ordered
            } else {
                Motion::New
            };
            self.seen.insert(serial.to_owned(), fresh(first));
            return (first, Duration::ZERO);
        };
        if s.track != track {
            *s = fresh(Motion::Moves);
            return (Motion::Moves, Duration::ZERO);
        }
        if now.saturating_duration_since(s.at) < SPAN {
            return (s.last, now.saturating_duration_since(s.moved));
        }
        let motion = match (s.progress, p.progress) {
            (None, None) => Motion::NoProgress,
            (Some(a), Some(b)) if (b - a).abs() < TOLERANCE => Motion::Still,
            _ => Motion::Moves,
        };
        if motion != Motion::Still {
            s.moved = now;
        }
        s.progress = p.progress;
        s.at = now;
        s.last = motion;
        (motion, now.saturating_duration_since(s.moved))
    }

    /// The speaker's own state says it plays: not still.
    pub(crate) fn confirm(&mut self, serial: &str, now: Instant) {
        if let Some(s) = self.seen.get_mut(serial) {
            s.moved = now;
        }
    }

    /// Moli gave `serial` a play or pause order: what was seen before says
    /// nothing of what comes next.
    pub(crate) fn reset(&mut self, serial: &str, now: Instant) {
        self.seen.remove(serial);
        self.ordered.insert(serial.to_owned(), now);
    }

    /// Only `playing` play now: the others start afresh when they play again.
    pub(crate) fn keep(&mut self, playing: &[&str]) {
        self.seen.retain(|s, _| playing.contains(&s.as_str()));
    }
}

/// What to show of a playing session, from how it moved and (when asked)
/// what the speaker's own state says; with a word for the debug log.
pub(crate) fn judged(
    session: &Playing,
    motion: Motion,
    still_for: Duration,
    own: Option<&Playing>,
) -> (Playing, &'static str) {
    let own_plays = own.is_some_and(|o| o.state == PLAYING);
    // The speaker says outright it does not play (`IDLE`, `PAUSED`…).
    let own_denies = own.is_some_and(|o| !o.is_unknown() && o.state != PLAYING);
    let word = match motion {
        Motion::Ordered | Motion::Moves => return (session.clone(), "advances"),
        _ if own_plays => return (session.clone(), "confirmed"),
        Motion::New if !own_denies => return (session.clone(), "new"),
        // A live radio gives no progress: only the speaker can deny it.
        Motion::NoProgress if !own_denies => return (session.clone(), "none"),
        _ => "frozen",
    };
    let volume = session.volume.or(own.and_then(|o| o.volume));
    let shown = if still_for >= IDLE_AFTER {
        Playing {
            state: IDLE.into(),
            volume,
            ..Playing::default()
        }
    } else {
        Playing {
            state: PAUSED.into(),
            volume,
            ..session.clone()
        }
    };
    (shown, word)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::media::{UNKNOWN, sessions};
    use serde_json::Value as Json;

    const ECHO: &str = "G0000000000000A1";

    /// The anonymized answer of the device form, its progress at `at`
    /// seconds, its track `track`.
    fn session(at: f64, track: &str) -> Playing {
        let mut answer: Json =
            serde_json::from_str(include_str!("../fixtures/list-media-sessions-device.json"))
                .unwrap();
        let now = &mut answer["mediaSessionList"][0]["nowPlayingData"];
        now["progress"]["mediaProgress"] = at.into();
        now["mediaId"] = track.into();
        sessions(&answer).remove(0).playing
    }

    fn own(state: &str) -> Playing {
        Playing {
            state: state.into(),
            ..Playing::default()
        }
    }

    /// Readings 30 s apart, as the poll makes them.
    fn readings(
        watch: &mut Watch,
        start: Instant,
        all: &[(u64, Playing, Option<Playing>)],
    ) -> Vec<(String, &'static str)> {
        all.iter()
            .map(|(t, p, o)| {
                let now = start + Duration::from_secs(*t);
                let (motion, still) = watch.judge(ECHO, p, now);
                let (shown, word) = judged(p, motion, still, o.as_ref());
                if shown.state == PLAYING && o.as_ref().is_some_and(|o| o.state == PLAYING) {
                    watch.confirm(ECHO, now);
                }
                (shown.state, word)
            })
            .collect()
    }

    #[test]
    fn a_frozen_session_is_paused_then_idle() {
        let p = session(97.0, "fictif-piste-1");
        assert_eq!(
            (p.state.as_str(), p.progress, p.track.as_str()),
            ("PLAYING", Some(97.0), "fictif-piste-1")
        );
        let mut watch = Watch::default();
        let start = Instant::now();
        let idle = Some(own("IDLE"));
        let seen = readings(
            &mut watch,
            start,
            &[
                (0, p.clone(), idle.clone()),
                (30, p.clone(), idle.clone()),
                // The speaker's own state says nothing: still frozen.
                (60, p.clone(), Some(own(UNKNOWN))),
                (599, p.clone(), idle.clone()),
                (601, p.clone(), idle.clone()),
            ],
        );
        let states: Vec<&str> = seen.iter().map(|(s, _)| s.as_str()).collect();
        assert_eq!(states, ["PAUSED", "PAUSED", "PAUSED", "PAUSED", "IDLE"]);
        assert!(seen.iter().all(|(_, w)| *w == "frozen"));
        // Paused keeps the title; idle shows nothing.
        let (paused, _) = judged(&p, Motion::Still, Duration::from_secs(60), None);
        assert_eq!(
            (paused.title.as_str(), paused.volume),
            ("Un titre", Some(30.0))
        );
        let (idle, _) = judged(&p, Motion::Still, IDLE_AFTER, None);
        assert!(idle.title.is_empty() && idle.artist.is_empty());
    }

    #[test]
    fn a_session_that_advances_plays() {
        let mut watch = Watch::default();
        let start = Instant::now();
        let seen = readings(
            &mut watch,
            start,
            &[
                // First seen, the speaker says nothing: the benefit of the doubt.
                (0, session(97.0, "fictif-piste-1"), Some(own(UNKNOWN))),
                (30, session(127.0, "fictif-piste-1"), None),
                (60, session(157.5, "fictif-piste-1"), None),
            ],
        );
        assert_eq!(
            seen,
            [
                ("PLAYING".to_owned(), "new"),
                ("PLAYING".to_owned(), "advances"),
                ("PLAYING".to_owned(), "advances"),
            ]
        );
        // Standing still while the speaker says it plays: believed, and
        // never idle while it does.
        let p = session(157.5, "fictif-piste-1");
        let seen = readings(
            &mut watch,
            start,
            &[
                (90, p.clone(), Some(own("PLAYING"))),
                (900, p.clone(), Some(own("PLAYING"))),
                (930, p.clone(), Some(own("IDLE"))),
            ],
        );
        assert_eq!(
            seen,
            [
                ("PLAYING".to_owned(), "confirmed"),
                ("PLAYING".to_owned(), "confirmed"),
                ("PAUSED".to_owned(), "frozen"),
            ]
        );
    }

    #[test]
    fn a_new_track_moves() {
        let mut watch = Watch::default();
        let start = Instant::now();
        let idle = Some(own("IDLE"));
        let seen = readings(
            &mut watch,
            start,
            &[
                (0, session(200.0, "fictif-piste-1"), None),
                (30, session(200.0, "fictif-piste-1"), idle.clone()),
                // Next track, its progress back near 0: it moves.
                (60, session(2.0, "fictif-piste-2"), idle.clone()),
                (90, session(32.0, "fictif-piste-2"), idle.clone()),
            ],
        );
        let states: Vec<&str> = seen.iter().map(|(s, _)| s.as_str()).collect();
        assert_eq!(states, ["PLAYING", "PAUSED", "PLAYING", "PLAYING"]);
        // No id: a new title is a new track all the same.
        let mut a = session(10.0, "");
        let mut watch = Watch::default();
        watch.judge(ECHO, &a, start);
        a.title = "Un autre titre".into();
        let later = start + Duration::from_secs(30);
        assert_eq!(watch.judge(ECHO, &a, later).0, Motion::Moves);
    }

    #[test]
    fn an_order_from_moli_starts_afresh() {
        let mut watch = Watch::default();
        let start = Instant::now();
        let p = session(97.0, "fictif-piste-1");
        let idle = Some(own("IDLE"));
        readings(
            &mut watch,
            start,
            &[(0, p.clone(), None), (30, p.clone(), None)],
        );
        // Paused by Moli, then played again: the speaker's own state lags.
        watch.reset(ECHO, start + Duration::from_secs(35));
        watch.keep(&[]);
        let seen = readings(
            &mut watch,
            start,
            &[
                (40, p.clone(), idle.clone()),
                // 3 s after the order: too soon to tell, the order holds.
                (43, p.clone(), idle.clone()),
                (73, session(127.0, "fictif-piste-1"), idle.clone()),
            ],
        );
        assert_eq!(
            seen,
            [
                ("PLAYING".to_owned(), "advances"),
                ("PLAYING".to_owned(), "advances"),
                ("PLAYING".to_owned(), "advances"),
            ]
        );
        // The order is used once: a session seen again later is new.
        watch.keep(&[]);
        assert_eq!(
            watch.judge(ECHO, &p, start + Duration::from_secs(200)).0,
            Motion::New
        );
        // An order long past speaks for nothing.
        watch.reset(ECHO, start);
        watch.keep(&[]);
        let later = start + ORDER_HOLDS + Duration::from_secs(1);
        assert_eq!(watch.judge(ECHO, &p, later).0, Motion::New);
    }

    #[test]
    fn a_session_without_progress_is_left_to_the_speaker() {
        let mut p = session(0.0, "fictif-radio");
        p.progress = None;
        let mut watch = Watch::default();
        let start = Instant::now();
        let seen = readings(
            &mut watch,
            start,
            &[
                (0, p.clone(), Some(own(UNKNOWN))),
                (30, p.clone(), Some(own(UNKNOWN))),
                (60, p.clone(), Some(own("IDLE"))),
            ],
        );
        assert_eq!(
            seen,
            [
                ("PLAYING".to_owned(), "new"),
                ("PLAYING".to_owned(), "none"),
                ("PAUSED".to_owned(), "frozen"),
            ]
        );
    }
}
