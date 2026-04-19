/// Swing pivot detection — shared by fib_confluence and harmonics.
///
/// detect_pivots: 3-bar high/low detection producing an alternating H/L sequence.
/// deduplicate_pivots: collapse consecutive same-kind pivots to the extremum.

use crate::domain::candle::OhlcvCandle;

#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) enum PivotKind { High, Low }

pub(crate) struct Pivot {
    pub ts:    String,
    pub price: f64,
    pub kind:  PivotKind,
}

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

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn alternating_pivots_are_not_deduplicated() {
        let pivots = vec![
            Pivot { ts: "a".into(), price: 1.0, kind: PivotKind::Low },
            Pivot { ts: "b".into(), price: 2.0, kind: PivotKind::High },
            Pivot { ts: "c".into(), price: 0.5, kind: PivotKind::Low },
        ];
        let deduped = deduplicate_pivots(pivots);
        assert_eq!(deduped.len(), 3);
    }
}
