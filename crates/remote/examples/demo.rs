//! Serves the page with a made-up service, to look at it without the app:
//! `cargo run -p lantern-remote --example demo [port] [plan|run|live]`,
//! then open the address it prints. Commands are printed, and Next walks
//! through the plan.

use std::collections::HashMap;
use std::sync::{Arc, mpsc};

use lantern_remote::{
    Badge, Bang, BibleBook, BibleBooks, Colors, Command, Found, Group, Hit, HitKind, Listed, Live,
    Mode, Note, Pace, Place, Plan, Server, Shown, Slides, Snapshot, Sound, Source, SourceKind,
    Starts, Step, StepKind, Timed, Tone, Upcoming,
};
use lantern_source_bible::{Book, Language};

/// The results' pictures: a wide photo, and a tall one for a result
/// whose shape is under 1.
const PHOTO: &[u8] = include_bytes!("../../../scripts/e2e/fixtures/photo.jpg");
const PORTRAIT: &[u8] = include_bytes!("../../../scripts/e2e/fixtures/portrait.jpg");

fn hit(title: &str, facts: &str) -> Hit {
    Hit {
        id: title.into(),
        title: title.into(),
        facts: facts.into(),
        ..Hit::default()
    }
}

fn badge(text: &str, tone: Tone) -> Option<Badge> {
    Some(Badge {
        text: text.into(),
        tone,
    })
}

/// A result of `kind`, its size `size` in the corner, and a picture of
/// `shape` where it has one.
fn kind(hit: Hit, kind: HitKind, size: &str, shape: Option<f32>) -> Hit {
    Hit {
        kind,
        size: Some(size.to_string()).filter(|size| !size.is_empty()),
        shape,
        picture: shape.is_some(),
        ..hit
    }
}

/// One of each kind of result, in each state a result can be in.
fn every_kind() -> Vec<Group> {
    let wide = Some(16.0 / 9.0);
    let screen = "Smaller than the screen (1920×1080): it will look soft";
    vec![
        Group {
            name: "Himnario".into(),
            error: None,
            hits: vec![
                kind(hit("Himno 316 Divino pastor", "Himnario · video"), HitKind::Video, "2:48", wide),
                Hit {
                    warning: badge("file missing", Tone::Problem),
                    ..kind(hit("Himno 99 Jamás podrá alguien separarnos", "Himnario · video"), HitKind::Video, "", None)
                },
                kind(hit("Cimentados en la Biblia", "Sermones · slides"), HitKind::Deck, "12 slides", wide),
                kind(hit("Boletín de octubre", "Anuncios · document"), HitKind::Pdf, "8 pages", Some(1080.0 / 1350.0)),
                kind(hit("Preludio", "Himnario · sound"), HitKind::Sound, "2:10", None),
            ],
        },
        Group {
            name: "Fotos".into(),
            error: None,
            hits: vec![
                kind(hit("Atardecer en el lago", "Fotos · picture"), HitKind::Picture, "1920×1080", wide),
                Hit {
                    state: badge("⚠", Tone::Waiting),
                    warning: badge(screen, Tone::Waiting),
                    ..kind(hit("Logo de la iglesia", "Fotos · picture"), HitKind::Picture, "800×600", Some(4.0 / 3.0))
                },
            ],
        },
        Group {
            name: "YouTube".into(),
            error: None,
            hits: vec![
                Hit {
                    state: badge("☁ online", Tone::Plain),
                    ..kind(hit("Himno 465 Ven junto a mí", "YouTube · video"), HitKind::Video, "2:41", wide)
                },
                Hit {
                    state: badge("↓ 40%", Tone::Plain),
                    progress: Some(0.4),
                    ..kind(hit("Himno 534 En tu nombre comenzamos", "YouTube · video"), HitKind::Video, "2:22", wide)
                },
                Hit {
                    state: badge("● live", Tone::Live),
                    ..kind(hit("Culto en vivo", "YouTube · video"), HitKind::Video, "", wide)
                },
            ],
        },
        Group {
            name: "RVG".into(),
            error: None,
            hits: vec![Hit {
                text: "Porque de tal manera amó Dios al mundo, que ha dado a su Hijo unigénito, para que todo aquel que en Él cree, no se pierda, mas tenga vida eterna.".into(),
                ..kind(hit("Juan 3:16-18", "Reina-Valera Gómez"), HitKind::Passage, "3 verses", None)
            }],
        },
        Group {
            name: "Time".into(),
            error: None,
            hits: vec![kind(hit("Countdown 5 min", "counts down from when it's shown"), HitKind::Time, "5:00", None)],
        },
    ]
}

