/// Market structure events — indicator-level follow-through backtest (ADR-017, Story #40).
///
/// Claim: a BOS (Break of Structure) signals trend continuation, so within `lookahead_bars`
/// price should make a higher high (bullish BOS) or lower low (bearish BOS) beyond the
/// break level by at least `follow_threshold`. A CHoCH (Change of Character) signals a
/// potential reversal with the same directional follow-through expectation.
///
/// Causal-safety: `compute_structure` emits break events at the break candle's ts; the
/// algorithm does not use future candles to detect past events. We compute once over the
/// full history, then iterate events by their index, validating each against its future
/// window. Events without enough history (idx < window_size) or future (idx + lookahead
/// >= len) are skipped.
///
/// Gate: `follow_rate > 0.55` per (event_type × direction) bucket. 0.55 rather than 0.50
/// to exclude near-random outcomes. Reported as `*_better_than_random` booleans.

use std::collections::HashMap;

use chrono::Utc;

use crate::domain::{
    candle::OhlcvCandle,
    smc::structure::compute_structure,
    types::{Direction, StructureType},
};

const DEFAULT_GATE: f64 = 0.55;

// ── Output types ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, serde::Serialize)]
pub struct StructureBucketMetrics {
    pub event_type:               String,   // "bos" | "choch"
    pub direction:                String,   // "bullish" | "bearish"
    pub n_events:                 usize,
    pub n_followed:               usize,
    pub follow_rate:              f64,
    pub avg_bars_to_follow:       Option<f64>,
    pub avg_follow_magnitude_pct: Option<f64>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct StructureBacktestResponse {
    pub pair:             String,
    pub tf:               String,
    pub total_events:     usize,
    pub buckets:          Vec<StructureBucketMetrics>,
    pub bos_bullish_better_than_random:   bool,
    pub bos_bearish_better_than_random:   bool,
    pub choch_bullish_better_than_random: bool,
    pub choch_bearish_better_than_random: bool,
    pub candles_analyzed: usize,
    pub window_size:      usize,
    pub lookahead_bars:   usize,
    pub follow_threshold: f64,
    pub computed_at:      String,
}

// ── Internal observation ──────────────────────────────────────────────────────

struct EventObservation {
    event_type:             StructureType,
    direction:              Direction,
    followed:               bool,
    bars_to_follow:         Option<usize>,
    follow_magnitude_pct:   Option<f64>,
}

// ── Public entry point ────────────────────────────────────────────────────────

pub fn backtest_structure(
    candles:          &[OhlcvCandle],
    window_size:      usize,
    lookahead_bars:   usize,
    follow_threshold: f64,
    tf_str:           &str,
    pair_str:         &str,
) -> StructureBacktestResponse {
    let empty = || StructureBacktestResponse {
        pair:             pair_str.to_string(),
        tf:               tf_str.to_string(),
        total_events:     0,
        buckets:          vec![],
        bos_bullish_better_than_random:   false,
        bos_bearish_better_than_random:   false,
        choch_bullish_better_than_random: false,
        choch_bearish_better_than_random: false,
        candles_analyzed: 0,
        window_size,
        lookahead_bars,
        follow_threshold,
        computed_at:      Utc::now().to_rfc3339(),
    };

    if candles.len() < window_size + lookahead_bars + 1 {
        return empty();
    }

    let events  = compute_structure(candles);
    let ts_idx: HashMap<String, usize> = candles.iter()
        .enumerate()
        .map(|(i, c)| (c.ts.clone(), i))
        .collect();

    let mut observations: Vec<EventObservation> = Vec::new();

    for event in &events {
        let Some(&idx) = ts_idx.get(&event.ts) else { continue };
        if idx < window_size { continue; }
        if idx + lookahead_bars >= candles.len() { continue; }

        let future = &candles[idx + 1..=idx + lookahead_bars];
        observations.push(validate_event(event, future, follow_threshold));
    }

    let buckets = aggregate(&observations);
    let (bos_bu, bos_be, ch_bu, ch_be) = gates(&buckets);
    let analyzed = candles.len()
        .saturating_sub(window_size)
        .saturating_sub(lookahead_bars);

    StructureBacktestResponse {
        pair:             pair_str.to_string(),
        tf:               tf_str.to_string(),
        total_events:     observations.len(),
        buckets,
        bos_bullish_better_than_random:   bos_bu,
        bos_bearish_better_than_random:   bos_be,
        choch_bullish_better_than_random: ch_bu,
        choch_bearish_better_than_random: ch_be,
        candles_analyzed: analyzed,
        window_size,
        lookahead_bars,
        follow_threshold,
        computed_at:      Utc::now().to_rfc3339(),
    }
}

// ── Validation ────────────────────────────────────────────────────────────────

fn validate_event(
    event:            &crate::domain::smc::structure::StructureEvent,
    future:           &[OhlcvCandle],
    follow_threshold: f64,
) -> EventObservation {
    let mut bars_to_follow       = None;
    let mut follow_magnitude_pct = None;

    if future.is_empty() || event.level <= 0.0 {
        return EventObservation {
            event_type:           event.event_type,
            direction:            event.direction,
            followed:             false,
            bars_to_follow,
            follow_magnitude_pct,
        };
    }

    let end_close = future.last().unwrap().close;

    let followed = match event.direction {
        Direction::Bullish => {
            let target = event.level * (1.0 + follow_threshold);

            for (i, c) in future.iter().enumerate() {
                if c.high >= target {
                    bars_to_follow = Some(i + 1);
                    break;
                }
            }
            let max_high = future.iter().map(|c| c.high).fold(f64::NEG_INFINITY, f64::max);
            if max_high > event.level {
                follow_magnitude_pct = Some((max_high - event.level) / event.level * 100.0);
            }

            end_close >= target
        }
        Direction::Bearish => {
            let target = event.level * (1.0 - follow_threshold);

            for (i, c) in future.iter().enumerate() {
                if c.low <= target {
                    bars_to_follow = Some(i + 1);
                    break;
                }
            }
            let min_low = future.iter().map(|c| c.low).fold(f64::INFINITY, f64::min);
            if min_low < event.level {
                follow_magnitude_pct = Some((event.level - min_low) / event.level * 100.0);
            }

            end_close <= target
        }
    };

    EventObservation {
        event_type:           event.event_type,
        direction:            event.direction,
        followed,
        bars_to_follow,
        follow_magnitude_pct,
    }
}

// ── Aggregation ───────────────────────────────────────────────────────────────

fn aggregate(obs: &[EventObservation]) -> Vec<StructureBucketMetrics> {
    let mut buckets = Vec::new();
    for et in [StructureType::Bos, StructureType::Choch] {
        for dir in [Direction::Bullish, Direction::Bearish] {
            let filtered: Vec<_> = obs.iter()
                .filter(|o| o.event_type == et && o.direction == dir)
                .collect();
            if filtered.is_empty() { continue; }

            let n_events    = filtered.len();
            let n_followed  = filtered.iter().filter(|o| o.followed).count();
            let follow_rate = n_followed as f64 / n_events as f64;

            let bars: Vec<f64> = filtered.iter().filter_map(|o| o.bars_to_follow.map(|b| b as f64)).collect();
            let avg_bars_to_follow = if bars.is_empty() { None } else { Some(round2(bars.iter().sum::<f64>() / bars.len() as f64)) };

            let mags: Vec<f64> = filtered.iter().filter_map(|o| o.follow_magnitude_pct).collect();
            let avg_follow_magnitude_pct = if mags.is_empty() { None } else { Some(round4(mags.iter().sum::<f64>() / mags.len() as f64)) };

            buckets.push(StructureBucketMetrics {
                event_type:  event_type_str(et).to_string(),
                direction:   direction_str(dir).to_string(),
                n_events,
                n_followed,
                follow_rate: round4(follow_rate),
                avg_bars_to_follow,
                avg_follow_magnitude_pct,
            });
        }
    }
    buckets
}

fn gates(buckets: &[StructureBucketMetrics]) -> (bool, bool, bool, bool) {
    let gate = |et: &str, dir: &str| -> bool {
        buckets.iter()
            .find(|b| b.event_type == et && b.direction == dir)
            .map(|b| b.n_events >= 30 && b.follow_rate >= DEFAULT_GATE)
            .unwrap_or(false)
    };
    (
        gate("bos",   "bullish"),
        gate("bos",   "bearish"),
        gate("choch", "bullish"),
        gate("choch", "bearish"),
    )
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn event_type_str(et: StructureType) -> &'static str {
    match et {
        StructureType::Bos   => "bos",
        StructureType::Choch => "choch",
    }
}

fn direction_str(d: Direction) -> &'static str {
    match d {
        Direction::Bullish => "bullish",
        Direction::Bearish => "bearish",
    }
}

