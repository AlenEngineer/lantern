//! A whole service, end to end: planned steps, a live slot, frames,
//! blackout, and what the stage window reports back.

use std::num::NonZeroUsize;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use lantern_engine::{
    Action, Capabilities, Engine, Kind, Location, MediaReport, Moved, Plan, Playback, Query,
    Resource, ResourceId, Source, SourceError, SourceId, StageEvent, StageFeed,
};

/// A tiny offline "hymnal": the kind of extension a real one would be.
struct Hymnal {
    id: SourceId,
}

impl Source for Hymnal {
    fn id(&self) -> &SourceId {
        &self.id
    }
    fn name(&self) -> &str {
        "Hymnal"
    }
    fn capabilities(&self) -> Capabilities {
        Capabilities::INDEXED
    }
    fn scan(&self) -> Result<Vec<Resource>, SourceError> {
        Ok(vec![
            hymn(
                &self.id,
                "1",
                "Santo, santo, santo",
                "¡Santo, santo, santo!\n\nCoro\n\nÚltima estrofa",
            ),
            hymn(&self.id, "2", "Cuán grande es Él", "Señor, mi Dios\n\nCoro"),
        ])
    }
}

fn hymn(source: &SourceId, number: &str, title: &str, lyrics: &str) -> Resource {
    Resource::new(
        ResourceId::new(source, number),
        title,
        Kind::Text,
        Location::Inline(lyrics.into()),
    )
    .with_tag(format!("himno {number}"))
}

fn resource(title: &str, kind: Kind, location: Location) -> Resource {
    let source = SourceId::new("test").unwrap();
    Resource::new(ResourceId::new(&source, title), title, kind, location)
}

fn image(title: &str) -> Resource {
    resource(title, Kind::Image, Location::Url(title.into()))
}

fn document(title: &str) -> Resource {
    resource(
        title,
        Kind::Document,
        Location::File(format!("/{title}.pdf").into()),
    )
}

fn video(title: &str) -> Resource {
    resource(
        title,
        Kind::Video,
        Location::File(format!("/{title}.mp4").into()),
    )
}

fn youtube_link() -> Resource {
    resource(
        "Worship video",
        Kind::Video,
        Location::Url("https://www.youtube.com/watch?v=x".into()),
    )
}

fn pages(n: usize) -> MediaReport {
    MediaReport::Frames(NonZeroUsize::new(n).unwrap())
}

fn secs(n: u64) -> Duration {
    Duration::from_secs(n)
}

fn playback(engine: &Engine) -> Option<Playback> {
    engine
        .stage()
        .program
        .and_then(|scene| scene.timed().map(|timed| timed.playback))
}

fn titles_logged(engine: &Engine) -> Vec<String> {
    engine.log().into_iter().map(|e| e.resource.title).collect()
}

#[test]
fn a_sunday_service() {
    let engine = Engine::new();
    engine
        .add_source(Hymnal {
            id: SourceId::new("hymnal").unwrap(),
        })
        .unwrap();
    assert_eq!(*engine.reindex()[0].result.as_ref().unwrap(), 2);

    // Prepare the plan: a planned hymn, a live slot, then prayer.
    let opening = engine.search_index(&Query::new("santo"))[0]
        .resource
        .clone();
    let mut plan = Plan::new("Sunday");
    let opening_step = plan.show((*opening).clone());
    let worship = plan.live("Worship", Some(Query::new("").with_kind(Kind::Text)));
    let prayer = plan.push("Prayer", Action::Blank);
    engine.load_plan(plan);

    let StageFeed { mut state, events } = engine.subscribe();

    // Opening hymn: the step, then its stanzas.
    assert_eq!(engine.next(), Moved::Step(opening_step));
    assert_eq!(engine.next(), Moved::Frame(1));
    assert_eq!(engine.next(), Moved::Frame(2));
    assert_eq!(engine.prev(), Moved::Frame(1));
    assert_eq!(engine.next(), Moved::Frame(2));

    // Live slot: the stage keeps the last stanza until we choose something.
    assert_eq!(engine.next(), Moved::Step(worship));
    assert_eq!(
        engine.stage().program.unwrap().resource.title,
        "Santo, santo, santo"
    );

    // The pastor asks for "Cuán grande es Él": search without accents, preview, take.
    let found = engine.search_index(&Query::new("cuan grande"))[0]
        .resource
        .clone();
    engine.preview(found);
    assert_eq!(
        engine.stage().program.unwrap().resource.title,
        "Santo, santo, santo"
    );
    assert!(engine.take());
    assert_eq!(
        engine.stage().program.unwrap().resource.title,
        "Cuán grande es Él"
    );

    // An off-plan image, then back to the plan.
    engine.show_now(Arc::new(image("Announcement")));
    assert!(engine.toggle_blackout());
    assert!(!engine.toggle_blackout());

    // `next` goes on to Prayer, which blanks the stage.
    assert_eq!(engine.next(), Moved::Step(prayer));
    assert!(engine.stage().program.is_none());
    assert_eq!(engine.next(), Moved::Nothing);

    // The live slot remembers what was shown in it.
    let plan = engine.plan();
    let Action::Live { shown, .. } = &plan.get(worship).unwrap().action else {
        panic!()
    };
    let titles: Vec<_> = shown.iter().map(|r| r.title.as_str()).collect();
    assert_eq!(titles, ["Cuán grande es Él", "Announcement"]);

    // The log has everything, planned or not.
    let log: Vec<_> = engine
        .log()
        .into_iter()
        .map(|e| (e.resource.title, e.planned))
        .collect();
    assert_eq!(
        log,
        [
            ("Santo, santo, santo".to_owned(), true),
            ("Cuán grande es Él".to_owned(), false),
            ("Announcement".to_owned(), false),
        ]
    );

    // And the stage window saw only what the audience should see, plus previews.
    let events: Vec<_> = events.try_iter().collect();
    let blackouts = events
        .iter()
        .filter(|e| matches!(e, StageEvent::Blackout { .. }))
        .count();
    assert_eq!(blackouts, 2);
    assert!(matches!(
        events.last(),
        Some(StageEvent::Program { scene: None })
    ));

    // A window that applied every event holds exactly the engine's stage.
    for event in &events {
        state.apply(event);
    }
    assert_eq!(state, engine.stage());
}