/// The pictures of `snapshot`'s results that have one.
fn thumbs(snapshot: &Snapshot) -> HashMap<String, Arc<Vec<u8>>> {
    let (photo, portrait) = (Arc::new(PHOTO.to_vec()), Arc::new(PORTRAIT.to_vec()));
    let found = snapshot.search.iter().flat_map(|found| &found.groups);
    let hits = found.flat_map(|group| &group.hits);
    let listed = snapshot.listed.iter().flat_map(|listed| &listed.hits);
    hits.chain(listed)
        .filter(|hit| hit.picture)
        .map(|hit| {
            let jpeg = match hit.shape {
                Some(shape) if shape < 1.0 => &portrait,
                _ => &photo,
            };
            (hit.id.clone(), jpeg.clone())
        })
        .collect()
}

/// What the Bible, the clock and typed text make of `text`, as the app's
/// sources would.
fn made(text: &str) -> Option<Vec<Hit>> {
    if let Some(reference) = text.strip_prefix("!b ") {
        let verses = [
            "Porque de tal manera amó Dios al mundo, que ha dado a su Hijo unigénito, para que todo aquel que en él cree, no se pierda, mas tenga vida eterna.",
            "Porque no envió Dios a su Hijo al mundo para que condene al mundo, mas para que el mundo sea salvo por él.",
            "El que en él cree, no es condenado; mas el que no cree, ya es condenado.",
        ];
        let (first, last) = reference
            .rsplit_once(':')
            .map(|(_, span)| span.split_once('-').unwrap_or((span, span)))
            .and_then(|(a, b)| Some((a.parse::<usize>().ok()?, b.parse::<usize>().ok()?)))?;
        let text = (first..=last)
            .map(|n| verses[n % verses.len()])
            .collect::<Vec<_>>()
            .join("\n\n");
        let verses = last + 1 - first.min(last);
        let size = match verses {
            1 => "1 verse".to_string(),
            n => format!("{n} verses"),
        };
        return Some(vec![Hit {
            text,
            ..kind(
                hit(reference, "Reina-Valera Gómez"),
                HitKind::Passage,
                &size,
                None,
            )
        }]);
    }
    if let Some(asked) = text.strip_prefix("!t ") {
        let time = |hit: Hit, size: &str| kind(hit, HitKind::Time, size, None);
        return Some(match asked {
            "clock" => vec![time(hit("Clock", "the time of day"), "")],
            "timer" => vec![time(hit("Timer", "counts up from when it's shown"), "")],
            at if at.contains(':') => vec![
                time(
                    hit(
                        &format!("Countdown {at}"),
                        "counts down from when it's shown",
                    ),
                    at,
                ),
                time(
                    hit(
                        &format!("Countdown to {at}"),
                        &format!("counts down to {at}"),
                    ),
                    "",
                ),
            ],
            minutes => vec![time(
                hit(
                    &format!("Countdown {minutes} min"),
                    "counts down from when it's shown",
                ),
                &format!("{minutes}:00"),
            )],
        });
    }
    if let Some(typed) = text.strip_prefix("!x ") {
        let slides = typed.split(" || ").count();
        return Some(vec![Hit {
            text: typed.replace(" || ", "\n\n").replace(" | ", "\n"),
            kind: HitKind::Text,
            ..hit(
                &typed.replace(" | ", " "),
                &format!("your text · {slides} slides"),
            )
        }]);
    }
    None
}

/// What a search for `text` finds: the same few verses whatever it is.
fn found(text: &str) -> Option<Found> {
    (!text.is_empty()).then(|| Found {
        text: text.into(),
        busy: false,
        waiting: text.starts_with("!yt "),
        groups: if let Some(hits) = made(text) {
            vec![Group {
                name: "Made".into(),
                error: None,
                hits,
            }]
        } else if text.starts_with("!yt ") {
            Vec::new()
        } else {
            every_kind()
        },
    })
}

