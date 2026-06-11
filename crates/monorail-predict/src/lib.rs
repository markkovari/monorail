//! Prediction models (ADR 0007): classical, inspectable fits over feature
//! frames produced by `monorail-store` — critical-power/W′, Riegel scaling,
//! regression on engineered features. Never raw SQL, never raw messages.

pub mod critical_power;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub use critical_power::CriticalPowerModel;

/// Features extracted from the athlete's history; produced by typed
/// `monorail-store` queries. Fields grow with the first real model.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FeatureFrame {
    /// Watermark: features computed from data up to this instant. Stored
    /// with every model fit so predictions are reproducible.
    pub data_through: DateTime<Utc>,
}

/// A prediction with provenance.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Prediction<T> {
    pub value: T,
    /// 0.0–1.0; models define their own calibration, residual tracking
    /// keeps them honest.
    pub confidence: f64,
    /// Identifies the model family + fit that produced this.
    pub model: String,
}

/// A fitted model that can judge/forecast performance.
pub trait Predictor {
    type Output;

    fn name(&self) -> &'static str;

    fn predict(&self, features: &FeatureFrame) -> Prediction<Self::Output>;
}

/// Feasibility check used by plan generation (ADR 0009): can the athlete
/// plausibly hold `target_split_s` for `duration_s`?
pub trait FeasibilityJudge {
    fn sustainable_split_s(&self, duration_s: f64) -> Option<f64>;
}