#[test]
fn resume_returns_to_the_plan_after_a_detour() {
    let engine = Engine::new();
    let mut plan = Plan::new("Sunday");
    let welcome = plan.show(image("Welcome"));
    engine.load_plan(plan);

    engine.next();
    engine.show_now(Arc::new(image("Oops, something else")));
    assert_eq!(engine.resume(), Some(welcome));
    assert_eq!(engine.stage().program.unwrap().resource.title, "Welcome");
}

#[test]
fn live_finds_can_join_the_plan() {
    let engine = Engine::new();
    let mut plan = Plan::new("Sunday");
    plan.show(image("Welcome"));
    plan.show(image("Closing"));
    engine.load_plan(plan);

    engine.next();
    let added = engine.add_resource(None, &image("Pastor's request"));
    assert_eq!(engine.up_next().unwrap().id, added);
    assert_eq!(engine.next(), Moved::Step(added));
    assert!(engine.go_to(added).is_ok());
}

#[test]
fn a_late_page_count_only_reaches_its_scene() {
    let engine = Engine::new();
    engine.show_now(Arc::new(document("Bulletin")));
    let bulletin = engine.stage().program.unwrap().id;
    engine.show_now(Arc::new(image("Announcement")));

    // The stage finishes rasterizing the bulletin after it has left.
    assert!(!engine.report(bulletin, pages(3)));
    assert_eq!(engine.next(), Moved::Nothing);
}

#[test]
fn the_stage_reports_how_a_video_plays() {
    let engine = Engine::new();
    engine.show_now(Arc::new(video("Welcome")));
    let welcome = engine.stage().program.unwrap().id;
    assert_eq!(
        playback(&engine),
        Some(Playback::Playing),
        "going on stage is the cue to play"
    );
    engine.report(
        welcome,
        MediaReport::Timeline {
            duration: Some(secs(30)),
        },
    );
    assert_eq!(
        playback(&engine),
        Some(Playback::Playing),
        "a report doesn't pause it"
    );
    assert!(engine.pause());
    assert_eq!(playback(&engine), Some(Playback::Paused));
    assert!(engine.play());
    engine.report(welcome, MediaReport::Position(secs(29)));
    engine.report(welcome, MediaReport::Ended);
    assert_eq!(playback(&engine), Some(Playback::Ended));

    let feed = engine.subscribe();
    assert!(engine.play());
    let events: Vec<_> = feed.events.try_iter().collect();
    assert_eq!(
        events,
        [
            StageEvent::Seek {
                position: Duration::ZERO
            },
            StageEvent::Playback {
                playback: Playback::Playing
            },
        ]
    );
}

#[test]
fn a_link_the_stage_cannot_play_has_no_timeline() {
    let engine = Engine::new();
    engine.show_now(Arc::new(youtube_link()));
    let link = engine.stage().program.unwrap().id;
    assert!(engine.report(
        link,
        MediaReport::Failed {
            reason: "this stage can't play links".into()
        }
    ));
    assert!(!engine.play());
    assert!(!engine.toggle_playback());
    assert!(!engine.seek(secs(5)));
}

