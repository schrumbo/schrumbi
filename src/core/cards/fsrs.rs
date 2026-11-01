//! FSRS-6, the Free Spaced Repetition Scheduler, with its published default parameters.
//! Source: open-spaced-repetition/fsrs-rs; evaluated on hundreds of millions of real reviews (srs-benchmark).

/// The published default parameters of FSRS-6.
pub const W: [f64; 21] = [0.212, 1.2931, 2.3065, 8.2956, 6.4133, 0.8334, 3.0194, 0.001, 1.8722, 0.1666, 0.796, 1.4835, 0.0614, 0.2629, 1.6483, 0.6014, 1.8729, 0.5425, 0.0912, 0.0658, 0.1542];
const S_MIN: f64 = 0.001;
const S_MAX: f64 = 36500.;
const D_MIN: f64 = 1.;
const D_MAX: f64 = 10.;

/// How well a card was remembered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Rating {
    Again = 1,
    Hard = 2,
    Good = 3,
    Easy = 4,
}

impl Rating {
    pub const ALL: [Rating; 4] = [Rating::Again, Rating::Hard, Rating::Good, Rating::Easy];
}

/// What is known about the memory of one card.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Memory {
    /// Days until the chance of remembering falls to 90 %.
    pub stability: f64,
    /// How hard the card is, from 1 to 10.
    pub difficulty: f64,
}

fn factor() -> f64 {
    0.9f64.powf(-1. / W[20]) - 1.
}

/// The chance of remembering after `days` without review.
pub fn retrievability(stability: f64, days: f64) -> f64 {
    (days.max(0.) / stability.max(S_MIN) * factor() + 1.).powf(-W[20])
}

/// How many days until the chance of remembering falls to `retention`.
pub fn interval(stability: f64, retention: f64) -> f64 {
    stability * (retention.powf(-1. / W[20]) - 1.) / factor()
}

fn init_difficulty(rating: f64) -> f64 {
    W[4] - (W[5] * (rating - 1.)).exp() + 1.
}

/// The memory after the first review.
pub fn first(rating: Rating) -> Memory {
    Memory { stability: W[rating as usize - 1].clamp(S_MIN, S_MAX), difficulty: init_difficulty(rating as usize as f64).clamp(D_MIN, D_MAX) }
}

/// The memory after a later review, `days` whole days after the one before; 0 means the same day.
pub fn next(memory: Memory, days: f64, rating: Rating) -> Memory {
    let (s, d) = (memory.stability.clamp(S_MIN, S_MAX), memory.difficulty.clamp(D_MIN, D_MAX));
    let g = rating as usize as f64;
    let delta = -W[6] * (g - 3.);
    let moved = d + (10. - d) / 9. * delta;
    let difficulty = (W[7] * init_difficulty(4.) + (1. - W[7]) * moved).clamp(D_MIN, D_MAX);
    let stability = if days <= 0. {
        let rise = (W[17] * (g - 3. + W[18])).exp() * s.powf(-W[19]);
        s * if rating != Rating::Again { rise.max(1.) } else { rise }
    } else {
        let r = retrievability(s, days);
        if rating == Rating::Again {
            let floor = s / (W[17] * W[18]).exp();
            (W[11] * d.powf(-W[12]) * ((s + 1.).powf(W[13]) - 1.) * ((1. - r) * W[14]).exp()).min(floor)
        } else {
            let hard = if rating == Rating::Hard { W[15] } else { 1. };
            let easy = if rating == Rating::Easy { W[16] } else { 1. };
            s * (W[8].exp() * (11. - d) * s.powf(-W[9]) * (((1. - r) * W[10]).exp() - 1.) * hard * easy + 1.)
        }
    };
    Memory { stability: stability.clamp(S_MIN, S_MAX), difficulty }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stability_is_where_ninety_percent_remain() {
        for s in [0.5, 2.3, 40., 900.] {
            assert!((retrievability(s, s) - 0.9).abs() < 1e-9);
            assert!((interval(s, 0.9) - s).abs() < 1e-6);
        }
        assert_eq!(retrievability(10., 0.), 1.);
    }

    #[test]
    fn first_reviews_use_the_published_parameters() {
        assert_eq!(first(Rating::Again).stability, 0.212);
        assert_eq!(first(Rating::Good).stability, 2.3065);
        assert_eq!(first(Rating::Easy).stability, 8.2956);
        assert!((first(Rating::Good).difficulty - (6.4133 - (2. * 0.8334f64).exp() + 1.)).abs() < 1e-9);
    }

    #[test]
    fn better_answers_mean_longer_memories() {
        let memory = Memory { stability: 10., difficulty: 5. };
        let after: Vec<Memory> = Rating::ALL.iter().map(|r| next(memory, 12., *r)).collect();
        assert!(after[0].stability < memory.stability);
        assert!(after[1].stability > memory.stability);
        assert!(after[1].stability < after[2].stability && after[2].stability < after[3].stability);
        assert!(after[0].difficulty > after[2].difficulty && after[2].difficulty > after[3].difficulty);
    }

    #[test]
    fn a_later_review_of_a_good_answer_teaches_more() {
        let memory = Memory { stability: 10., difficulty: 5. };
        assert!(next(memory, 30., Rating::Good).stability > next(memory, 3., Rating::Good).stability);
    }

    #[test]
    fn same_day_reviews_barely_move_a_strong_memory() {
        let memory = Memory { stability: 100., difficulty: 5. };
        let good = next(memory, 0., Rating::Good);
        assert!(good.stability >= 100. && good.stability < 110.);
        assert!(next(memory, 0., Rating::Again).stability < 100.);
    }

    #[test]
    fn stays_in_bounds_over_many_reviews() {
        let mut memory = first(Rating::Easy);
        for _ in 0..200 {
            memory = next(memory, memory.stability, Rating::Easy);
            assert!(memory.stability.is_finite() && memory.stability <= S_MAX);
            assert!((D_MIN..=D_MAX).contains(&memory.difficulty));
        }
        for _ in 0..200 {
            memory = next(memory, 1., Rating::Again);
            assert!(memory.stability >= S_MIN);
        }
    }
}