fn round2(x: f64) -> f64 { (x * 100.0).round() / 100.0 }
fn round4(x: f64) -> f64 { (x * 10_000.0).round() / 10_000.0 }

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::smc::structure::StructureEvent;

    fn c(ts: &str, open: f64, high: f64, low: f64, close: f64) -> OhlcvCandle {
        OhlcvCandle {
            ts: ts.to_string(), open, high, low, close, volume: 100.0,
            mb_vol: None, ms_vol: None, lb_vol: None, ls_vol: None,
            mb_count: None, ms_count: None, lb_count: None, ls_count: None,
        }
    }

    #[test]
    fn too_few_candles_returns_empty() {
        let candles: Vec<_> = (0..50).map(|i| c(&i.to_string(), 100.0, 101.0, 99.0, 100.0)).collect();
        let r = backtest_structure(&candles, 100, 10, 0.005, "1d", "BTCEUR");
        assert_eq!(r.total_events, 0);
        assert!(!r.bos_bullish_better_than_random);
    }

    #[test]
    fn bullish_event_with_higher_high_is_followed() {
        let event = StructureEvent {
            ts: "10".to_string(),
            event_type: StructureType::Bos,
            level: 100.0,
            direction: Direction::Bullish,
        };
        let future = vec![
            c("11", 100.0, 100.3, 99.8, 100.2),   // high 100.3 < target 100.5 — not yet
            c("12", 100.2, 102.0, 100.0, 101.8),  // high 102.0 ≥ target 100.5 ✓
            c("13", 101.8, 102.5, 101.5, 102.3),
        ];
        let obs = validate_event(&event, &future, 0.005);
        assert!(obs.followed);
        assert_eq!(obs.bars_to_follow, Some(2));
        assert!(obs.follow_magnitude_pct.unwrap() > 2.0);
    }

    #[test]
    fn bullish_false_break_that_reverses_is_not_followed() {
        let event = StructureEvent {
            ts: "10".to_string(),
            event_type: StructureType::Bos,
            level: 100.0,
            direction: Direction::Bullish,
        };
        let future = vec![
            c("11", 100.0, 102.0, 99.8, 101.5),    // spikes up past 100.5
            c("12", 101.5, 101.7, 99.0, 99.5),     // reverses
            c("13", 99.5, 99.8, 97.5, 98.0),       // below level — break failed
        ];
        let obs = validate_event(&event, &future, 0.005);
        assert_eq!(obs.bars_to_follow, Some(1));
        assert!(!obs.followed);
    }

    #[test]
    fn bullish_event_without_higher_high_is_not_followed() {
        let event = StructureEvent {
            ts: "10".to_string(),
            event_type: StructureType::Bos,
            level: 100.0,
            direction: Direction::Bullish,
        };
        let future = vec![
            c("11", 100.0, 100.3, 99.5, 99.8),
            c("12", 99.8, 100.4, 99.2, 99.5),
            c("13", 99.5, 100.2, 98.5, 99.0),
        ];
        let obs = validate_event(&event, &future, 0.005);
        assert!(!obs.followed);
        assert_eq!(obs.bars_to_follow, None);
    }

    #[test]
    fn bearish_event_with_lower_low_is_followed() {
        let event = StructureEvent {
            ts: "10".to_string(),
            event_type: StructureType::Choch,
            level: 100.0,
            direction: Direction::Bearish,
        };
        let future = vec![
            c("11", 100.0, 100.2, 99.7, 99.8),   // low 99.7 > target 99.5 — not yet
            c("12", 99.7, 99.9, 99.0, 99.2),     // low 99.0 ≤ target 99.5 ✓
        ];
        let obs = validate_event(&event, &future, 0.005);
        assert!(obs.followed);
        assert_eq!(obs.bars_to_follow, Some(2));
    }

    #[test]
    fn aggregate_buckets_combine_event_type_and_direction() {
        let obs = vec![
            EventObservation { event_type: StructureType::Bos, direction: Direction::Bullish, followed: true,  bars_to_follow: Some(2), follow_magnitude_pct: Some(1.5) },
            EventObservation { event_type: StructureType::Bos, direction: Direction::Bullish, followed: false, bars_to_follow: None,    follow_magnitude_pct: None },
            EventObservation { event_type: StructureType::Bos, direction: Direction::Bearish, followed: true,  bars_to_follow: Some(3), follow_magnitude_pct: Some(0.8) },
        ];
        let buckets = aggregate(&obs);
        let bullish = buckets.iter().find(|b| b.event_type == "bos" && b.direction == "bullish").unwrap();
        assert_eq!(bullish.n_events, 2);
        assert_eq!(bullish.n_followed, 1);
        assert!((bullish.follow_rate - 0.5).abs() < 1e-9);
    }

    #[test]
    fn gate_requires_n_at_least_30_and_rate_at_least_055() {
        let buckets = vec![
            StructureBucketMetrics {
                event_type: "bos".into(), direction: "bullish".into(),
                n_events: 50, n_followed: 30, follow_rate: 0.6,
                avg_bars_to_follow: None, avg_follow_magnitude_pct: None,
            },
            StructureBucketMetrics {
                event_type: "bos".into(), direction: "bearish".into(),
                n_events: 10, n_followed: 8, follow_rate: 0.8,  // high rate but n<30
                avg_bars_to_follow: None, avg_follow_magnitude_pct: None,
            },
            StructureBucketMetrics {
                event_type: "choch".into(), direction: "bullish".into(),
                n_events: 40, n_followed: 20, follow_rate: 0.5,  // n ok but rate<0.55
                avg_bars_to_follow: None, avg_follow_magnitude_pct: None,
            },
        ];
        let (bos_bu, bos_be, ch_bu, _ch_be) = gates(&buckets);
        assert!(bos_bu);
        assert!(!bos_be);
        assert!(!ch_bu);
    }

    #[test]
    fn walk_forward_skips_events_near_boundaries() {
        let mut candles: Vec<OhlcvCandle> = (0..200).map(|i| {
            let p = 100.0 + (i as f64 * 0.1);
            c(&i.to_string(), p, p + 0.5, p - 0.5, p)
        }).collect();
        for i in 50..55 {
            candles[i] = c(&i.to_string(), 105.0, 115.0, 100.0, 114.0);
        }
        let r = backtest_structure(&candles, 100, 10, 0.005, "1d", "TEST");
        assert!(r.candles_analyzed > 0);
    }
}
