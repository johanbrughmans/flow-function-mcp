/// Fibonacci Confluence — DiNapoli/Boroden methodology.
///
/// Parameters are controlled by `FibProfile` — see `fib_profile.rs` for canonical sets.
///
/// Retracements: 38.2%, 50.0%, 61.8% (DiNapoli primary entry levels).
/// Expansions from ABC pattern:
///   COP = C ± AB × 0.618   (Contracted Objective Point)
///   OP  = C ± AB × 1.000   (Objective Point)
///   XOP = C ± AB × 1.618   (Expanded Objective Point)
/// Cluster tolerance and minimum cluster size come from the profile.
/// ATR compression parameters also come from the profile.

use crate::domain::{
    candle::OhlcvCandle,
    indicators::atr::compute_atr,
    smc::fib_profile::FibProfile,
};

const ATR_PERIOD: usize = 14;

const RETRACE_LEVELS: &[(f64, &str)] = &[
    (0.382, "38.2%"),
    (0.500, "50.0%"),
    (0.618, "61.8%"),
];
const EXPAND_LEVELS: &[(f64, &str)] = &[
    (0.618, "COP"),
    (1.000, "OP"),
    (1.618, "XOP"),
];

// ── Output types ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, serde::Serialize)]
pub struct FibLevel {
    pub label:     String,
    pub price:     f64,
    pub anchor_ts: String,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct FibCluster {
    pub price:          f64,
    pub strength:       usize,
    pub direction:      String,
    pub levels:         Vec<FibLevel>,
    pub atr_compressed: bool,
    pub distance_pct:   f64,
}

// ── Internal pivot (pub(crate) for future harmonics.rs) ───────────────────────

#[derive(Clone, Copy, PartialEq)]
pub(crate) enum PivotKind { High, Low }

pub(crate) struct Pivot {
    pub ts:    String,
    pub price: f64,
    pub kind:  PivotKind,
}

// ── Public entry point ────────────────────────────────────────────────────────

pub fn compute_fib_confluence(raw: &[OhlcvCandle], profile: &FibProfile) -> Vec<FibCluster> {
    if raw.len() < 5 { return vec![]; }

    let current_close  = raw.last().unwrap().close;
    let atr_compressed = is_atr_compressed(raw, profile);
    let pivots         = detect_pivots(raw);
    if pivots.len() < 2 { return vec![]; }

    let mut all_levels: Vec<FibLevel> = Vec::new();

    for w in pivots.windows(2) {
        let a  = &w[0];
        let b  = &w[1];
        let ab = b.price - a.price;
        for &(r, label) in RETRACE_LEVELS {
            let price = b.price - ab * r;
            all_levels.push(FibLevel {
                label:     format!("{} retrace", label),
                price,
                anchor_ts: a.ts.clone(),
            });
        }
    }

    for w in pivots.windows(3) {
        let a = &w[0];
        let b = &w[1];
        let c = &w[2];
        if b.kind == a.kind { continue; }
        let ab  = (b.price - a.price).abs();
        let dir = if b.price > a.price { 1.0_f64 } else { -1.0_f64 };
        for &(ratio, name) in EXPAND_LEVELS {
            let price = c.price + dir * ab * ratio;
            all_levels.push(FibLevel {
                label:     name.to_string(),
                price,
                anchor_ts: a.ts.clone(),
            });
        }
    }

    cluster_levels(all_levels, current_close, atr_compressed, profile)
}

// ── Pivot detection (pub(crate) — shared with harmonics once implemented) ─────

pub(crate) fn detect_pivots(raw: &[OhlcvCandle]) -> Vec<Pivot> {
    let mut raw_pivots: Vec<Pivot> = Vec::new();
    for i in 1..raw.len().saturating_sub(1) {
        let prev = &raw[i - 1];
        let curr = &raw[i];
        let next = &raw[i + 1];
        if curr.high > prev.high && curr.high >= next.high {
            raw_pivots.push(Pivot { ts: curr.ts.clone(), price: curr.high, kind: PivotKind::High });
        } else if curr.low < prev.low && curr.low <= next.low {
            raw_pivots.push(Pivot { ts: curr.ts.clone(), price: curr.low, kind: PivotKind::Low });
        }
    }
    deduplicate_pivots(raw_pivots)
}

pub(crate) fn deduplicate_pivots(pivots: Vec<Pivot>) -> Vec<Pivot> {
    let mut result: Vec<Pivot> = Vec::new();
    for p in pivots {
        let merged = if let Some(last) = result.last_mut() {
            if last.kind == p.kind {
                if p.kind == PivotKind::High && p.price > last.price {
                    last.price = p.price;
                    last.ts    = p.ts.clone();
                } else if p.kind == PivotKind::Low && p.price < last.price {
                    last.price = p.price;
                    last.ts    = p.ts.clone();
                }
                true
            } else {
                false
            }
        } else {
            false
        };
        if !merged { result.push(p); }
    }
    result
}

// ── Clustering ────────────────────────────────────────────────────────────────

fn cluster_levels(
    mut levels:     Vec<FibLevel>,
    current_close:  f64,
    atr_compressed: bool,
    profile:        &FibProfile,
) -> Vec<FibCluster> {
    levels.sort_by(|a, b| a.price.partial_cmp(&b.price).unwrap_or(std::cmp::Ordering::Equal));

    let mut clusters: Vec<FibCluster> = Vec::new();
    let mut i = 0;
    while i < levels.len() {
        let base = levels[i].price;
        let tol  = base * profile.cluster_tolerance;
        let end  = levels[i..].partition_point(|l| (l.price - base).abs() <= tol);
        let n    = end.max(1);
        let group: Vec<FibLevel> = levels[i..i + n].to_vec();
        let center = group.iter().map(|l| l.price).sum::<f64>() / group.len() as f64;
        let dist   = ((center - current_close) / current_close).abs() * 100.0;
        let dir    = if center <= current_close { "support" } else { "resistance" };
        if group.len() >= profile.min_cluster_size {
            clusters.push(FibCluster {
                price:          round5(center),
                strength:       group.len(),
                direction:      dir.to_string(),
                levels:         group,
                atr_compressed,
                distance_pct:   round2(dist),
            });
        }
        i += n;
    }

    clusters.sort_by(|a, b| a.distance_pct.partial_cmp(&b.distance_pct).unwrap_or(std::cmp::Ordering::Equal));
    clusters
}

// ── ATR compression ───────────────────────────────────────────────────────────

fn is_atr_compressed(raw: &[OhlcvCandle], profile: &FibProfile) -> bool {
    let atr_pts = compute_atr(raw, ATR_PERIOD);
    if atr_pts.len() < profile.atr_compression_lookback { return false; }
    let recent   = &atr_pts[atr_pts.len() - profile.atr_compression_lookback..];
    let mean_atr = recent.iter().map(|p| p.atr).sum::<f64>() / recent.len() as f64;
    let current  = recent.last().map(|p| p.atr).unwrap_or(0.0);
    current < mean_atr * profile.atr_compression_ratio
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn round2(x: f64) -> f64 { (x * 100.0).round() / 100.0 }
fn round5(x: f64) -> f64 { (x * 100_000.0).round() / 100_000.0 }

#[cfg(test)]
mod tests {
    use super::*;

    fn c(ts: &str, open: f64, high: f64, low: f64, close: f64) -> OhlcvCandle {
        OhlcvCandle {
            ts: ts.to_string(), open, high, low, close, volume: 100.0,
            mb_vol: None, ms_vol: None, lb_vol: None, ls_vol: None,
            mb_count: None, ms_count: None, lb_count: None, ls_count: None,
        }
    }

    #[test]
    fn returns_empty_when_too_few_candles() {
        let raw = vec![c("0", 1.0, 1.1, 0.9, 1.0)];
        assert!(compute_fib_confluence(&raw, &FibProfile::mature()).is_empty());
    }

    #[test]
    fn does_not_panic_on_flat_market() {
        let raw: Vec<_> = (0..30).map(|i| c(&i.to_string(), 1.0, 1.0, 1.0, 1.0)).collect();
        let _ = compute_fib_confluence(&raw, &FibProfile::mature());
    }

    #[test]
    fn nascent_wider_tolerance_than_mature() {
        assert!(FibProfile::nascent().cluster_tolerance > FibProfile::mature().cluster_tolerance);
    }

    #[test]
    fn deduplication_keeps_highest_high() {
        let pivots = vec![
            Pivot { ts: "a".into(), price: 1.0, kind: PivotKind::High },
            Pivot { ts: "b".into(), price: 1.5, kind: PivotKind::High },
            Pivot { ts: "c".into(), price: 0.8, kind: PivotKind::Low },
        ];
        let deduped = deduplicate_pivots(pivots);
        assert_eq!(deduped.len(), 2);
        assert!((deduped[0].price - 1.5).abs() < 1e-9);
        assert_eq!(deduped[0].kind, PivotKind::High);
    }

    #[test]
    fn deduplication_keeps_lowest_low() {
        let pivots = vec![
            Pivot { ts: "a".into(), price: 1.0, kind: PivotKind::Low },
            Pivot { ts: "b".into(), price: 0.5, kind: PivotKind::Low },
            Pivot { ts: "c".into(), price: 2.0, kind: PivotKind::High },
        ];
        let deduped = deduplicate_pivots(pivots);
        assert_eq!(deduped.len(), 2);
        assert!((deduped[0].price - 0.5).abs() < 1e-9);
    }
}
