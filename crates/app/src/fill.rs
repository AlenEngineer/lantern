//! Filling a section: choosing, from what a library holds, the things
//! that come to a time, or to a number of things, or both.
//!
//! This is the choosing alone. It knows lengths and nothing else: what
//! the things are, where they come from and how they are shown is the
//! caller's. It proposes; nothing here adds to a plan.

use std::time::Duration;

/// How the total may stand to the time asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fit {
    /// Never over it, and no more under than the margin.
    AtMost,
    /// As near it as can be.
    Exact,
    /// Never under it, and no more over than the margin.
    AtLeast,
    /// Either side of it, by no more than the margin.
    Either,
}

/// The order things are taken in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Order {
    /// As the library holds them, none passed over: the first that
    /// would go past the time ends it.
    Held,
    /// Drawn at random.
    Drawn,
    /// Drawn with what is used most likeliest: see [`Thing::weight`].
    Usual,
}

/// What is asked for: a time to come to, a number of things, or both.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Wish {
    pub time: Option<Duration>,
    pub count: Option<usize>,
    pub fit: Fit,
    pub margin: Duration,
}

impl Wish {
    /// The least and the most the total may be, with a time asked for.
    pub fn window(&self) -> Option<(Duration, Duration)> {
        let time = self.time?;
        let under = time.saturating_sub(self.margin);
        let over = time + self.margin;
        Some(match self.fit {
            Fit::AtMost => (under, time),
            Fit::Exact => (time, time),
            Fit::AtLeast => (time, over),
            Fit::Either => (under, over),
        })
    }
}

/// One thing that could be chosen.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Thing {
    /// How long it plays. `None` for something still, a picture or a
    /// passage: it takes whatever share of the time is left over.
    pub length: Option<Duration>,
    /// How likely [`Order::Usual`] is to draw it: how often it has been
    /// used. 0 is something never used, drawn only when the rest can't
    /// make the time.
    pub weight: f32,
}

/// What was chosen.
#[derive(Debug, Clone, PartialEq)]
pub struct Chosen {
    /// Which, by their place among the things offered, in the order they
    /// would go into the plan.
    pub picks: Vec<usize>,
    /// What the ones with a length of their own add up to.
    pub total: Duration,
    /// The length each still thing gets, when a time was asked for and
    /// some were chosen: what is left of it, shared evenly.
    pub each_still: Option<Duration>,
    /// How far the total is from what was asked: zero when it fits.
    pub off: Duration,
}

/// A small generator, so a draw can be made again from its seed.
struct Draw(u64);

impl Draw {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    /// A number from 0 up to, not including, 1.
    fn part(&mut self) -> f32 {
        (self.next() >> 40) as f32 / (1u64 << 24) as f32
    }
}

/// How far `total` is outside the window: zero inside it.
fn off(total: Duration, window: Option<(Duration, Duration)>) -> Duration {
    match window {
        Some((least, _)) if total < least => least - total,
        Some((_, most)) if total > most => total - most,
        _ => Duration::ZERO,
    }
}

fn total(things: &[Thing], picks: &[usize]) -> Duration {
    picks.iter().filter_map(|&at| things[at].length).sum()
}

/// Chooses among `things` for `wish`. `kept` are chosen already and stay
/// (by their place among the things); `seed` makes a draw that can be
/// made again, and another seed another draw.
pub fn choose(things: &[Thing], wish: Wish, order: Order, kept: &[usize], seed: u64) -> Chosen {
    let window = wish.window();
    let free: Vec<usize> = (0..things.len()).filter(|at| !kept.contains(at)).collect();
    let picks = match order {
        Order::Held => held(things, wish, kept, &free),
        Order::Drawn | Order::Usual => {
            let mut draw = Draw(seed | 1);
            // Several draws, each then bettered one swap at a time: the
            // nearest to what was asked is the one.
            (0..24)
                .map(|_| drawn(things, wish, order, kept, &free, &mut draw))
                .min_by_key(|picks| {
                    let short = wish.count.map_or(0, |count| count.abs_diff(picks.len()));
                    (short, off(total(things, picks), window))
                })
                .unwrap_or_else(|| kept.to_vec())
        }
    };
    let sum = total(things, &picks);
    let still = picks
        .iter()
        .filter(|&&at| things[at].length.is_none())
        .count();
    Chosen {
        each_still: wish
            .time
            .filter(|_| still > 0)
            .map(|time| time.saturating_sub(sum) / still as u32),
        // Still things take up what is left: with any chosen, a total
        // short of the time is no longer short.
        off: match (still, window) {
            (1.., Some((_, most))) if sum <= most => Duration::ZERO,
            _ => off(sum, window),
        },
        total: sum,
        picks,
    }
}

