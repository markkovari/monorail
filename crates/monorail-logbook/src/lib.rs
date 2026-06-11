//! Read-only Concept2 Logbook API client (ADR 0013).
//!
//! ErgData and the PM5 sync results to `log.concept2.com`; this crate pulls
//! them with a user-supplied OAuth access token. Deserialization is
//! tolerant (unknown fields ignored, optionals defaulted) and the checked-in
//! fixture in `tests/fixtures/` documents the schema we rely on, per the
//! ADR 0005 discipline.

use serde::Deserialize;
use thiserror::Error;

pub const DEFAULT_BASE_URL: &str = "https://log.concept2.com";

#[derive(Debug, Error)]
pub enum LogbookError {
    #[error("http error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("logbook answered {status}: {body}")]
    Api { status: u16, body: String },
}

/// One result as the Logbook reports it. Field set is the subset we rely
/// on; everything else stays in the raw JSON.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct LogbookResult {
    pub id: u64,
    /// Local date-time string as the Logbook formats it.
    pub date: String,
    /// Meters.
    #[serde(default)]
    pub distance: Option<f64>,
    /// Logbook reports time in tenths of a second.
    #[serde(default)]
    pub time: Option<u64>,
    #[serde(default)]
    pub calories_total: Option<u32>,
    #[serde(default)]
    pub stroke_rate: Option<u32>,
    #[serde(rename = "type", default)]
    pub kind: Option<String>,
}

impl LogbookResult {
    pub fn duration_s(&self) -> Option<f64> {
        self.time.map(|deciseconds| deciseconds as f64 / 10.0)
    }
}

#[derive(Debug, Deserialize)]
struct ResultsPage {
    data: Vec<serde_json::Value>,
    #[serde(default)]
    meta: Option<Meta>,
}

#[derive(Debug, Deserialize)]
struct Meta {
    #[serde(default)]
    pagination: Option<Pagination>,
}

#[derive(Debug, Deserialize)]
struct Pagination {
    #[serde(default)]
    links: Option<PaginationLinks>,
}

#[derive(Debug, Deserialize)]
struct PaginationLinks {
    #[serde(default)]
    next: Option<String>,
}

/// A fetched result: parsed subset + lossless raw JSON for storage.
#[derive(Debug, Clone, PartialEq)]
pub struct FetchedResult {
    pub result: LogbookResult,
    pub raw: String,
}

/// Parse one results page; returns the parsed results and the next-page URL.
/// Separated from I/O so fixtures can exercise it.
fn parse_page(body: &str) -> Result<(Vec<FetchedResult>, Option<String>), serde_json::Error> {
    let page: ResultsPage = serde_json::from_str(body)?;
    let next = page
        .meta
        .and_then(|m| m.pagination)
        .and_then(|p| p.links)
        .and_then(|l| l.next);
    let results = page
        .data
        .into_iter()
        .filter_map(|value| {
            let raw = value.to_string();
            match serde_json::from_value::<LogbookResult>(value) {
                Ok(result) => Some(FetchedResult { result, raw }),
                Err(error) => {
                    tracing::warn!(%error, "skipping unparseable logbook result");
                    None
                }
            }
        })
        .collect();
    Ok((results, next))
}

/// Authenticated Logbook client.
pub struct LogbookClient {
    http: reqwest::Client,
    base_url: String,
    token: String,
}

impl LogbookClient {
    pub fn new(token: impl Into<String>) -> Self {
        Self::with_base_url(DEFAULT_BASE_URL, token)
    }

    pub fn with_base_url(base_url: impl Into<String>, token: impl Into<String>) -> Self {
        Self {
            http: reqwest::Client::new(),
            base_url: base_url.into(),
            token: token.into(),
        }
    }

    /// Fetch all rower results, following pagination.
    pub async fn fetch_results(&self) -> Result<Vec<FetchedResult>, LogbookError> {
        let mut url = format!("{}/api/users/me/results?type=rower", self.base_url);
        let mut all = Vec::new();
        loop {
            let response = self
                .http
                .get(&url)
                .bearer_auth(&self.token)
                .header("accept", "application/json")
                .send()
                .await?;
            let status = response.status();
            let body = response.text().await?;
            if !status.is_success() {
                return Err(LogbookError::Api {
                    status: status.as_u16(),
                    body: body.chars().take(200).collect(),
                });
            }
            let (results, next) = parse_page(&body).map_err(|error| LogbookError::Api {
                status: status.as_u16(),
                body: format!("unparseable page: {error}"),
            })?;
            all.extend(results);
            match next {
                Some(next_url) => url = next_url,
                None => break,
            }
        }
        Ok(all)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = include_str!("../tests/fixtures/results_page.json");

    #[test]
    fn fixture_page_parses_with_pagination() {
        let (results, next) = parse_page(FIXTURE).unwrap();
        assert_eq!(results.len(), 2);

        let first = &results[0].result;
        assert_eq!(first.id, 1_000_001);
        assert_eq!(first.distance, Some(10_000.0));
        assert_eq!(first.duration_s(), Some(2400.0));
        assert_eq!(first.calories_total, Some(702));
        assert_eq!(first.stroke_rate, Some(20));
        assert_eq!(first.kind.as_deref(), Some("rower"));
        // Raw JSON kept lossless, including fields we don't model.
        assert!(results[0].raw.contains("heart_rate"));

        assert_eq!(
            next.as_deref(),
            Some("https://log.concept2.com/api/users/me/results?type=rower&page=2")
        );
    }

    #[test]
    fn last_page_has_no_next_and_tolerates_sparse_results() {
        let body = r#"{
            "data": [{"id": 7, "date": "2026-06-01 07:00:00"}],
            "meta": {"pagination": {"links": {}}}
        }"#;
        let (results, next) = parse_page(body).unwrap();
        assert_eq!(next, None);
        assert_eq!(results.len(), 1);
        let r = &results[0].result;
        assert_eq!(r.distance, None);
        assert_eq!(r.duration_s(), None);
        assert_eq!(r.calories_total, None);
    }

    #[test]
    fn unparseable_entries_are_skipped_not_fatal() {
        let body = r#"{"data": [{"no_id_here": true}, {"id": 8, "date": "2026-06-01 07:00:00"}]}"#;
        let (results, next) = parse_page(body).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].result.id, 8);
        assert_eq!(next, None);
    }
}
