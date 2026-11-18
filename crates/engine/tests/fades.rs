//! Steps that fade: up as they go on stage, and out before the plan goes
//! on from them.

use lantern_engine::{
    Action, Ending, Engine, Fade, FadeOut, FadeTime, Flow, Kind, Location, MediaReport, Moved,
    Plan, Repeat, Resource, ResourceId, SceneId, SourceId, StepId, StepSound, TimeOfDay,
};

fn resource(title: &str, kind: Kind) -> Resource {
    let source = SourceId::new("test").unwrap();
    let location = match kind {
        Kind::Image => Location::Url(title.into()),
        _ => Location::File(format!("/{title}").into()),
    };
    Resource::new(ResourceId::new(&source, title), title, kind, location)
}

fn seconds(n: u64) -> Option<FadeTime> {
    FadeTime::from_secs(n)
}

fn fading(plan: &mut Plan, step: StepId, fade_in: u64, fade_out: u64) {
    plan.get_mut(step).unwrap().fade = Fade {
        fade_in: seconds(fade_in),
        fade_out: seconds(fade_out),
    };
}

fn current(engine: &Engine) -> Option<StepId> {
    engine.current_step().map(|step| step.id)
}

fn on_program(engine: &Engine) -> Option<(SceneId, String)> {
    let program = engine.stage().program?;
    Some((program.id, program.resource.title.clone()))
}

fn seen(engine: &Engine) -> Option<String> {
    engine
        .stage()
        .picture()
        .map(|scene| scene.resource.title.clone())
}

/// A song that fades out over four seconds, then a picture.
fn song_then_picture() -> (Engine, StepId, StepId) {
    let mut plan = Plan::new("Sunday");
    let song = plan.show(resource("Song", Kind::Video));
    let picture = plan.show(resource("Welcome", Kind::Image));
    fading(&mut plan, song, 0, 4);
    let engine = Engine::new();
    engine.load_plan(plan);
    (engine, song, picture)
}

#[test]
fn going_on_from_a_step_that_fades_out_waits_until_it_has_faded() {
    let (engine, song, picture) = song_then_picture();
    assert_eq!(engine.next(), Moved::Step(song));
    let (scene, _) = on_program(&engine).unwrap();

    assert_eq!(engine.next(), Moved::FadingOut(seconds(4).unwrap()));
    assert_eq!(current(&engine), Some(song), "still on the song");
    assert_eq!(on_program(&engine), Some((scene, "Song".into())));
    assert_eq!(
        engine.stage().fading_out,
        Some(FadeOut {
            over: seconds(4).unwrap(),
            picture: true,
        })
    );

    assert!(engine.faded(scene));
    assert_eq!(current(&engine), Some(picture));
    assert_eq!(seen(&engine).as_deref(), Some("Welcome"));
    assert_eq!(engine.stage().fading_out, None);
    let logged: Vec<String> = engine.log().into_iter().map(|e| e.resource.title).collect();
    assert_eq!(logged, ["Song", "Welcome"]);
    assert!(!engine.faded(scene), "said twice, it goes on once");
    assert_eq!(current(&engine), Some(picture));
}

#[test]
fn pressed_again_while_it_fades_it_goes_on_at_once() {
    let (engine, song, picture) = song_then_picture();
    engine.next();
    let (scene, _) = on_program(&engine).unwrap();
    assert!(matches!(engine.next(), Moved::FadingOut(_)));
    assert_eq!(engine.next(), Moved::Step(picture));
    assert_eq!(seen(&engine).as_deref(), Some("Welcome"));
    assert!(
        !engine.faded(scene),
        "the stage's word comes too late to move the plan again"
    );
    assert_eq!(current(&engine), Some(picture));
    engine.edit_plan(|plan| fading(plan, song, 0, 0));
    engine.go_to(song).unwrap();
    assert_eq!(
        engine.next(),
        Moved::Step(picture),
        "without a fade out, at once as ever"
    );
}

#[test]
fn a_step_comes_up_over_its_fade_in_with_the_sound_it_plays() {
    let mut plan = Plan::new("Sunday");
    let psalm = plan.show(Resource::new(
        ResourceId::new(&SourceId::new("test").unwrap(), "psalm"),
        "Salmo 23",
        Kind::Text,
        Location::Inline("Uno\n\nDos".into()),
    ));
    let still = plan.show(resource("Welcome", Kind::Image));
    fading(&mut plan, psalm, 3, 0);
    plan.get_mut(psalm).unwrap().sound = Some(StepSound {
        resource: resource("Piano", Kind::Audio),
        from: 0,
    });
    let engine = Engine::new();
    engine.load_plan(plan);

    engine.next();
    let stage = engine.stage();
    let (sound, picture) = (stage.program.unwrap(), stage.under.unwrap());
    assert_eq!(sound.resource.title, "Piano");
    assert_eq!(sound.fade_in, seconds(3));
    assert_eq!(picture.resource.title, "Salmo 23");
    assert_eq!(picture.fade_in, seconds(3));
    engine.go_to_at(psalm, 1).unwrap();
    assert_eq!(
        engine.stage().under.unwrap().fade_in,
        seconds(3),
        "picked up where it was, it comes up the same way"
    );
    engine.go_to(still).unwrap();
    assert_eq!(engine.stage().program.unwrap().fade_in, None, "at once");
}