const ROWS: [(StepKind, &str, &str); 12] = [
    (StepKind::Section, "Servicio de Canto", "15 min"),
    (
        StepKind::Show,
        "Himno 99 Jamás podrá alguien separarnos",
        "3:07",
    ),
    (StepKind::Show, "Himno 465 Ven junto a mí", "2:41"),
    (StepKind::Show, "Himno 534 En tu nombre comenzamos", "2:22"),
    (StepKind::Section, "Escuela Sabática", "30 min"),
    (StepKind::Show, "Himno 66 ¿Sabes cuántos?", "2:12"),
    (StepKind::Live, "Lección: búsqueda en vivo", "live"),
    (StepKind::Blank, "Oración", "black"),
    (StepKind::Show, "Juan 3:16", "~30s"),
    (StepKind::Section, "Culto Divino", "1 h"),
    (StepKind::Show, "Bienvenida", "~1 min"),
    (StepKind::Show, "Sermón: Cimentados en la Biblia", "~35 min"),
];

struct Demo {
    listed: bool,
    filter: String,
    mode: Mode,
    at: usize,
    blackout: bool,
    playing: bool,
    text: String,
    sound: Sound,
}

fn snapshot(demo: &Demo) -> Snapshot {
    let Demo {
        mode,
        at,
        blackout,
        playing,
        ref text,
        ref sound,
        ..
    } = *demo;
    let rows = ROWS;
    let shown: Vec<usize> = (0..rows.len())
        .filter(|row| rows[*row].0 != StepKind::Section)
        .collect();
    let current = shown[at % shown.len()];
    let steps = rows
        .iter()
        .enumerate()
        .map(|(row, (kind, label, detail))| Step {
            id: row as u64,
            label: (*label).into(),
            kind: *kind,
            place: match row.cmp(&current) {
                std::cmp::Ordering::Less => Place::Done,
                std::cmp::Ordering::Equal => Place::Current,
                std::cmp::Ordering::Greater => Place::Ahead,
            },
            detail: (*detail).into(),
            pace: if row == 0 { Pace::Tight } else { Pace::OnTrack },
            problem: (row == 3).then(|| "file missing".to_string()),
        })
        .collect();
    let verse = rows[current].1 == "Juan 3:16";
    let next = shown[(at + 1) % shown.len()];
    Snapshot {
        mode,
        starts: Some(Starts {
            at: "10:30".into(),
            in_seconds: Some(24 * 60 + 13),
        }),
        colors: Colors {
            ground: "#1a1113".into(),
            card: "#261a1d".into(),
            raised: "#3d2f32".into(),
            text: "#f0dee1".into(),
            dim: "#d3c2c5".into(),
            accent: "#ffb1c8".into(),
            on_accent: "#561d32".into(),
            live: "#efbd94".into(),
            danger: "#ffb4ab".into(),
        },
        plan: Some(Plan {
            name: "Sábado".into(),
            steps,
        }),
        live: Some(Live {
            title: rows[current].1.into(),
            facts: if verse { "RVG" } else { "Himnario · video" }.into(),
            timed: (!verse).then_some(Timed {
                playing,
                ended: false,
                position: 74,
                length: Some(187),
            }),
            slides: verse.then_some(Slides { at: 1, of: Some(2) }),
            failed: None,
            attached: if verse {
                vec!["Video".into(), "YouTube video".into()]
            } else {
                Vec::new()
            },
            back: (!verse).then_some(3),
        }),
        next: Some(Upcoming {
            title: rows[next].1.into(),
            detail: rows[next].2.into(),
        }),
        blackout,
        off_plan: false,
        notes: vec![
            Note {
                text: "Servicio de Canto · 4:10 left".into(),
                pace: Pace::Tight,
            },
            Note {
                text: "4 min behind".into(),
                pace: Pace::OnTrack,
            },
        ],
        bangs: [
            ("b", "RVG, RVG 2004"),
            ("yt", "YouTube"),
            ("h", "Music"),
            ("x", "Text"),
            ("t", "Time"),
            ("gi", "Images"),
        ]
        .iter()
        .map(|(bang, name)| Bang {
            bang: (*bang).into(),
            name: (*name).into(),
        })
        .collect(),
        search: found(text),
        sound: Some(sound.clone()),
        sources: live_sources(),
        bible: Some(BibleBooks {
            name: "RVG".into(),
            books: Book::all()
                .map(|book| BibleBook {
                    name: book.name(Language::Spanish).into(),
                    short: book.short(Language::Spanish),
                    verses: if book.osis() == "John" {
                        vec![
                            51, 25, 36, 54, 47, 71, 53, 59, 41, 42, 57, 50, 38, 31, 27, 33, 26, 40,
                            42, 31, 25,
                        ]
                    } else {
                        vec![30; 12]
                    },
                })
                .collect(),
        }),
        shown: [
            "Bienvenida",
            "Himno 99 Jamás podrá alguien separarnos",
            "Juan 3:16",
        ]
        .iter()
        .enumerate()
        .map(|(n, title)| Shown {
            n,
            at: format!("19:0{}", n * 3),
            title: (*title).into(),
            live: n == 2,
        })
        .collect(),
        listed: demo.listed.then(|| Listed {
            folder: 1,
            filter: demo.filter.clone(),
            files: 14,
            matching: 14,
            hits: (1..=14)
                .map(|n| {
                    let title = format!("{n:03} Himno {n}");
                    kind(
                        hit(&title, "Himnario · video"),
                        HitKind::Video,
                        "3:07",
                        Some(16.0 / 9.0),
                    )
                })
                .collect(),
        }),
    }
}