/// In the order held, none passed over.
fn held(things: &[Thing], wish: Wish, kept: &[usize], free: &[usize]) -> Vec<usize> {
    let window = wish.window();
    let mut picks = kept.to_vec();
    let mut sum = total(things, &picks);
    for &at in free {
        if wish.count.is_some_and(|count| picks.len() >= count) {
            break;
        }
        // Enough already, with no number to reach.
        if wish.count.is_none() && window.is_some_and(|(least, _)| sum >= least) {
            break;
        }
        let with = sum + things[at].length.unwrap_or_default();
        if window.is_some_and(|(_, most)| with > most) {
            break;
        }
        picks.push(at);
        sum = with;
    }
    picks
}

/// One draw: things taken at random while they fit, then swapped one
/// for another for as long as that brings the total nearer.
fn drawn(
    things: &[Thing],
    wish: Wish,
    order: Order,
    kept: &[usize],
    free: &[usize],
    draw: &mut Draw,
) -> Vec<usize> {
    let window = wish.window();
    // The order they are tried in: at random, the used ones first and
    // the likelier the more used when that is asked.
    let mut tried: Vec<(f32, usize)> = free
        .iter()
        .map(|&at| {
            let lot = draw.part();
            let rank = match order {
                Order::Usual if things[at].weight > 0.0 => lot.powf(1.0 / things[at].weight),
                // Never used: after every one that has been.
                Order::Usual => lot - 1.0,
                _ => lot,
            };
            (rank, at)
        })
        .collect();
    tried.sort_by(|a, b| b.0.total_cmp(&a.0));
    let mut picks = kept.to_vec();
    let mut sum = total(things, &picks);
    let mut spare = Vec::new();
    for (_, at) in tried {
        let full = wish.count.is_some_and(|count| picks.len() >= count);
        let enough = wish.count.is_none() && window.is_some_and(|(least, _)| sum >= least);
        let with = sum + things[at].length.unwrap_or_default();
        let past = window.is_some_and(|(_, most)| with > most);
        if full || enough || past {
            spare.push(at);
        } else {
            picks.push(at);
            sum = with;
        }
    }
    // Nearer by swapping one chosen (not a kept one) for one left out.
    let mut far = off(sum, window);
    while !far.is_zero() {
        let better = (kept.len()..picks.len())
            .flat_map(|chosen| (0..spare.len()).map(move |left| (chosen, left)))
            .map(|(chosen, left)| {
                let swapped = sum + things[spare[left]].length.unwrap_or_default()
                    - things[picks[chosen]].length.unwrap_or_default();
                (off(swapped, window), chosen, left, swapped)
            })
            .min_by_key(|(far, ..)| *far)
            .filter(|(nearer, ..)| *nearer < far);
        let Some((nearer, chosen, left, swapped)) = better else {
            break;
        };
        std::mem::swap(&mut picks[chosen], &mut spare[left]);
        (far, sum) = (nearer, swapped);
    }
    picks
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIN: Duration = Duration::from_secs(60);

    fn songs(seconds: &[u64]) -> Vec<Thing> {
        seconds
            .iter()
            .map(|&seconds| Thing {
                length: Some(Duration::from_secs(seconds)),
                weight: 1.0,
            })
            .collect()
    }

    fn wish(time: Option<Duration>, count: Option<usize>, fit: Fit) -> Wish {
        Wish {
            time,
            count,
            fit,
            margin: MIN,
        }
    }

    #[test]
    fn the_window_is_what_the_fit_and_the_margin_allow() {
        let at = |fit| wish(Some(15 * MIN), None, fit).window().unwrap();
        assert_eq!(at(Fit::AtMost), (14 * MIN, 15 * MIN));
        assert_eq!(at(Fit::Exact), (15 * MIN, 15 * MIN));
        assert_eq!(at(Fit::AtLeast), (15 * MIN, 16 * MIN));
        assert_eq!(at(Fit::Either), (14 * MIN, 16 * MIN));
        assert_eq!(wish(None, Some(3), Fit::AtMost).window(), None);
    }

    #[test]
    fn in_the_order_held_none_is_passed_over() {
        let library = songs(&[200, 300, 400, 100, 100]);
        // Up to 10 min: 200 + 300 fit; 400 would go past, and it ends
        // there, the two short ones after it left alone.
        let chosen = choose(
            &library,
            wish(Some(10 * MIN), None, Fit::AtMost),
            Order::Held,
            &[],
            1,
        );
        assert_eq!(chosen.picks, [0, 1]);
        assert_eq!(chosen.total, Duration::from_secs(500));
        assert_eq!(
            chosen.off,
            Duration::from_secs(40),
            "short of the 9 min least"
        );
        // A number alone: the first three.
        let three = choose(
            &library,
            wish(None, Some(3), Fit::AtMost),
            Order::Held,
            &[],
            1,
        );
        assert_eq!(three.picks, [0, 1, 2]);
        assert_eq!(three.off, Duration::ZERO);
    }

    #[test]
    fn a_draw_comes_to_the_time_and_can_be_made_again() {
        let library = songs(&[187, 173, 142, 220, 140, 142, 200, 95, 260, 131, 175, 210]);
        let asked = wish(Some(15 * MIN), None, Fit::AtMost);
        for seed in 1..40 {
            let chosen = choose(&library, asked, Order::Drawn, &[], seed);
            assert_eq!(chosen.off, Duration::ZERO, "seed {seed}: {chosen:?}");
            assert!(chosen.total <= 15 * MIN && chosen.total >= 14 * MIN);
            let mut once = chosen.picks.clone();
            once.sort_unstable();
            once.dedup();
            assert_eq!(once.len(), chosen.picks.len(), "none twice");
            assert_eq!(choose(&library, asked, Order::Drawn, &[], seed), chosen);
        }
        // Another seed, another draw.
        let draws: std::collections::HashSet<Vec<usize>> = (1..20)
            .map(|seed| choose(&library, asked, Order::Drawn, &[], seed).picks)
            .collect();
        assert!(draws.len() > 5);
    }

    #[test]
    fn a_number_and_a_time_together() {
        let library = songs(&[187, 173, 142, 220, 140, 142, 200, 95, 260, 131, 175, 210]);
        // Five things in 15 to 16 min.
        let asked = Wish {
            time: Some(15 * MIN),
            count: Some(5),
            fit: Fit::AtLeast,
            margin: MIN,
        };
        for seed in 1..40 {
            let chosen = choose(&library, asked, Order::Drawn, &[], seed);
            assert_eq!(chosen.picks.len(), 5, "seed {seed}");
            assert_eq!(chosen.off, Duration::ZERO, "seed {seed}: {chosen:?}");
        }
        // What can't be done says how far off it is: two things can't
        // come to fifteen minutes.
        let two = choose(
            &library,
            Wish {
                count: Some(2),
                ..asked
            },
            Order::Drawn,
            &[],
            3,
        );
        assert_eq!(two.picks.len(), 2);
        assert!(two.off > MIN);
    }

    #[test]
    fn what_is_kept_stays_and_the_rest_is_drawn_round_it() {
        let library = songs(&[187, 173, 142, 220, 140, 142, 200, 95, 260, 131]);
        let asked = wish(Some(12 * MIN), None, Fit::AtMost);
        for seed in 1..20 {
            let chosen = choose(&library, asked, Order::Drawn, &[3, 8], seed);
            assert_eq!(&chosen.picks[..2], &[3, 8]);
            assert_eq!(chosen.off, Duration::ZERO, "seed {seed}");
        }
    }

    #[test]
    fn the_usual_ones_come_before_what_was_never_used() {
        let mut library = songs(&[180; 12]);
        // The first four have never been used.
        for thing in &mut library[..4] {
            thing.weight = 0.0;
        }
        library[11].weight = 20.0;
        let asked = wish(Some(12 * MIN), None, Fit::AtMost);
        let mut favourite = 0;
        for seed in 1..60 {
            let chosen = choose(&library, asked, Order::Usual, &[], seed);
            assert_eq!(chosen.picks.len(), 4);
            assert!(
                chosen.picks.iter().all(|&at| at >= 4),
                "seed {seed}: none unused"
            );
            favourite += usize::from(chosen.picks.contains(&11));
        }
        assert!(
            favourite > 50,
            "the most used is nearly always in: {favourite}"
        );
        // With too few used ones, the unused make up the number.
        let many = choose(
            &library,
            wish(None, Some(10), Fit::AtMost),
            Order::Usual,
            &[],
            5,
        );
        assert_eq!(many.picks.len(), 10);
    }

    #[test]
    fn still_things_share_what_time_is_left() {
        let pictures = vec![
            Thing {
                length: None,
                weight: 1.0
            };
            8
        ];
        let chosen = choose(
            &pictures,
            wish(Some(20 * MIN), Some(5), Fit::AtMost),
            Order::Held,
            &[],
            1,
        );
        assert_eq!(chosen.picks.len(), 5);
        assert_eq!(chosen.each_still, Some(4 * MIN));
        assert_eq!(chosen.off, Duration::ZERO);
    }
}
