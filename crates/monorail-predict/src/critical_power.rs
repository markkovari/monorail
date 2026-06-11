//! Critical-power / W′ model (ADR 0007).
//!
//! The two-parameter hyperbolic model `P(t) = CP + W′/t` linearizes as
//! `work(t) = CP·t + W′`, so a least-squares line over per-effort
//! `(duration, total work)` points yields CP (slope) and W′ (intercept).
//! Inputs are whole-session average powers — honest for steady erg work,
//! refined later when interval efforts are split out.

use crate::FeasibilityJudge;

// Pace↔power relations live with the other Concept2 metrics (ADR 0012);
// re-exported here so this module's API is unchanged.
pub use monorail_core::metrics::{split_s_to_watts, watts_to_split_s};

/// Fitted critical-power model.
#[derive(Debug, Clone, PartialEq)]
pub struct CriticalPowerModel {
    /// Sustainable aerobic power, watts.
    pub cp_watts: f64,
    /// Anaerobic work capacity, joules.
    pub w_prime_j: f64,
}

impl CriticalPowerModel {
    /// Minimum effort duration considered; shorter sessions say nothing
    /// about aerobic capacity.
    pub const MIN_EFFORT_S: f64 = 120.0;

    /// Fit from `(duration_s, avg_power_w)` efforts. Returns `None` without
    /// at least two sufficiently long efforts with distinct durations, or
    /// when the fit is unphysical (CP or W′ non-positive).
    pub fn fit(efforts: &[(f64, f64)]) -> Option<Self> {
        let points: Vec<(f64, f64)> = efforts
            .iter()
            .filter(|(t, p)| *t >= Self::MIN_EFFORT_S && *p > 0.0)
            .map(|(t, p)| (*t, t * p)) // (duration, total work)
            .collect();
        if points.len() < 2 {
            return None;
        }

        let n = points.len() as f64;
        let sum_t: f64 = points.iter().map(|(t, _)| t).sum();
        let sum_w: f64 = points.iter().map(|(_, w)| w).sum();
        let sum_tt: f64 = points.iter().map(|(t, _)| t * t).sum();
        let sum_tw: f64 = points.iter().map(|(t, w)| t * w).sum();

        let denom = n * sum_tt - sum_t * sum_t;
        if denom.abs() < f64::EPSILON {
            return None; // all efforts the same duration
        }
        let cp = (n * sum_tw - sum_t * sum_w) / denom;
        let w_prime = (sum_w - cp * sum_t) / n;

        (cp > 0.0 && w_prime > 0.0).then_some(Self {
            cp_watts: cp,
            w_prime_j: w_prime,
        })
    }

    /// Power sustainable for `duration_s` under the model.
    pub fn sustainable_watts(&self, duration_s: f64) -> f64 {
        self.cp_watts + self.w_prime_j / duration_s
    }
}

impl FeasibilityJudge for CriticalPowerModel {
    fn sustainable_split_s(&self, duration_s: f64) -> Option<f64> {
        if duration_s <= 0.0 {
            return None;
        }
        Some(watts_to_split_s(self.sustainable_watts(duration_s)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Synthetic athlete: CP 200 W, W′ 15 kJ.
    fn synthetic_efforts() -> Vec<(f64, f64)> {
        [240.0, 480.0, 1200.0, 2400.0]
            .iter()
            .map(|&t| (t, 200.0 + 15_000.0 / t))
            .collect()
    }

    #[test]
    fn fit_recovers_synthetic_athlete() {
        let model = CriticalPowerModel::fit(&synthetic_efforts()).unwrap();
        assert!((model.cp_watts - 200.0).abs() < 1.0, "{}", model.cp_watts);
        assert!(
            (model.w_prime_j - 15_000.0).abs() < 200.0,
            "{}",
            model.w_prime_j
        );
    }

    #[test]
    fn refuses_insufficient_or_degenerate_data() {
        assert_eq!(CriticalPowerModel::fit(&[]), None);
        assert_eq!(CriticalPowerModel::fit(&[(2400.0, 200.0)]), None);
        // Two efforts, same duration: slope undefined.
        assert_eq!(
            CriticalPowerModel::fit(&[(600.0, 210.0), (600.0, 215.0)]),
            None
        );
        // Too short to mean anything.
        assert_eq!(
            CriticalPowerModel::fit(&[(30.0, 400.0), (60.0, 350.0)]),
            None
        );
    }

    #[test]
    fn judge_predicts_slower_split_for_longer_efforts() {
        let model = CriticalPowerModel::fit(&synthetic_efforts()).unwrap();
        let short = model.sustainable_split_s(300.0).unwrap();
        let long = model.sustainable_split_s(3600.0).unwrap();
        assert!(
            short < long,
            "short {short} should be faster than long {long}"
        );
        assert_eq!(model.sustainable_split_s(0.0), None);
    }
}
