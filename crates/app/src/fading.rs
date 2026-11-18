//! How far along the stage's fades are: when each scene came up, and
//! when what is on began fading out. Whatever the dashboard's motion is
//! set to, these run: they are what the step asks of the projector.

use std::time::{Duration, Instant};

use lantern_engine::{Scene, SceneId, StageState};

/// How much of what is on gets through, from 0 (none) to 1 (all of it).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Levels {
    /// Of what is seen: the program, or the picture under a sound.
    pub picture: f32,
    /// Of what plays on program.
    pub sound: f32,
    /// How much of a fade out is still to go; `None` with none under way.
    pub left: Option<f32>,
}

impl Levels {
    /// All of it, with nothing fading.
    pub const FULL: Levels = Levels {
        picture: 1.0,
        sound: 1.0,
        left: None,
    };
}

#[derive(Debug, Default)]
pub struct Fades {
    /// The scenes on stage with when they were ready to show.
    up: Vec<(SceneId, Instant)>,
    /// The program that began fading out, and when.
    out: Option<(SceneId, Instant)>,
}

impl Fades {
    /// `scene` is ready to show from `now`, unless it was already: a
    /// picture back from under a sound doesn't come up again.
    pub fn up(&mut self, scene: SceneId, now: Instant) {
        if !self.up.iter().any(|(id, _)| *id == scene) {
            self.up.push((scene, now));
        }
    }

    /// Follows the stage: forgets what left it, and a fade out starts
    /// when the stage is first seen with one.
    pub fn sync(&mut self, stage: &StageState, now: Instant) {
        let on = |id: SceneId| {
            [&stage.program, &stage.under]
                .into_iter()
                .flatten()
                .any(|scene| scene.id == id)
        };
        self.up.retain(|(id, _)| on(*id));
        let program = stage.program.as_ref().map(|scene| scene.id);
        self.out = match (stage.fading_out, program) {
            (Some(_), Some(program)) => match self.out {
                Some((id, since)) if id == program => Some((id, since)),
                _ => Some((program, now)),
            },
            _ => None,
        };
    }

    /// How far up `scene` is at `now`: all the way without a fade in,
    /// none of the way before it is ready to show.
    fn up_by(&self, scene: &Scene, now: Instant) -> f32 {
        let Some(fade_in) = scene.fade_in else {
            return 1.0;
        };
        match self.up.iter().find(|(id, _)| *id == scene.id) {
            Some((_, since)) => along(*since, fade_in.duration(), now),
            None => 0.0,
        }
    }

    /// The levels at `now`.
    pub fn levels(&self, stage: &StageState, now: Instant) -> Levels {
        let left = stage
            .fading_out
            .zip(self.out)
            .map(|(fade, (_, since))| 1.0 - along(since, fade.over.duration(), now));
        let going = left.unwrap_or(1.0);
        let picture = stage.picture().map_or(1.0, |scene| self.up_by(scene, now));
        let picture_going = match stage.fading_out {
            Some(fade) if fade.picture => going,
            _ => 1.0,
        };
        let sound = stage
            .program
            .as_ref()
            .map_or(1.0, |scene| self.up_by(scene, now));
        Levels {
            picture: picture * picture_going,
            sound: sound * going,
            left,
        }
    }

    /// Whether anything is still on its way up or down at `now`.
    pub fn moving(&self, stage: &StageState, now: Instant) -> bool {
        let levels = self.levels(stage, now);
        let rising = [&stage.program, &stage.under]
            .into_iter()
            .flatten()
            .any(|scene| {
                scene.fade_in.is_some()
                    && self.up.iter().any(|(id, _)| *id == scene.id)
                    && self.up_by(scene, now) < 1.0
            });
        rising || levels.left.is_some()
    }

    /// The program, once it has faded out by `now`: for the engine to
    /// take it off and go on.
    pub fn faded(&self, stage: &StageState, now: Instant) -> Option<SceneId> {
        let (program, since) = self.out?;
        let fade = stage.fading_out?;
        (now.saturating_duration_since(since) >= fade.over.duration()).then_some(program)
    }
}

/// How far from `since` to `since + over` `now` is, from 0 to 1.
fn along(since: Instant, over: Duration, now: Instant) -> f32 {
    let gone = now.saturating_duration_since(since).as_secs_f32();
    (gone / over.as_secs_f32().max(f32::EPSILON)).clamp(0.0, 1.0)
}