#[test]
fn a_sound_alone_fades_and_leaves_the_picture_it_played_over() {
    let mut plan = Plan::new("Sunday");
    plan.show(resource("Welcome", Kind::Image));
    let music = plan.show(resource("Music", Kind::Audio));
    let prayer = plan.push("Prayer", Action::Note);
    fading(&mut plan, music, 0, 5);
    let engine = Engine::new();
    engine.load_plan(plan);

    engine.next();
    engine.next();
    let (scene, title) = on_program(&engine).unwrap();
    assert_eq!(title, "Music");
    assert_eq!(seen(&engine).as_deref(), Some("Welcome"));
    assert!(matches!(engine.next(), Moved::FadingOut(_)));
    assert_eq!(
        engine.stage().fading_out.map(|fade| fade.picture),
        Some(false)
    );

    assert!(engine.faded(scene));
    assert_eq!(current(&engine), Some(prayer));
    assert_eq!(
        on_program(&engine).map(|(_, title)| title).as_deref(),
        Some("Welcome")
    );
}

#[test]
fn what_fades_out_before_a_note_is_gone_after_it() {
    let mut plan = Plan::new("Sunday");
    let song = plan.show(resource("Song", Kind::Video));
    let prayer = plan.push("Prayer", Action::Note);
    fading(&mut plan, song, 0, 4);
    let engine = Engine::new();
    engine.load_plan(plan);

    engine.next();
    let (scene, _) = on_program(&engine).unwrap();
    engine.next();
    assert!(engine.faded(scene));
    assert_eq!(current(&engine), Some(prayer));
    assert_eq!(engine.stage().program, None, "not back as it was");
}

#[test]
fn what_has_ended_or_is_not_the_steps_goes_at_once() {
    let (engine, song, picture) = song_then_picture();
    engine.next();
    let (scene, _) = on_program(&engine).unwrap();
    engine.report(scene, MediaReport::Ended);
    assert_eq!(engine.next(), Moved::Step(picture), "an ended song is over");

    engine.go_to(song).unwrap();
    engine.show_now(resource("Notice", Kind::Image).into());
    assert_eq!(
        engine.next(),
        Moved::Step(picture),
        "what is shown off the plan isn't the step's to fade"
    );
}

#[test]
fn going_elsewhere_while_it_fades_leaves_the_plan_there() {
    let mut plan = Plan::new("Sunday");
    let intro = plan.show(resource("Intro", Kind::Image));
    let song = plan.show(resource("Song", Kind::Video));
    let prayer = plan.push("Prayer", Action::Note);
    plan.show(resource("Welcome", Kind::Image));
    fading(&mut plan, song, 0, 4);
    let engine = Engine::new();
    engine.load_plan(plan);

    engine.go_to(song).unwrap();
    engine.next();
    assert_eq!(engine.prev(), Moved::Step(intro));
    assert_eq!(engine.stage().fading_out, None, "back, nothing fades");
    assert_eq!(seen(&engine).as_deref(), Some("Intro"));

    engine.go_to(song).unwrap();
    let (scene, _) = on_program(&engine).unwrap();
    engine.next();
    engine.go_to(prayer).unwrap();
    assert!(engine.faded(scene));
    assert_eq!(current(&engine), Some(prayer), "the plan stays at the note");
    assert_eq!(engine.stage().program, None, "and the song came off");
}

#[test]
fn a_step_cut_off_at_a_time_of_day_fades_out_first() {
    let at = |text: &str| TimeOfDay::parse(text).unwrap();
    let mut plan = Plan::new("Sabbath");
    let music = plan.section("Music", None);
    let song = plan.show(resource("Song", Kind::Video));
    plan.show(resource("Hymn", Kind::Image));
    plan.section("Sermon", None);
    let sermon = plan.show(resource("Sermon", Kind::Image));
    fading(&mut plan, song, 0, 6);
    let sharp = Flow {
        repeat: Repeat::Until(at("10:30")),
        ends: Ending::Sharp,
        ..Flow::default()
    };
    plan.set_flow(music, sharp).unwrap();
    let engine = Engine::new();
    engine.load_plan(plan);
    engine.tell_time(at("10:00").since_midnight());

    assert_eq!(engine.next(), Moved::Step(song));
    let (scene, _) = on_program(&engine).unwrap();
    engine.tell_time(at("10:30").since_midnight());
    assert!(engine.cut_now());
    assert_eq!(
        engine.next(),
        Moved::FadingOut(seconds(6).unwrap()),
        "cut as the app cuts when the time comes, it fades out first"
    );
    assert!(!engine.cut_now(), "it is being cut already");
    assert!(engine.faded(scene));
    assert_eq!(current(&engine), Some(sermon), "out of the loop");
}
