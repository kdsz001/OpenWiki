//! Detects time-bound phrases in user questions and converts them into
//! a (start, end) UTC range for use as a SQL `created_at` filter in
//! the Q&A retrieval pipeline.
//!
//! Why detect at all: the LLM cannot filter by `created_at` if the
//! retrieval layer never narrows the candidate set in the first place.
//! By spotting common temporal expressions client-side, we let SQL
//! return only date-relevant rows before the AI ever sees them.
//!
//! What this is NOT: a full natural-language date parser. It covers
//! the high-frequency patterns ("最近一周" / "今天" / "last month"),
//! and falls back to None for ambiguous input — in which case the
//! retrieval layer simply uses FTS without a date constraint, and the
//! LLM still has the current date in context to reason about absolute
//! dates if needed.

use chrono::{DateTime, Datelike, Duration, NaiveDate, TimeZone, Utc};

#[derive(Debug, Clone, PartialEq)]
pub struct TimeRange {
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
    /// Human-readable label for logs / debugging.
    pub label: String,
}

impl TimeRange {
    pub fn iso_start(&self) -> String {
        self.start.format("%Y-%m-%dT%H:%M:%SZ").to_string()
    }
    pub fn iso_end(&self) -> String {
        self.end.format("%Y-%m-%dT%H:%M:%SZ").to_string()
    }
}

/// Detect a temporal phrase in `query` and return a UTC range.
/// `now` is passed in for testability (and so the caller controls the
/// "today" reference — important when the user's locale differs from UTC).
pub fn detect_time_range(query: &str, now: DateTime<Utc>) -> Option<TimeRange> {
    let q = query.to_lowercase();

    // ---- Anchor: today ----
    let today = now.date_naive();
    let day_start = |d: NaiveDate| Utc.from_utc_datetime(&d.and_hms_opt(0, 0, 0).unwrap());
    let day_end = |d: NaiveDate| Utc.from_utc_datetime(&d.and_hms_opt(23, 59, 59).unwrap());

    // ---- Specific named days ----
    if q.contains("今天") || q.contains("今日") || q.contains("today") {
        return Some(TimeRange {
            start: day_start(today),
            end: day_end(today),
            label: "today".into(),
        });
    }
    if q.contains("昨天") || q.contains("昨日") || q.contains("yesterday") {
        let d = today - Duration::days(1);
        return Some(TimeRange {
            start: day_start(d),
            end: day_end(d),
            label: "yesterday".into(),
        });
    }
    if q.contains("前天") {
        let d = today - Duration::days(2);
        return Some(TimeRange {
            start: day_start(d),
            end: day_end(d),
            label: "day before yesterday".into(),
        });
    }

    // ---- Week ranges ----
    // Treat 本周 as Monday-of-this-week through today (inclusive).
    if q.contains("本周") || q.contains("这周") || q.contains("this week") {
        let weekday = today.weekday().num_days_from_monday() as i64;
        let monday = today - Duration::days(weekday);
        return Some(TimeRange {
            start: day_start(monday),
            end: day_end(today),
            label: "this week".into(),
        });
    }
    if q.contains("上周") || q.contains("上个星期") || q.contains("last week") {
        let weekday = today.weekday().num_days_from_monday() as i64;
        let monday_this = today - Duration::days(weekday);
        let monday_last = monday_this - Duration::days(7);
        let sunday_last = monday_this - Duration::days(1);
        return Some(TimeRange {
            start: day_start(monday_last),
            end: day_end(sunday_last),
            label: "last week".into(),
        });
    }

    // ---- Month ranges ----
    if q.contains("本月") || q.contains("这个月") || q.contains("this month") {
        let first = NaiveDate::from_ymd_opt(today.year(), today.month(), 1).unwrap();
        return Some(TimeRange {
            start: day_start(first),
            end: day_end(today),
            label: "this month".into(),
        });
    }
    if q.contains("上月") || q.contains("上个月") || q.contains("last month") {
        let (y, m) = if today.month() == 1 {
            (today.year() - 1, 12)
        } else {
            (today.year(), today.month() - 1)
        };
        let first = NaiveDate::from_ymd_opt(y, m, 1).unwrap();
        // last day of that month
        let next_first = if m == 12 {
            NaiveDate::from_ymd_opt(y + 1, 1, 1).unwrap()
        } else {
            NaiveDate::from_ymd_opt(y, m + 1, 1).unwrap()
        };
        let last = next_first - Duration::days(1);
        return Some(TimeRange {
            start: day_start(first),
            end: day_end(last),
            label: "last month".into(),
        });
    }

    // ---- "最近 N 天/周/月" / "recent N days" / "past N weeks" ----
    if let Some(range) = parse_recent_n(&q, now, today) {
        return Some(range);
    }

    // ---- "N 天前" / "N weeks ago" — single anchored day ----
    if let Some(range) = parse_n_units_ago(&q, today) {
        return Some(range);
    }

    // ---- Loose "最近" / "近来" / "前阵子" / "recently" → default last 7 days ----
    if q.contains("最近") || q.contains("近来") || q.contains("前阵子")
        || q.contains("这几天") || q.contains("recently") || q.contains("recent")
    {
        let start_day = today - Duration::days(6);
        return Some(TimeRange {
            start: day_start(start_day),
            end: day_end(today),
            label: "recent (default 7 days)".into(),
        });
    }

    None
}

