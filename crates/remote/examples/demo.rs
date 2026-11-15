//! Serves the page with a made-up service, to look at it without the app:
//! `cargo run -p lantern-remote --example demo`, then open the address it
//! prints. Commands are printed, and Next walks through the plan.

use std::sync::mpsc;

use lantern_remote::{
    Colors, Command, Live, Place, Plan, Server, Slides, Snapshot, Step, StepKind, Timed, Upcoming,
};

fn snapshot(at: usize, blackout: bool, playing: bool) -> Snapshot {
    let rows = [
        (StepKind::Section, "Servicio de Canto", ""),
        (
            StepKind::Show,
            "Himno 99 Jamás podrá alguien separarnos",
            "3:07",
        ),
        (StepKind::Show, "Himno 465 Ven junto a mí", "2:41"),
        (StepKind::Show, "Himno 534 En tu nombre comenzamos", "2:22"),
        (StepKind::Section, "Escuela Sabática", ""),
        (StepKind::Show, "Himno 66 ¿Sabes cuántos?", "2:12"),
        (StepKind::Live, "Lección: búsqueda en vivo", "live"),
        (StepKind::Blank, "Oración", "black"),
        (StepKind::Show, "Juan 3:16", "~30s"),
        (StepKind::Section, "Culto Divino", ""),
        (StepKind::Show, "Bienvenida", "~1 min"),
        (StepKind::Show, "Sermón: Cimentados en la Biblia", "~35 min"),
    ];
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
            problem: (row == 3).then(|| "file missing".to_string()),
        })
        .collect();
    let verse = rows[current].1 == "Juan 3:16";
    Snapshot {
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
        }),
        next: Some(Upcoming {
            title: rows[shown[(at + 1) % shown.len()]].1.into(),
        }),
        blackout,
        off_plan: false,
        notes: vec![
            "Escuela Sabática · 4:10 / 30m".into(),
            "4 min behind".into(),
        ],
    }
}

fn main() -> std::io::Result<()> {
    let (sent, commands) = mpsc::channel();
    let port = std::env::args().nth(1).and_then(|port| port.parse().ok());
    let server = Server::start(port.unwrap_or(7700), "demo", move |command| {
        let _ = sent.send(command);
    })?;
    println!("http://localhost:{}/?t=demo", server.port());
    let (mut at, mut blackout, mut playing) = (4, false, true);
    server.publish(&snapshot(at, blackout, playing));
    for command in commands {
        println!("{command:?}");
        match command {
            Command::Next => at += 1,
            Command::Prev => at = at.saturating_sub(1),
            Command::Blackout => blackout = !blackout,
            Command::PlayPause => playing = !playing,
            _ => {}
        }
        server.publish(&snapshot(at, blackout, playing));
    }
    Ok(())
}
