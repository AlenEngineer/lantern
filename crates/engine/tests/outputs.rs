//! Where each step's picture and sound come from: its own, another file,
//! what was on, or nothing; and a sound that keeps going under the steps
//! after its own.

use lantern_engine::{
    Action, Engine, Fade, FadeTime, Kind, Location, MediaReport, Meets, Picture, Plan, Resource,
    ResourceId, SourceId, StepSound,
};

fn file(name: &str, kind: Kind) -> Resource {
    Resource::new(
        ResourceId::new(&SourceId::new("test").unwrap(), name),
        name,
        kind,
        Location::File(format!("/{name}").into()),
    )
}

/// What is seen, what plays apart, whether what is seen is silent, and
/// whether what plays apart is lowered.
fn on(engine: &Engine) -> (Option<String>, Option<String>, bool, bool) {
    let stage = engine.stage();
    let title = |scene: Option<&lantern_engine::Scene>| scene.map(|s| s.resource.title.clone());
    (
        title(stage.program.as_ref()),
        title(stage.bed.as_ref()),
        stage.muted,
        stage.lowered,
    )
}

fn some(name: &str) -> Option<String> {
    Some(name.to_string())
}

#[test]
fn a_steps_picture_can_come_from_elsewhere_while_its_own_sound_plays() {
    let mut plan = Plan::new("Sunday");
    plan.show(file("welcome.png", Kind::Image));
    let song = plan.show(file("song.mp3", Kind::Audio));
    let hymn = plan.show(file("hymn.mp4", Kind::Video));
    let video = plan.show(file("clip.mp4", Kind::Video));
    plan.get_mut(song).unwrap().picture = Picture::File(file("cross.png", Kind::Image));
    plan.get_mut(hymn).unwrap().picture = Picture::Keep;
    plan.get_mut(video).unwrap().picture = Picture::Black;
    let engine = Engine::new();
    engine.load_plan(plan);

    engine.next();
    assert_eq!(on(&engine), (some("welcome.png"), None, false, false));
    // Another picture in its place; its own sound plays apart.
    engine.next();
    assert_eq!(
        on(&engine),
        (some("cross.png"), some("song.mp3"), false, false)
    );
    // The picture before stays, and the video's sound plays apart.
    engine.next();
    assert_eq!(
        on(&engine),
        (some("cross.png"), some("hymn.mp4"), true, false)
    );
    // Nothing is seen; the clip is heard.
    engine.next();
    assert_eq!(on(&engine), (None, some("clip.mp4"), false, false));
}

#[test]
fn a_step_can_be_silent() {
    let mut plan = Plan::new("Sunday");
    let hymn = plan.show(file("hymn.mp4", Kind::Video));
    plan.get_mut(hymn).unwrap().silent = true;
    let engine = Engine::new();
    engine.load_plan(plan);
    engine.next();
    assert_eq!(on(&engine), (some("hymn.mp4"), None, true, false));
}

#[test]
fn a_sound_keeps_going_under_the_steps_after_its_own() {
    let plan = |meets: Meets| {
        let mut plan = Plan::new("Sunday");
        let reading = plan.show(file("reading.png", Kind::Image));
        plan.show(file("prayer.png", Kind::Image));
        let hymn = plan.show(file("hymn.mp4", Kind::Video));
        plan.show(file("after.png", Kind::Image));
        plan.get_mut(reading).unwrap().sound = Some(StepSound {
            until: Some(hymn),
            meets,
            ..StepSound::new(file("piano.mp3", Kind::Audio))
        });
        plan
    };

    let engine = Engine::new();
    engine.load_plan(plan(Meets::Fade));
    engine.next();
    let piano = engine.stage().bed.unwrap();
    assert_eq!(piano.resource.title, "piano.mp3");
    // Under the next step, the same sound goes on.
    engine.next();
    assert_eq!(
        on(&engine),
        (some("prayer.png"), some("piano.mp3"), false, false)
    );
    assert_eq!(engine.stage().bed.unwrap().id, piano.id);
    // A step with a sound of its own: it fades out there.
    engine.next();
    assert_eq!(on(&engine), (some("hymn.mp4"), None, false, false));

    // Set to go under such a step, it plays on lowered.
    let engine = Engine::new();
    engine.load_plan(plan(Meets::Under));
    engine.next();
    engine.next();
    engine.next();
    assert_eq!(
        on(&engine),
        (some("hymn.mp4"), some("piano.mp3"), false, true)
    );
    // Past where it keeps going to, it goes.
    engine.next();
    assert_eq!(on(&engine), (some("after.png"), None, false, false));
}

