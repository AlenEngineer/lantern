//! A whole service, end to end: planned steps, a live slot, frames, blackout.

use std::sync::Arc;

use chorquest_engine::{
    Action, Capabilities, Engine, Kind, Location, Moved, Plan, Query, Resource, ResourceId, Source,
    SourceError, SourceId, StageEvent,
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

fn image(title: &str) -> Resource {
    let source = SourceId::new("test").unwrap();
    Resource::new(
        ResourceId::new(&source, title),
        title,
        Kind::Image,
        Location::Url(title.into()),
    )
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

    let stage = engine.subscribe();

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
    engine.with_stage(|s| s.toggle_blackout());
    engine.with_stage(|s| s.toggle_blackout());

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
    let events: Vec<_> = stage.try_iter().collect();
    let blackouts = events
        .iter()
        .filter(|e| matches!(e, StageEvent::Blackout { .. }))
        .count();
    assert_eq!(blackouts, 2);
    assert!(matches!(
        events.last(),
        Some(StageEvent::Program { scene: None })
    ));
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
    let added = engine.add_after_current(&image("Pastor's request"));
    assert_eq!(engine.up_next().unwrap().id, added);
    assert_eq!(engine.next(), Moved::Step(added));
    assert!(engine.go_to(added).is_ok());
}