#[test]
fn controls_and_reports_never_reach_the_log() {
    let engine = Engine::new();
    let clip = Arc::new(video("Welcome"));
    engine.show_now(Arc::clone(&clip));
    let id = engine.stage().program.unwrap().id;

    engine.report(id, MediaReport::Timeline { duration: None });
    engine.toggle_playback();
    engine.report(id, MediaReport::Position(secs(3)));
    engine.seek(secs(10));
    engine.pause();
    engine.set_blackout(true);
    engine.set_blackout(true);
    engine.goto_frame(0);
    engine.preview(Arc::new(image("Later")));
    engine.clear_preview();
    assert_eq!(titles_logged(&engine), ["Welcome"]);

    engine.show_now(clip);
    assert_eq!(titles_logged(&engine), ["Welcome", "Welcome"]);
    engine.clear();
    assert_eq!(titles_logged(&engine), ["Welcome", "Welcome"]);
}

#[test]
fn a_preview_report_never_reaches_program() {
    let engine = Engine::new();
    engine.show_now(Arc::new(document("Bulletin")));
    engine.preview(Arc::new(document("Sermon notes")));
    let stage = engine.stage();
    let bulletin = stage.program.unwrap().id;
    let notes = stage.preview.unwrap().id;

    assert!(engine.report(notes, pages(3)));
    assert_eq!(engine.next(), Moved::Nothing);

    assert!(engine.take());
    let on_air = engine.stage().program.unwrap();
    assert_ne!(on_air.id, notes);
    assert!(on_air.has_next_frame());
    assert!(!engine.report(notes, pages(1)));
    assert!(!engine.report(bulletin, pages(1)));
    assert_eq!(engine.next(), Moved::Frame(1));
    assert_eq!(titles_logged(&engine), ["Bulletin", "Sermon notes"]);
}

#[test]
fn a_window_opened_mid_service_converges() {
    let engine = Arc::new(Engine::new());
    let writer = {
        let engine = Arc::clone(&engine);
        thread::spawn(move || {
            for round in 0..200 {
                match round % 5 {
                    0 => engine.show_now(Arc::new(document("Bulletin"))),
                    1 => engine.preview(Arc::new(video("Welcome"))),
                    2 => {
                        engine.take();
                    }
                    3 => {
                        engine.toggle_blackout();
                    }
                    _ => {}
                }
                let stage = engine.stage();
                if let Some(scene) = stage.program {
                    engine.report(scene.id, pages(round % 4 + 1));
                    engine.report(
                        scene.id,
                        MediaReport::Timeline {
                            duration: Some(secs(60)),
                        },
                    );
                    engine.toggle_playback();
                    engine.report(scene.id, MediaReport::Position(secs(round as u64 % 60)));
                    engine.next();
                }
                if let Some(scene) = stage.preview {
                    engine.report(scene.id, MediaReport::Ended);
                }
            }
        })
    };

    let mut feeds = Vec::new();
    while !writer.is_finished() {
        feeds.push(engine.subscribe());
        thread::yield_now();
    }
    writer.join().unwrap();
    feeds.push(engine.subscribe());

    let end = engine.stage();
    for StageFeed { mut state, events } in feeds {
        let snapshot = state.program.as_ref().map(|scene| scene.id);
        let mut first_program = None;
        for event in events.try_iter() {
            if let StageEvent::Program { scene: Some(scene) } = &event {
                first_program.get_or_insert(scene.id);
            }
            state.apply(&event);
        }
        if let (Some(snapshot), Some(first)) = (snapshot, first_program) {
            assert!(
                first > snapshot,
                "ids only grow, so no event predates its snapshot"
            );
        }
        assert_eq!(state, end);
    }
}

#[test]
fn a_video_that_goes_on_by_itself_moves_the_plan_when_it_ends() {
    let engine = Engine::new();
    let mut plan = Plan::new("Sunday");
    let intro = plan.show(video("Intro"));
    let welcome = plan.show(image("Welcome"));
    let outro = plan.show(video("Outro"));
    plan.get_mut(intro).unwrap().advance = true;
    engine.load_plan(plan);

    engine.next();
    let scene = engine.stage().program.unwrap().id;
    assert_eq!(playback(&engine), Some(Playback::Playing));
    engine.report(scene, MediaReport::Ended);
    assert_eq!(
        engine.current_step().map(|s| s.id),
        Some(welcome),
        "it went on"
    );

    // Without the switch, an ended video waits for the operator.
    engine.next();
    let scene = engine.stage().program.unwrap().id;
    engine.report(scene, MediaReport::Ended);
    assert_eq!(engine.current_step().map(|s| s.id), Some(outro));
    assert_eq!(playback(&engine), Some(Playback::Ended));
}