#[test]
fn where_a_step_keeps_its_sound_is_saved_with_the_plan() {
    let mut plan = Plan::new("Sunday");
    let a = plan.show(file("a.png", Kind::Image));
    let b = plan.push("b", Action::Note);
    plan.get_mut(a).unwrap().picture = Picture::Keep;
    plan.get_mut(a).unwrap().silent = true;
    plan.get_mut(a).unwrap().sound = Some(StepSound {
        until: Some(b),
        meets: Meets::Cut,
        ..StepSound::new(file("piano.mp3", Kind::Audio))
    });
    let json = plan.to_json();
    let back = Plan::from_json(&json).unwrap();
    assert_eq!(back.steps()[0], plan.steps()[0]);
    // A step as ever says nothing of it.
    let plain = Plan::new("x");
    assert!(!plain.to_json().contains("picture"));
}

#[test]
fn a_still_goes_on_when_the_sound_it_plays_apart_ends() {
    let mut plan = Plan::new("Sunday");
    let song = plan.show(file("song.mp3", Kind::Audio));
    let after = plan.show(file("welcome.png", Kind::Image));
    let hymn = plan.show(file("hymn.mp4", Kind::Video));
    let last = plan.show(file("end.png", Kind::Image));
    plan.get_mut(song).unwrap().picture = Picture::File(file("cross.png", Kind::Image));
    plan.get_mut(hymn).unwrap().sound = Some(StepSound::new(file("track.mp3", Kind::Audio)));
    plan.join_over(song, after).unwrap();
    plan.join_over(hymn, last).unwrap();
    let engine = Engine::new();
    engine.load_plan(plan);

    engine.next();
    let (bed, owner) = engine.plays_apart().unwrap();
    assert_eq!(owner, song);
    engine.report(bed, MediaReport::Ended);
    assert_eq!(
        engine.current_step().map(|s| s.id),
        Some(after),
        "it went on"
    );

    // Under a video of its own, the video's end moves the plan, not the
    // sound's.
    engine.next();
    let (bed, _) = engine.plays_apart().unwrap();
    let video = engine.stage().program.unwrap().id;
    engine.report(video, MediaReport::Timeline { duration: None });
    engine.report(bed, MediaReport::Ended);
    assert_eq!(engine.current_step().map(|s| s.id), Some(hymn));
}

#[test]
fn a_steps_sound_apart_can_fade_on_its_own() {
    let secs = |n| FadeTime::from_secs(n);
    let plan = |own: bool| {
        let mut plan = Plan::new("Sunday");
        let song = plan.show(file("song.mp3", Kind::Audio));
        plan.show(file("welcome.png", Kind::Image));
        let step = plan.get_mut(song).unwrap();
        step.picture = Picture::File(file("cross.png", Kind::Image));
        step.fade = Fade {
            fade_in: secs(1),
            fade_out: secs(2),
        };
        if own {
            step.sound_fade = Some(Fade {
                fade_in: secs(4),
                fade_out: secs(6),
            });
        }
        plan
    };
    for own in [false, true] {
        let engine = Engine::new();
        engine.load_plan(plan(own));
        engine.next();
        let bed = engine.stage().bed.unwrap();
        let fade_in = if own { secs(4) } else { secs(1) };
        assert_eq!(bed.fade_in, fade_in, "own fade: {own}");
        // Going on, what is seen fades out first; a sound fading on its
        // own goes at once, over its own fade.
        let feed = engine.subscribe();
        engine.next();
        let stage = engine.stage();
        assert!(stage.fading_out.is_some());
        assert_eq!(stage.bed.is_none(), own, "own fade: {own}");
        let lifted = feed.events.try_iter().find_map(|event| match event {
            lantern_engine::StageEvent::Bed {
                scene: None,
                fade_out,
            } => Some(fade_out),
            _ => None,
        });
        assert_eq!(lifted, own.then_some(secs(6)), "own fade: {own}");
    }
}
