//! Telemetry sources. The real PM5 source (CSAFE polling, ADR 0003) plugs in
//! behind the same event vocabulary; until then [`FakePm5`] generates a
//! plausible deterministic workout so the whole pipeline runs without
//! hardware.

use monorail_core::telemetry::{MonitorSample, StrokePhase, StrokeSample, WorkoutSummary};

/// One thing a source observed during a tick.
#[derive(Debug, Clone, PartialEq)]
pub enum SourceEvent {
    Monitor(MonitorSample),
    Stroke(StrokeSample),
}

/// Deterministic synthetic erg: holds a target split/rate with slow sine
/// drift, integrates distance, emits a stroke whenever the stroke period
/// elapses. No randomness — same inputs, same workout (replayable tests).
pub struct FakePm5 {
    target_split_s: f32,
    target_spm: f32,
    elapsed_s: f64,
    distance_m: f64,
    stroke_count: u32,
    next_stroke_at_s: f64,
}

impl FakePm5 {
    pub fn new(target_split_s: f32, target_spm: f32) -> Self {
        Self {
            target_split_s,
            target_spm,
            elapsed_s: 0.0,
            distance_m: 0.0,
            stroke_count: 0,
            next_stroke_at_s: 0.0,
        }
    }

    /// Current split with a slow ±1.5 s drift, mimicking human pacing.
    fn split_s(&self) -> f32 {
        self.target_split_s + 1.5 * (self.elapsed_s / 45.0).sin() as f32
    }

    /// Current stroke rate with a slow ±0.8 spm drift.
    fn spm(&self) -> f32 {
        self.target_spm + 0.8 * (self.elapsed_s / 30.0).cos() as f32
    }

    /// Concept2 pace→power relation: watts = 2.80 / (sec-per-meter)^3.
    fn power_watts(&self) -> f32 {
        let pace_s_per_m = self.split_s() as f64 / 500.0;
        (2.80 / pace_s_per_m.powi(3)) as f32
    }

    /// Advance simulated time by `dt_s`, returning what happened.
    pub fn advance(&mut self, dt_s: f64) -> Vec<SourceEvent> {
        let mut events = Vec::new();

        self.elapsed_s += dt_s;
        let speed_m_per_s = 500.0 / self.split_s() as f64;
        self.distance_m += speed_m_per_s * dt_s;

        while self.elapsed_s >= self.next_stroke_at_s {
            self.stroke_count += 1;
            let period_s = 60.0 / self.spm() as f64;
            self.next_stroke_at_s += period_s;

            // Roughly 40% drive / 60% recovery within the stroke cycle.
            let drive_time_ms = (period_s * 0.4 * 1000.0) as u32;
            let recovery_time_ms = (period_s * 0.6 * 1000.0) as u32;
            events.push(SourceEvent::Stroke(StrokeSample {
                stroke_number: self.stroke_count,
                drive_time_ms,
                recovery_time_ms,
                stroke_rate_spm: self.spm(),
                power_watts: self.power_watts(),
                split_s_per_500m: self.split_s(),
                distance_m: self.distance_m,
                drive_length_m: Some(1.45),
            }));
        }

        events.push(SourceEvent::Monitor(MonitorSample {
            elapsed_s: self.elapsed_s,
            distance_m: self.distance_m,
            split_s_per_500m: self.split_s(),
            stroke_rate_spm: self.spm(),
            power_watts: self.power_watts(),
            heart_rate_bpm: None,
            phase: if (self.elapsed_s - (self.next_stroke_at_s - 60.0 / self.spm() as f64))
                < 60.0 / self.spm() as f64 * 0.4
            {
                StrokePhase::Drive
            } else {
                StrokePhase::Recovery
            },
        }));

        events
    }

    /// Totals so far, for the workout-end summary.
    pub fn summary(&self) -> WorkoutSummary {
        let avg_split = if self.distance_m > 0.0 {
            (self.elapsed_s / self.distance_m * 500.0) as f32
        } else {
            0.0
        };
        WorkoutSummary {
            duration_s: self.elapsed_s,
            distance_m: self.distance_m,
            avg_split_s_per_500m: avg_split,
            avg_stroke_rate_spm: if self.elapsed_s > 0.0 {
                self.stroke_count as f32 / (self.elapsed_s / 60.0) as f32
            } else {
                0.0
            },
            avg_power_watts: self.power_watts(),
            stroke_count: self.stroke_count,
            avg_heart_rate_bpm: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run_for(fake: &mut FakePm5, seconds: f64, hz: f64) -> Vec<SourceEvent> {
        let dt = 1.0 / hz;
        let mut all = Vec::new();
        let steps = (seconds * hz) as usize;
        for _ in 0..steps {
            all.extend(fake.advance(dt));
        }
        all
    }

    #[test]
    fn distance_is_monotonic_and_plausible() {
        let mut fake = FakePm5::new(120.0, 20.0);
        run_for(&mut fake, 60.0, 10.0);
        // At ~2:00/500m, one minute covers ~250 m.
        assert!(
            (fake.distance_m - 250.0).abs() < 10.0,
            "{}",
            fake.distance_m
        );
    }

    #[test]
    fn stroke_rate_matches_target() {
        let mut fake = FakePm5::new(120.0, 20.0);
        let events = run_for(&mut fake, 120.0, 10.0);
        let strokes = events
            .iter()
            .filter(|e| matches!(e, SourceEvent::Stroke(_)))
            .count();
        // 2 minutes at ~20 spm: ~40 strokes (allow drift slack).
        assert!((38..=43).contains(&strokes), "{strokes}");
    }

    #[test]
    fn stroke_numbers_increase_without_gaps() {
        let mut fake = FakePm5::new(115.0, 24.0);
        let events = run_for(&mut fake, 30.0, 10.0);
        let numbers: Vec<u32> = events
            .iter()
            .filter_map(|e| match e {
                SourceEvent::Stroke(s) => Some(s.stroke_number),
                _ => None,
            })
            .collect();
        let expected: Vec<u32> = (1..=numbers.len() as u32).collect();
        assert_eq!(numbers, expected);
    }

    #[test]
    fn power_is_physical() {
        let fake = FakePm5::new(120.0, 20.0);
        // 2:00/500m is canonically ~203 W on a Concept2.
        assert!(
            (fake.power_watts() - 203.0).abs() < 10.0,
            "{}",
            fake.power_watts()
        );
    }

    #[test]
    fn summary_reflects_run() {
        let mut fake = FakePm5::new(120.0, 20.0);
        run_for(&mut fake, 60.0, 10.0);
        let summary = fake.summary();
        assert!((summary.duration_s - 60.0).abs() < 0.01);
        assert!(summary.stroke_count > 0);
        assert!((summary.avg_split_s_per_500m - 120.0).abs() < 5.0);
    }
}