/// Parse "最近N天/周/月" and English equivalents.
fn parse_recent_n(q: &str, now: DateTime<Utc>, today: NaiveDate) -> Option<TimeRange> {
    let day_start = |d: NaiveDate| Utc.from_utc_datetime(&d.and_hms_opt(0, 0, 0).unwrap());
    let day_end = |d: NaiveDate| Utc.from_utc_datetime(&d.and_hms_opt(23, 59, 59).unwrap());

    // Patterns like "最近5天", "最近 3 周", "past 7 days", "last 2 months"
    // We scan for a number followed by a CN unit char or EN unit word.
    let chars: Vec<char> = q.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i].is_ascii_digit() {
            let start = i;
            while i < chars.len() && chars[i].is_ascii_digit() {
                i += 1;
            }
            let num: i64 = chars[start..i].iter().collect::<String>().parse().ok()?;
            // Skip whitespace
            while i < chars.len() && chars[i].is_whitespace() {
                i += 1;
            }
            // Look ahead for a unit
            let tail: String = chars[i..].iter().take(8).collect();
            let unit_days: Option<i64> = if tail.starts_with('天') || tail.starts_with("day") {
                Some(num)
            } else if tail.starts_with('周')
                || tail.starts_with("星期")
                || tail.starts_with("week")
            {
                Some(num * 7)
            } else if tail.starts_with("个月") || tail.starts_with('月') || tail.starts_with("month")
            {
                Some(num * 30)
            } else {
                None
            };
            if let Some(days) = unit_days {
                // Only treat as "recent N" if a recency word appears in the query.
                if q.contains("最近") || q.contains("近") || q.contains("过去")
                    || q.contains("past") || q.contains("last") || q.contains("recent")
                {
                    let start_day = today - Duration::days(days - 1);
                    let _ = now;
                    return Some(TimeRange {
                        start: day_start(start_day),
                        end: day_end(today),
                        label: format!("recent {} days", days),
                    });
                }
            }
        } else {
            i += 1;
        }
    }
    None
}

