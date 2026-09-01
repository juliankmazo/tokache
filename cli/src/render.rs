//! Terminal gauges and human time formatting. Pure functions, no I/O.

use chrono::{DateTime, Local, TimeZone, Utc};
use tokache_core::usage::Usage;

const BAR_WIDTH: usize = 30;

/// `[██████░░░░]` for a 0–100 percentage, clamped.
pub fn bar(percent: f64) -> String {
    let clamped = percent.clamp(0.0, 100.0);
    let filled = ((clamped / 100.0) * BAR_WIDTH as f64).round() as usize;
    format!("[{}{}]", "█".repeat(filled), "░".repeat(BAR_WIDTH - filled))
}

/// "2h 14m", "3d 2h", "45m", "<1m". Never negative.
pub fn human_duration(secs: i64) -> String {
    let secs = secs.max(0);
    let (d, h, m) = (secs / 86_400, (secs % 86_400) / 3_600, (secs % 3_600) / 60);
    match (d, h, m) {
        (0, 0, 0) => "<1m".into(),
        (0, 0, m) => format!("{m}m"),
        (0, h, m) => format!("{h}h {m}m"),
        (d, h, _) => format!("{d}d {h}h"),
    }
}

/// "resets in 2h 14m (at 4:00 PM)", with a weekday once it's >24h out.
/// Generic over the timezone so tests can pin one.
pub fn reset_line<Tz: TimeZone>(resets_at: &str, now: DateTime<Utc>, tz: &Tz) -> Option<String>
where
    Tz::Offset: std::fmt::Display,
{
    let at = DateTime::parse_from_rfc3339(resets_at)
        .ok()?
        .with_timezone(&Utc);
    let secs = (at - now).num_seconds();
    let local = at.with_timezone(tz);
    let clock = if secs >= 86_400 {
        local.format("%a %-I:%M %p")
    } else {
        local.format("at %-I:%M %p")
    };
    Some(format!("resets in {} ({})", human_duration(secs), clock))
}

/// One line per window: `5h        [██░░…]  34%  resets in …`.
pub fn gauges(usage: &Usage, now: DateTime<Utc>, color: bool) -> Vec<String> {
    usage
        .windows()
        .into_iter()
        .map(|(label, w)| {
            let pct = format!("{:>3.0}%", w.utilization);
            let bar = if color {
                colorize(w.utilization, &bar(w.utilization))
            } else {
                bar(w.utilization)
            };
            let reset = w
                .resets_at
                .as_deref()
                .and_then(|r| reset_line(r, now, &Local))
                .unwrap_or_default();
            format!("{label:<10}{bar}  {pct}  {reset}")
                .trim_end()
                .to_string()
        })
        .collect()
}

fn colorize(percent: f64, bar: &str) -> String {
    let code = if percent >= 90.0 {
        "31" // red
    } else if percent >= 70.0 {
        "33" // yellow
    } else {
        "32" // green
    };
    format!("\x1b[{code}m{bar}\x1b[0m")
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::FixedOffset;

    #[test]
    fn bar_edges() {
        assert_eq!(bar(0.0), format!("[{}]", "░".repeat(30)));
        assert_eq!(bar(100.0), format!("[{}]", "█".repeat(30)));
        assert_eq!(bar(250.0), format!("[{}]", "█".repeat(30))); // clamped
        assert_eq!(bar(50.0), format!("[{}{}]", "█".repeat(15), "░".repeat(15)));
    }

    #[test]
    fn durations() {
        assert_eq!(human_duration(30), "<1m");
        assert_eq!(human_duration(45 * 60), "45m");
        assert_eq!(human_duration(2 * 3600 + 14 * 60), "2h 14m");
        assert_eq!(human_duration(3 * 86400 + 2 * 3600 + 59 * 60), "3d 2h");
        assert_eq!(human_duration(-5), "<1m");
    }

    #[test]
    fn reset_line_same_day_and_far() {
        let tz = FixedOffset::west_opt(5 * 3600).unwrap(); // UTC-5
        let now = Utc.with_ymd_and_hms(2026, 8, 31, 18, 46, 0).unwrap();
        // 21:00 UTC = 4:00 PM UTC-5, 2h14m away.
        assert_eq!(
            reset_line("2026-08-31T21:00:00+00:00", now, &tz).unwrap(),
            "resets in 2h 14m (at 4:00 PM)"
        );
        // >24h out: weekday shown. Sep 4 2026 is a Friday; 07:00 UTC = 2:00 AM UTC-5.
        assert_eq!(
            reset_line("2026-09-04T07:00:00+00:00", now, &tz).unwrap(),
            "resets in 3d 12h (Fri 2:00 AM)"
        );
        assert!(reset_line("garbage", now, &tz).is_none());
    }

    #[test]
    fn gauge_lines() {
        let usage = Usage::parse(
            r#"{"five_hour":{"utilization":34.0},"seven_day_opus":null,
               "seven_day_sonnet":{"utilization":100.0}}"#,
        )
        .unwrap();
        let lines = gauges(&usage, Utc::now(), false);
        assert_eq!(lines.len(), 2);
        assert!(lines[0].starts_with("5h        ["));
        assert!(lines[0].ends_with(" 34%"));
        assert!(lines[1].starts_with("7d sonnet ["));
        assert!(lines[1].ends_with("100%"));
    }
}