fn live_sources() -> Vec<Source> {
    let tile = |kind, name: &str, bang: &str, hint: &str, folder| Source {
        kind,
        name: name.into(),
        bang: bang.into(),
        hint: hint.into(),
        folder,
    };
    vec![
        tile(SourceKind::Bible, "Bible", "b", "RVG · by book", None),
        tile(
            SourceKind::Folder,
            "Himnario",
            "h",
            "your folder · !h",
            Some(1),
        ),
        tile(SourceKind::Youtube, "YouTube", "yt", "search videos", None),
        tile(SourceKind::Text, "Text", "x", "type a slide", None),
        tile(SourceKind::Time, "Time", "t", "clock, countdowns", None),
        tile(SourceKind::Images, "Images", "gi", "web pictures", None),
    ]
}

fn main() -> std::io::Result<()> {
    let (sent, commands) = mpsc::channel();
    let mut args = std::env::args().skip(1);
    let port = args.next().and_then(|port| port.parse().ok());
    let mode = match args.next().as_deref() {
        Some("plan") => Mode::Plan,
        Some("live") => Mode::Live,
        _ => Mode::Run,
    };
    let server = Server::start(port.unwrap_or(7700), "demo", move |command| {
        let _ = sent.send(command);
    })?;
    println!("http://localhost:{}/?t=demo", server.port());
    let mut demo = Demo {
        listed: false,
        filter: String::new(),
        mode,
        at: 0,
        blackout: false,
        playing: true,
        text: String::new(),
        sound: Sound {
            output: "HD Audio Speaker".into(),
            volume: 0.62,
            muted: false,
        },
    };
    let publish = |demo: &Demo| {
        let snapshot = snapshot(demo);
        server.set_thumbs(thumbs(&snapshot));
        server.publish(&snapshot);
    };
    publish(&demo);
    for command in commands {
        println!("{command:?}");
        match command {
            Command::Next => demo.at += 1,
            Command::Prev => demo.at = demo.at.saturating_sub(1),
            Command::Go { step } => {
                let upto = ROWS.iter().take(step as usize + 1);
                demo.at = upto
                    .filter(|row| row.0 != StepKind::Section)
                    .count()
                    .saturating_sub(1);
            }
            Command::Blackout => demo.blackout = !demo.blackout,
            Command::PlayPause => demo.playing = !demo.playing,
            Command::Search { text, .. } => demo.text = text,
            Command::Browse { folder, filter } => {
                demo.listed = folder.is_some();
                demo.filter = filter;
            }
            Command::Volume { level } => {
                demo.sound.volume = level.clamp(0.0, 1.0);
                demo.sound.muted = false;
            }
            Command::Mute { on } => demo.sound.muted = on,
            _ => {}
        }
        publish(&demo);
    }
    Ok(())
}