#[test]
fn a_steps_sound_plays_from_its_slide_and_stops_with_the_step() {
    use lantern_engine::StepSound;

    let source = SourceId::new("test").unwrap();
    let music = Resource::new(
        ResourceId::new(&source, "music"),
        "Instrumental",
        Kind::Audio,
        Location::File("/music.mp3".into()),
    );
    let first = hymn(&source, "1", "Santo", "Uno\n\nDos\n\nTres");
    let second = hymn(&source, "2", "Grande", "Uno");

    let engine = Engine::new();
    let step = engine.edit_plan(|plan| {
        let step = plan.show(first.clone());
        plan.push("Oración", Action::Note);
        plan.show(second.clone());
        plan.get_mut(step).unwrap().sound = Some(StepSound {
            resource: music.clone(),
            from: 1,
        });
        step
    });
    let on = |engine: &Engine| {
        let stage = engine.stage();
        let program = stage
            .program
            .as_ref()
            .map(|scene| scene.resource.title.clone());
        let picture = stage
            .picture()
            .map(|scene| (scene.resource.title.clone(), scene.frame()));
        (program, picture)
    };
    let hymn_at = |frame| Some(("Santo".to_string(), frame));

    // The first slide: no sound yet.
    assert_eq!(engine.next(), Moved::Step(step));
    assert_eq!(on(&engine), (Some("Santo".into()), hymn_at(0)));

    // The slide it starts from: the sound goes on over it, playing.
    assert_eq!(engine.next(), Moved::Frame(1));
    assert_eq!(on(&engine), (Some("Instrumental".into()), hymn_at(1)));
    let sound = engine.stage().program.unwrap();
    assert_eq!(sound.timed().unwrap().playback, Playback::Playing);

    // The slides go on under the same sound.
    assert_eq!(engine.next(), Moved::Frame(2));
    assert_eq!(engine.stage().program.unwrap().id, sound.id);

    // Back before its slide: it stops, and the slide is as it was.
    assert_eq!(engine.prev(), Moved::Frame(1));
    assert_eq!(engine.prev(), Moved::Frame(0));
    assert_eq!(on(&engine), (Some("Santo".into()), hymn_at(0)));

    // On again: it starts over.
    assert_eq!(engine.next(), Moved::Frame(1));
    let again = engine.stage().program.unwrap();
    assert_eq!(again.resource.title, "Instrumental");
    assert_ne!(again.id, sound.id);

    // A sound the operator puts on instead is left alone.
    let other = Resource::new(
        ResourceId::new(&source, "other"),
        "Otra",
        Kind::Audio,
        Location::File("/other.mp3".into()),
    );
    engine.show_now(Arc::new(other));
    assert_eq!(engine.next(), Moved::Frame(2));
    assert_eq!(on(&engine), (Some("Otra".into()), hymn_at(2)));

    // The step ends at a note, which leaves the stage as it is: the
    // slide stays. The operator's sound isn't the step's to stop.
    assert!(matches!(engine.next(), Moved::Step(_)));
    assert_eq!(on(&engine), (Some("Otra".into()), hymn_at(2)));

    // Again, with the step's own sound: it stops with the step.
    engine.go_to(step).unwrap();
    assert_eq!(engine.next(), Moved::Frame(1));
    assert_eq!(engine.next(), Moved::Frame(2));
    assert!(matches!(engine.next(), Moved::Step(_)));
    assert_eq!(on(&engine), (Some("Santo".into()), hymn_at(2)));

    // And the next thing shown takes its place as ever.
    assert!(matches!(engine.next(), Moved::Step(_)));
    assert_eq!(
        on(&engine),
        (Some("Grande".into()), Some(("Grande".into(), 0)))
    );
}

#[test]
fn a_steps_sound_from_the_first_slide_starts_with_the_step() {
    use lantern_engine::StepSound;

    let source = SourceId::new("test").unwrap();
    let music = Resource::new(
        ResourceId::new(&source, "music"),
        "Instrumental",
        Kind::Audio,
        Location::File("/music.mp3".into()),
    );
    let engine = Engine::new();
    engine.edit_plan(|plan| {
        let step = plan.show(hymn(&source, "1", "Santo", "Uno\n\nDos"));
        plan.get_mut(step).unwrap().sound = Some(StepSound {
            resource: music,
            from: 0,
        });
    });
    engine.next();
    let stage = engine.stage();
    assert_eq!(
        stage.program.as_ref().unwrap().resource.title,
        "Instrumental"
    );
    assert_eq!(stage.picture().unwrap().resource.title, "Santo");

    // Saved with the plan; the first slide goes without saying.
    let json = engine.plan().to_json();
    assert!(json.contains("\"sound\""), "{json}");
    assert!(!json.contains("\"from\""), "{json}");
    let back = Plan::from_json(&json).unwrap();
    assert_eq!(back.steps()[0].sound, engine.plan().steps()[0].sound);
}
