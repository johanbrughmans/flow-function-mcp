/// Fibonacci Take-Profit Targets — ADR-003.
///
/// Given an entry price, returns resistance clusters above current price as
/// actionable take-profit levels, plus the nearest support below current price.
/// Reuses `compute_fib_confluence` — no duplication of DiNapoli logic.

use crate::domain::{
    candle::OhlcvCandle,
    smc::{
        fib_confluence::compute_fib_confluence,
        fib_profile::FibProfile,
    },
};

// ── Output types ──────────────────────────────────────────────────────────────

#[derive(Debug, serde::Serialize)]
pub struct FibTarget {
    pub price:                     f64,
    pub strength:                  usize,
    pub distance_from_current_pct: f64,
    pub distance_from_entry_pct:   f64,
}

#[derive(Debug, serde::Serialize)]
pub struct NearestZone {
    pub price:        f64,
    pub strength:     usize,
    pub distance_pct: f64,
}

#[derive(Debug, serde::Serialize)]
pub struct FibTargetResult {
    pub current_price:   f64,
    pub entry_price:     f64,
    pub pnl_pct:         f64,
    pub targets:         Vec<FibTarget>,
    pub nearest_support: Option<NearestZone>,
    pub profile:         String,
    pub exploratory:     bool,
}

// ── Public entry point ────────────────────────────────────────────────────────

pub fn compute_fib_targets(
    raw:         &[OhlcvCandle],
    entry_price: f64,
    profile:     &FibProfile,
) -> Result<FibTargetResult, String> {
    if entry_price <= 0.0 {
        return Err("entry_price must be > 0".to_string());
    }
    if raw.is_empty() {
        return Err("no candle data".to_string());
    }

    let current_price = raw.last().unwrap().close;
    let clusters      = compute_fib_confluence(raw, profile);

    let mut targets: Vec<FibTarget> = clusters.iter()
        .filter(|c| c.direction == "resistance")
        .map(|c| FibTarget {
            price:                     round5(c.price),
            strength:                  c.strength,
            distance_from_current_pct: round2((c.price - current_price) / current_price * 100.0),
            distance_from_entry_pct:   round2((c.price - entry_price)   / entry_price   * 100.0),
        })
        .collect();
    targets.sort_by(|a, b| a.price.partial_cmp(&b.price).unwrap_or(std::cmp::Ordering::Equal));

    let nearest_support = clusters.iter()
        .filter(|c| c.direction == "support" && c.distance_pct <= 20.0)
        .max_by_key(|c| c.strength)
        .map(|c| NearestZone {
            price:        c.price,
            strength:     c.strength,
            distance_pct: c.distance_pct,
        });

    let pnl_pct = round2((current_price - entry_price) / entry_price * 100.0);

    Ok(FibTargetResult {
        current_price: round5(current_price),
        entry_price:   round5(entry_price),
        pnl_pct,
        targets,
        nearest_support,
        profile:       profile.name.clone(),
        exploratory:   profile.exploratory,
    })
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
    fn rejects_zero_entry_price() {
        let raw = vec![c("0", 1.0, 1.1, 0.9, 1.0)];
        assert!(compute_fib_targets(&raw, 0.0, &FibProfile::mature()).is_err());
    }

    #[test]
    fn rejects_negative_entry_price() {
        let raw = vec![c("0", 1.0, 1.1, 0.9, 1.0)];
        assert!(compute_fib_targets(&raw, -1.0, &FibProfile::mature()).is_err());
    }

    #[test]
    fn rejects_empty_candles() {
        assert!(compute_fib_targets(&[], 1.0, &FibProfile::mature()).is_err());
    }

    #[test]
    fn pnl_negative_when_current_below_entry() {
        let raw: Vec<_> = (0..6)
            .map(|i| c(&i.to_string(), 1.0, 1.0 + i as f64 * 0.1, 0.9, 1.0 + i as f64 * 0.1))
            .collect();
        let result = compute_fib_targets(&raw, 2.0, &FibProfile::mature()).unwrap();
        assert!(result.pnl_pct < 0.0);
    }

    #[test]
    fn pnl_positive_when_current_above_entry() {
        let raw: Vec<_> = (0..6)
            .map(|i| c(&i.to_string(), 1.0, 1.0 + i as f64 * 0.1, 0.9, 1.0 + i as f64 * 0.1))
            .collect();
        let result = compute_fib_targets(&raw, 0.5, &FibProfile::mature()).unwrap();
        assert!(result.pnl_pct > 0.0);
    }

    #[test]
    fn targets_sorted_ascending_by_price() {
        let raw: Vec<_> = (0..6).map(|i| c(&i.to_string(), 1.0, 1.0, 1.0, 1.0)).collect();
        let result = compute_fib_targets(&raw, 0.5, &FibProfile::mature()).unwrap();
        for w in result.targets.windows(2) {
            assert!(w[0].price <= w[1].price);
        }
    }

    #[test]
    fn exploratory_flag_mirrors_profile() {
        let raw = vec![c("0", 1.0, 1.1, 0.9, 1.0)];
        assert!(compute_fib_targets(&raw, 0.5, &FibProfile::nascent()).unwrap().exploratory);
        assert!(!compute_fib_targets(&raw, 0.5, &FibProfile::mature()).unwrap().exploratory);
    }

    #[test]
    fn profile_name_in_output() {
        let raw = vec![c("0", 1.0, 1.1, 0.9, 1.0)];
        let r = compute_fib_targets(&raw, 0.5, &FibProfile::developing()).unwrap();
        assert_eq!(r.profile, "developing");
    }
}
