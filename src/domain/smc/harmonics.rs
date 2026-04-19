/// Harmonic Pattern detection — XABCD geometric analysis (ADR-001, Epic #9).
///
/// Detects completed XABCD patterns: Gartley, Bat, Butterfly, Crab.
/// Ratios per Carney/Pesavento methodology.
/// Enabled patterns and ratio tolerances are controlled by FibProfile.
///
/// Bullish: X(Low) A(High) B(Low) C(High) D(Low) — buy at D.
/// Bearish: X(High) A(Low) B(High) C(Low) D(High) — sell at D.
///
/// D completion ratios (key signal, tolerance = profile.harmonic_ratio_tolerance):
///   Gartley   xd/xa = 0.786
///   Bat       xd/xa = 0.886
///   Butterfly xd/xa = 1.272  (D extends beyond X)
///   Crab      xd/xa = 1.618  (D extends beyond X, most extreme)

use crate::domain::{
    candle::OhlcvCandle,
    smc::{
        fib_profile::FibProfile,
        pivots::{detect_pivots, PivotKind},
    },
};

// ── Output ────────────────────────────────────────────────────────────────────

#[derive(Debug, serde::Serialize)]
pub struct HarmonicPattern {
    pub ts_x:          String,
    pub ts_a:          String,
    pub ts_b:          String,
    pub ts_c:          String,
    pub ts_d:          String,
    pub pattern:       String,
    pub direction:     String,
    pub d_price:       f64,
    pub xabcd_quality: f64,
    pub exploratory:   bool,
}

// ── Pattern specs ─────────────────────────────────────────────────────────────

struct PatternSpec {
    name:        &'static str,
    ab_xa_ideal: f64,
    ab_xa_min:   f64,
    ab_xa_max:   f64,
    bc_ab_min:   f64,
    bc_ab_max:   f64,
    d_xa_ideal:  f64,
}

const PATTERNS: &[PatternSpec] = &[
    PatternSpec {
        name:        "Gartley",
        ab_xa_ideal: 0.618,
        ab_xa_min:   0.50,
        ab_xa_max:   0.786,
        bc_ab_min:   0.382,
        bc_ab_max:   0.886,
        d_xa_ideal:  0.786,
    },
    PatternSpec {
        name:        "Bat",
        ab_xa_ideal: 0.50,
        ab_xa_min:   0.382,
        ab_xa_max:   0.618,
        bc_ab_min:   0.382,
        bc_ab_max:   0.886,
        d_xa_ideal:  0.886,
    },
    PatternSpec {
        name:        "Butterfly",
        ab_xa_ideal: 0.786,
        ab_xa_min:   0.618,
        ab_xa_max:   0.886,
        bc_ab_min:   0.382,
        bc_ab_max:   0.886,
        d_xa_ideal:  1.272,
    },
    PatternSpec {
        name:        "Crab",
        ab_xa_ideal: 0.50,
        ab_xa_min:   0.382,
        ab_xa_max:   0.618,
        bc_ab_min:   0.382,
        bc_ab_max:   0.886,
        d_xa_ideal:  1.618,
    },
];

// ── Entry point ───────────────────────────────────────────────────────────────

