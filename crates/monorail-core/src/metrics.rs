//! Concept2 metric relations (ADR 0012): pace↔power and calories,
//! including weight adjustment. Pure functions; SI units at the boundary
//! (kg, watts, seconds) — pounds exist only inside the published formula.

use serde::{Deserialize, Serialize};

/// Concept2 pace↔power relation constant: `watts = 2.80 / (s/m)³`.
pub const C2_PACE_CUBE: f64 = 2.80;

/// Exact physics: 1 watt-hour = 0.8604 kcal.
pub const KCAL_PER_WATT_HR: f64 = 0.8604;

/// PM5 assumes ~25% human mechanical efficiency: metabolic = 4 × mechanical.
pub const METABOLIC_FACTOR: f64 = 4.0;

/// Baseline burn the PM5 assumes, for its 175 lb reference athlete.
pub const PM_BASE_KCAL_PER_HR: f64 = 300.0;

/// Concept2's published per-pound baseline (1.714 × 175 lb = 300).
pub const KCAL_PER_HR_PER_LB: f64 = 1.714;

pub const LB_PER_KG: f64 = 2.204_622_6;

/// The reference athlete weight baked into PM5 calories.
pub const PM_REFERENCE_WEIGHT_KG: f64 = 175.0 / LB_PER_KG;

/// Athlete state that cannot be derived from telemetry (ADR 0012).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct AthleteProfile {
    pub weight_kg: f32,
}

/// Convert watts to split (seconds per 500 m).
pub fn watts_to_split_s(watts: f64) -> f64 {
    (C2_PACE_CUBE / watts).cbrt() * 500.0
}

/// Convert split (seconds per 500 m) to watts.
pub fn split_s_to_watts(split_s: f64) -> f64 {
    C2_PACE_CUBE / (split_s / 500.0).powi(3)
}

/// Calories/hour as the PM5 displays them (175 lb reference athlete):
/// `kcal/hr = watts × 4 × 0.8604 + 300`.
pub fn pm_kcal_per_hr(watts: f64) -> f64 {
    watts * METABOLIC_FACTOR * KCAL_PER_WATT_HR + PM_BASE_KCAL_PER_HR
}

/// Concept2's weight correction: replace the reference baseline with one
/// scaled to actual body weight:
/// `true kcal/hr = PM kcal/hr − 300 + 1.714 × weight_lb`.
pub fn weight_adjusted_kcal_per_hr(watts: f64, weight_kg: f64) -> f64 {
    pm_kcal_per_hr(watts) - PM_BASE_KCAL_PER_HR + KCAL_PER_HR_PER_LB * weight_kg * LB_PER_KG
}

/// Total calories for a workout at a steady rate.
pub fn workout_kcal(kcal_per_hr: f64, duration_s: f64) -> f64 {
    kcal_per_hr * duration_s / 3600.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pace_power_relation_round_trips() {
        // Canonical Concept2 point: 2:00/500m ≈ 203 W.
        assert!((split_s_to_watts(120.0) - 202.5).abs() < 1.0);
        assert!((watts_to_split_s(202.5) - 120.0).abs() < 0.1);
        let w = split_s_to_watts(watts_to_split_s(250.0));
        assert!((w - 250.0).abs() < 1e-6);
    }

    #[test]
    fn adjusted_equals_pm_at_reference_weight() {
        // 1.714 × 175 lb = 299.95 ≈ the 300 baseline; formulas agree at the
        // reference athlete to within the published constants' rounding.
        let pm = pm_kcal_per_hr(200.0);
        let adjusted = weight_adjusted_kcal_per_hr(200.0, PM_REFERENCE_WEIGHT_KG);
        assert!(
            (pm - adjusted).abs() < 0.1,
            "pm {pm} vs adjusted {adjusted}"
        );
    }

    #[test]
    fn heavier_athletes_burn_more() {
        let light = weight_adjusted_kcal_per_hr(200.0, 60.0);
        let heavy = weight_adjusted_kcal_per_hr(200.0, 100.0);
        assert!(heavy > light);
        // 40 kg ≈ 88.2 lb difference ⇒ ≈ 151 kcal/hr difference.
        assert!((heavy - light - 151.1).abs() < 1.0, "{}", heavy - light);
    }

    #[test]
    fn hour_at_two_minute_split_is_about_a_thousand_kcal() {
        // 2:00/500m ⇒ ~202.5 W ⇒ PM ~997 kcal/hr.
        let kcal = workout_kcal(pm_kcal_per_hr(split_s_to_watts(120.0)), 3600.0);
        assert!((kcal - 997.0).abs() < 5.0, "{kcal}");
    }
}
