//! Deserialization of `GET /api/oauth/usage` responses.

use serde::Deserialize;

use crate::Result;

/// One rate-limit window. `utilization` is a percentage (0–100).
#[derive(Debug, Clone, Deserialize)]
pub struct UsageWindow {
    pub utilization: f64,
    /// ISO 8601, may be absent.
    #[serde(default)]
    pub resets_at: Option<String>,
}

/// The windows we render. Anything else in the response (`extra_usage`, new
/// fields) is ignored here but survives in `--json`, which prints the raw body.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Usage {
    pub five_hour: Option<UsageWindow>,
    pub seven_day: Option<UsageWindow>,
    pub seven_day_opus: Option<UsageWindow>,
    pub seven_day_sonnet: Option<UsageWindow>,
}

impl Usage {
    pub fn parse(body: &str) -> Result<Self> {
        Ok(serde_json::from_str(body)?)
    }

    /// Present windows with display labels, in render order.
    pub fn windows(&self) -> Vec<(&'static str, &UsageWindow)> {
        [
            ("5h", &self.five_hour),
            ("7d", &self.seven_day),
            ("7d opus", &self.seven_day_opus),
            ("7d sonnet", &self.seven_day_sonnet),
        ]
        .into_iter()
        .filter_map(|(label, w)| w.as_ref().map(|w| (label, w)))
        .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Shape documented in PLAN.md (fields beyond the windows are ignored).
    const FIXTURE: &str = r#"{
        "five_hour": {"utilization": 34.2, "resets_at": "2026-08-31T21:00:00+00:00"},
        "seven_day": {"utilization": 61.0, "resets_at": "2026-09-04T07:00:00+00:00"},
        "seven_day_opus": null,
        "seven_day_sonnet": {"utilization": 12.5, "resets_at": "2026-09-04T07:00:00+00:00"},
        "extra_usage": {"is_enabled": false, "monthly_limit": null}
    }"#;

    #[test]
    fn parses_fixture_and_skips_nulls() {
        let usage = Usage::parse(FIXTURE).unwrap();
        let windows = usage.windows();
        let labels: Vec<_> = windows.iter().map(|(l, _)| *l).collect();
        assert_eq!(labels, ["5h", "7d", "7d sonnet"]);
        assert_eq!(windows[0].1.utilization, 34.2);
        assert_eq!(
            windows[0].1.resets_at.as_deref(),
            Some("2026-08-31T21:00:00+00:00")
        );
    }

    #[test]
    fn tolerates_missing_fields() {
        let usage = Usage::parse(r#"{"five_hour": {"utilization": 0.0}}"#).unwrap();
        assert_eq!(usage.windows().len(), 1);
        assert!(usage.windows()[0].1.resets_at.is_none());
    }
}