pub fn compute_harmonic_patterns(raw: &[OhlcvCandle], profile: &FibProfile) -> Vec<HarmonicPattern> {
    if raw.len() < 10 { return vec![]; }

    let pivots = detect_pivots(raw);
    if pivots.len() < 5 { return vec![]; }

    let tol = profile.harmonic_ratio_tolerance;
    let mut results: Vec<HarmonicPattern> = Vec::new();

    'outer: for w in pivots.windows(5) {
        let (x, a, b, c, d) = (&w[0], &w[1], &w[2], &w[3], &w[4]);

        let (direction, xa, ab, bc, xd) = if x.kind == PivotKind::Low {
            // Bullish: X(L) A(H) B(L) C(H) D(L)
            if a.kind != PivotKind::High || b.kind != PivotKind::Low
                || c.kind != PivotKind::High || d.kind != PivotKind::Low
            {
                continue 'outer;
            }
            // Structural validity: B above X, C below A
            if b.price <= x.price || c.price >= a.price { continue 'outer; }
            (
                "bullish",
                a.price - x.price,
                a.price - b.price,
                c.price - b.price,
                a.price - d.price,
            )
        } else {
            // Bearish: X(H) A(L) B(H) C(L) D(H)
            if a.kind != PivotKind::Low || b.kind != PivotKind::High
                || c.kind != PivotKind::Low || d.kind != PivotKind::High
            {
                continue 'outer;
            }
            // Structural validity: B below X, C above A
            if b.price >= x.price || c.price <= a.price { continue 'outer; }
            (
                "bearish",
                x.price - a.price,
                b.price - a.price,
                b.price - c.price,
                d.price - a.price,
            )
        };

        if xa < 1e-10 || ab < 1e-10 || bc < 1e-10 { continue; }

        let ab_xa = ab / xa;
        let bc_ab = bc / ab;
        let xd_xa = xd / xa;

        for spec in PATTERNS {
            if !profile.harmonic_patterns.iter().any(|p| p == spec.name) { continue; }
            if ab_xa < spec.ab_xa_min - tol || ab_xa > spec.ab_xa_max + tol { continue; }
            if bc_ab < spec.bc_ab_min - tol || bc_ab > spec.bc_ab_max + tol { continue; }
            if (xd_xa - spec.d_xa_ideal).abs() > tol { continue; }

            let q_ab = (1.0 - (ab_xa - spec.ab_xa_ideal).abs() / tol).max(0.0);
            let q_d  = (1.0 - (xd_xa - spec.d_xa_ideal).abs() / tol).max(0.0);
            let quality = round2(q_ab * 0.4 + q_d * 0.6);

            results.push(HarmonicPattern {
                ts_x:          x.ts.clone(),
                ts_a:          a.ts.clone(),
                ts_b:          b.ts.clone(),
                ts_c:          c.ts.clone(),
                ts_d:          d.ts.clone(),
                pattern:       spec.name.to_string(),
                direction:     direction.to_string(),
                d_price:       round5(d.price),
                xabcd_quality: quality,
                exploratory:   profile.exploratory,
            });
        }
    }

    results
}

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

    fn gartley_candles() -> Vec<OhlcvCandle> {
        // Pivots: X(L)=0.500  A(H)=1.000  B(L)=0.691  C(H)=0.882  D(L)=0.607
        // AB/XA=0.618 (Gartley ideal), BC/AB=0.618, XD/XA=0.786 (key D ratio)
        // Each pivot fires on the 3-bar rule; no spurious pivots between them.
        vec![
            c("0",  0.80, 0.82,  0.55,  0.60),
            c("1",  0.60, 0.62,  0.50,  0.52),   // X low=0.500
            c("2",  0.52, 0.60,  0.53,  0.58),
            c("3",  0.60, 0.75,  0.59,  0.74),
            c("4",  0.74, 1.00,  0.73,  0.99),   // A high=1.000
            c("5",  0.99, 0.98,  0.80,  0.85),
            c("6",  0.85, 0.87,  0.72,  0.73),
            c("7",  0.73, 0.74,  0.691, 0.70),   // B low=0.691
            c("8",  0.70, 0.78,  0.695, 0.77),
            c("9",  0.77, 0.85,  0.76,  0.84),
            c("10", 0.84, 0.882, 0.83,  0.875),  // C high=0.882
            c("11", 0.875, 0.878, 0.75, 0.76),
            c("12", 0.76, 0.77,  0.65,  0.66),
            c("13", 0.66, 0.67,  0.607, 0.62),   // D low=0.607
            c("14", 0.62, 0.70,  0.615, 0.68),
            c("15", 0.70, 0.80,  0.69,  0.78),
        ]
    }

    #[test]
    fn returns_empty_when_too_few_candles() {
        let raw = vec![c("0", 1.0, 1.1, 0.9, 1.0)];
        assert!(compute_harmonic_patterns(&raw, &FibProfile::mature()).is_empty());
    }

    #[test]
    fn does_not_panic_on_flat_market() {
        let raw: Vec<_> = (0..30).map(|i| c(&i.to_string(), 1.0, 1.0, 1.0, 1.0)).collect();
        let _ = compute_harmonic_patterns(&raw, &FibProfile::mature());
    }

    #[test]
    fn does_not_panic_on_trending_market() {
        let raw: Vec<_> = (0..50)
            .map(|i| { let p = 1.0 + i as f64 * 0.01; c(&i.to_string(), p, p + 0.005, p - 0.003, p) })
            .collect();
        let _ = compute_harmonic_patterns(&raw, &FibProfile::mature());
    }

    #[test]
    fn profile_controls_enabled_patterns() {
        assert!(!FibProfile::nascent().harmonic_patterns.contains(&"Crab".to_string()));
        assert!(FibProfile::mature().harmonic_patterns.contains(&"Crab".to_string()));
    }

    #[test]
    fn synthetic_gartley_bullish_detected() {
        let found = compute_harmonic_patterns(&gartley_candles(), &FibProfile::mature());
        assert!(
            found.iter().any(|p| p.pattern == "Gartley" && p.direction == "bullish"),
            "Expected bullish Gartley, got: {:?}",
            found.iter().map(|p| format!("{}/{}", p.pattern, p.direction)).collect::<Vec<_>>()
        );
    }

    #[test]
    fn gartley_d_price_correct() {
        let found = compute_harmonic_patterns(&gartley_candles(), &FibProfile::mature());
        let g = found.iter().find(|p| p.pattern == "Gartley").expect("Gartley not found");
        assert!((g.d_price - 0.607).abs() < 1e-4, "d_price={}", g.d_price);
    }

    #[test]
    fn gartley_quality_in_range() {
        let found = compute_harmonic_patterns(&gartley_candles(), &FibProfile::mature());
        let g = found.iter().find(|p| p.pattern == "Gartley").expect("Gartley not found");
        assert!(g.xabcd_quality > 0.0 && g.xabcd_quality <= 1.0, "quality={}", g.xabcd_quality);
    }

    #[test]
    fn exploratory_flag_mirrors_profile() {
        let found_n = compute_harmonic_patterns(&gartley_candles(), &FibProfile::nascent());
        let found_m = compute_harmonic_patterns(&gartley_candles(), &FibProfile::mature());
        for p in &found_n { assert!(p.exploratory, "nascent pattern should be exploratory"); }
        for p in &found_m { assert!(!p.exploratory, "mature pattern should not be exploratory"); }
    }
}