/// How loud to play something `level` of the way up: on a curve, as the
/// ear hears, so a fade sounds even rather than gone in its last moment.
pub fn loudness(level: f32) -> f64 {
    f64::from(level.clamp(0.0, 1.0)).powi(3)
}

#[cfg(test)]
mod tests {
    use lantern_engine::{FadeOut, FadeTime, Kind, Location, Resource, ResourceId, SourceId};

    use super::*;

    /// A scene as a window is sent it.
    fn scene(id: u64, kind: Kind, fade_in: u64) -> Scene {
        let source = SourceId::new("test").unwrap();
        let resource = Resource::new(
            ResourceId::new(&source, &id.to_string()),
            id.to_string(),
            kind,
            Location::File("/x".into()),
        );
        let mut scene: Scene = serde_json::from_value(serde_json::json!({
            "id": id,
            "resource": resource,
            "media": {"type": "frames", "frame": 0, "count": null},
        }))
        .unwrap();
        scene.fade_in = FadeTime::from_secs(fade_in);
        scene
    }

    fn secs(n: f32) -> Duration {
        Duration::from_secs_f32(n)
    }

    #[test]
    fn a_picture_comes_up_from_when_it_is_ready_and_stays_up() {
        let start = Instant::now();
        let picture = scene(1, Kind::Image, 2);
        let stage = StageState {
            program: Some(picture.clone()),
            ..StageState::default()
        };
        let mut fades = Fades::default();
        fades.sync(&stage, start);
        assert_eq!(fades.levels(&stage, start).picture, 0.0, "not ready yet");
        fades.up(picture.id, start);
        assert_eq!(fades.levels(&stage, start).picture, 0.0);
        let half = fades.levels(&stage, start + secs(1.0)).picture;
        assert!((half - 0.5).abs() < 0.01, "{half}");
        assert!(fades.moving(&stage, start + secs(1.0)));
        assert_eq!(fades.levels(&stage, start + secs(3.0)).picture, 1.0);
        assert!(!fades.moving(&stage, start + secs(3.0)));
        fades.up(picture.id, start + secs(3.0));
        assert_eq!(
            fades.levels(&stage, start + secs(3.0)).picture,
            1.0,
            "back from under a sound, it is up already"
        );
    }

    #[test]
    fn a_fade_out_takes_the_sound_and_the_picture_it_says() {
        let start = Instant::now();
        let video = scene(1, Kind::Video, 0);
        let mut stage = StageState {
            program: Some(video.clone()),
            ..StageState::default()
        };
        let mut fades = Fades::default();
        fades.sync(&stage, start);
        assert_eq!(fades.faded(&stage, start), None);
        stage.fading_out = Some(FadeOut {
            over: FadeTime::from_secs(4).unwrap(),
            picture: true,
        });
        fades.sync(&stage, start);
        fades.sync(&stage, start + secs(1.0));
        let levels = fades.levels(&stage, start + secs(1.0));
        assert!(
            (levels.picture - 0.75).abs() < 0.01,
            "seen again later, the fade keeps its start: {levels:?}"
        );
        assert!((levels.sound - 0.75).abs() < 0.01, "{levels:?}");
        assert_eq!(fades.faded(&stage, start + secs(3.9)), None);
        assert_eq!(fades.faded(&stage, start + secs(4.0)), Some(video.id));

        stage.fading_out = Some(FadeOut {
            over: FadeTime::from_secs(4).unwrap(),
            picture: false,
        });
        let levels = fades.levels(&stage, start + secs(2.0));
        assert_eq!(levels.picture, 1.0, "a sound alone leaves the picture");
        assert!((levels.sound - 0.5).abs() < 0.01, "{levels:?}");
        assert!(matches!(levels.left, Some(left) if (left - 0.5).abs() < 0.01));

        stage.program = Some(scene(2, Kind::Image, 0));
        stage.fading_out = None;
        fades.sync(&stage, start + secs(5.0));
        assert_eq!(
            fades.faded(&stage, start + secs(9.0)),
            None,
            "something else on: nothing is told faded"
        );
        assert_eq!(fades.levels(&stage, start + secs(9.0)).left, None);
    }

    #[test]
    fn loudness_goes_on_a_curve_from_silence_to_all() {
        assert_eq!(loudness(0.0), 0.0);
        assert_eq!(loudness(1.0), 1.0);
        assert!(loudness(0.5) < 0.5 / 2.0, "half way is well down");
        assert_eq!(loudness(2.0), 1.0);
    }
}