/// Parse "3天前", "2 weeks ago" — point in past, returns single-day range.
fn parse_n_units_ago(q: &str, today: NaiveDate) -> Option<TimeRange> {
    let day_start = |d: NaiveDate| Utc.from_utc_datetime(&d.and_hms_opt(0, 0, 0).unwrap());
    let day_end = |d: NaiveDate| Utc.from_utc_datetime(&d.and_hms_opt(23, 59, 59).unwrap());

    let chars: Vec<char> = q.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i].is_ascii_digit() {
            let start = i;
            while i < chars.len() && chars[i].is_ascii_digit() {
                i += 1;
            }
            let num: i64 = chars[start..i].iter().collect::<String>().parse().ok()?;
            while i < chars.len() && chars[i].is_whitespace() {
                i += 1;
            }
            let tail: String = chars[i..].iter().take(12).collect();
            let days: Option<i64> = if tail.starts_with("天前") {
                Some(num)
            } else if tail.starts_with("周前") || tail.starts_with("星期前") {
                Some(num * 7)
            } else if tail.starts_with("个月前") || tail.starts_with("月前") {
                Some(num * 30)
            } else if tail.contains("days ago") || tail.contains("day ago") {
                Some(num)
            } else if tail.contains("weeks ago") || tail.contains("week ago") {
                Some(num * 7)
            } else if tail.contains("months ago") || tail.contains("month ago") {
                Some(num * 30)
            } else {
                None
            };
            if let Some(d) = days {
                let day = today - Duration::days(d);
                return Some(TimeRange {
                    start: day_start(day),
                    end: day_end(day),
                    label: format!("{} days ago", d),
                });
            }
        } else {
            i += 1;
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixed_now() -> DateTime<Utc> {
        // Sunday, 2026-04-26 12:00:00 UTC — same as today's date for stability
        Utc.with_ymd_and_hms(2026, 4, 26, 12, 0, 0).unwrap()
    }

    #[test]
    fn detects_today() {
        let r = detect_time_range("今天保存了啥", fixed_now()).unwrap();
        assert_eq!(r.label, "today");
        assert!(r.iso_start().starts_with("2026-04-26"));
        assert!(r.iso_end().starts_with("2026-04-26"));
    }

    #[test]
    fn detects_yesterday() {
        let r = detect_time_range("昨天我看了啥", fixed_now()).unwrap();
        assert_eq!(r.label, "yesterday");
        assert!(r.iso_start().starts_with("2026-04-25"));
    }

    #[test]
    fn detects_last_week() {
        let r = detect_time_range("上周关于 AI 的内容", fixed_now()).unwrap();
        assert_eq!(r.label, "last week");
        // Week of 2026-04-13 (Mon) through 2026-04-19 (Sun)
        assert!(r.iso_start().starts_with("2026-04-13"));
        assert!(r.iso_end().starts_with("2026-04-19"));
    }

    #[test]
    fn detects_recent_7_days() {
        let r = detect_time_range("最近一周保存了什么", fixed_now()).unwrap();
        // "最近一周" doesn't match parse_recent_n (no digit), falls through to
        // the loose "最近" branch which defaults to 7 days.
        assert_eq!(r.label, "recent (default 7 days)");
        assert!(r.iso_start().starts_with("2026-04-20"));
    }

    #[test]
    fn detects_recent_n_days_with_digit() {
        let r = detect_time_range("最近5天有啥", fixed_now()).unwrap();
        assert_eq!(r.label, "recent 5 days");
        assert!(r.iso_start().starts_with("2026-04-22"));
    }

    #[test]
    fn detects_past_n_weeks_english() {
        let r = detect_time_range("show me past 2 weeks", fixed_now()).unwrap();
        assert_eq!(r.label, "recent 14 days");
    }

    #[test]
    fn detects_n_days_ago() {
        let r = detect_time_range("3天前关注的", fixed_now()).unwrap();
        assert_eq!(r.label, "3 days ago");
        assert!(r.iso_start().starts_with("2026-04-23"));
    }

    #[test]
    fn detects_this_month() {
        let r = detect_time_range("本月有啥", fixed_now()).unwrap();
        assert_eq!(r.label, "this month");
        assert!(r.iso_start().starts_with("2026-04-01"));
    }

    #[test]
    fn detects_last_month() {
        let r = detect_time_range("上个月看了啥", fixed_now()).unwrap();
        assert_eq!(r.label, "last month");
        assert!(r.iso_start().starts_with("2026-03-01"));
        assert!(r.iso_end().starts_with("2026-03-31"));
    }

    #[test]
    fn no_match_returns_none() {
        assert!(detect_time_range("AI 是什么", fixed_now()).is_none());
        assert!(detect_time_range("怎么写 Rust", fixed_now()).is_none());
    }

    #[test]
    fn loose_recently_word() {
        let r = detect_time_range("最近关注的内容", fixed_now()).unwrap();
        assert_eq!(r.label, "recent (default 7 days)");
    }
}
