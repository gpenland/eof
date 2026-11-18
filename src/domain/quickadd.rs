//! Natural-language quick add, server side: "Quiz 4 fri #cs2110 !" → title, due, tags, star.
//!
//! A port of `web/js/quickadd.js` (which runs live in the composer to draw chips) so callers that
//! only have a sentence — an iOS Shortcut, Siri, an MCP client — get the same understanding.
//! Keep the two in step when adding rules.

use std::sync::LazyLock;

use chrono::{Datelike, Duration, Months, NaiveDate, Weekday};
use regex::{Captures, Regex};
use serde::Serialize;

#[derive(Debug, Default, PartialEq, Serialize)]
pub struct Parsed {
    pub title: String,
    pub due: Option<NaiveDate>,
    pub repeat: Option<&'static str>,
    pub tags: Vec<String>,
    pub starred: bool,
}

enum Hit {
    Repeat(&'static str, Option<NaiveDate>),
    Due(NaiveDate),
}

type RuleFn = fn(&Captures, NaiveDate) -> Option<Hit>;

const WD: &str = r"(sun(?:day)?|mon(?:day)?|tue(?:s|sday)?|wed(?:nesday)?|thu(?:r|rs|rsday)?|fri(?:day)?|sat(?:urday)?)";
const MON: &str = r"(jan(?:uary)?|feb(?:ruary)?|mar(?:ch)?|apr(?:il)?|may|june?|july?|aug(?:ust)?|sep(?:t|tember)?|oct(?:ober)?|nov(?:ember)?|dec(?:ember)?)";
const MONTHS: [&str; 12] = ["jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec"];

fn weekday(s: &str) -> Weekday {
    s[..3].to_lowercase().parse().unwrap_or(Weekday::Mon)
}

fn month(s: &str) -> u32 {
    let s = s[..3].to_lowercase();
    MONTHS.iter().position(|m| *m == s).unwrap_or(0) as u32
}

/// The next `wd` on or after `from`; one week later still when `skip_this_week`.
fn next_weekday(from: NaiveDate, wd: Weekday, skip_this_week: bool) -> NaiveDate {
    let mut d = from;
    while d.weekday() != wd {
        d += Duration::days(1);
    }
    if skip_this_week { d + Duration::days(7) } else { d }
}

/// `mon` is 0-based. Without a year, a date already past this year means next year.
fn month_day(today: NaiveDate, mon: u32, day: u32, year: Option<i32>) -> Option<Hit> {
    let d = NaiveDate::from_ymd_opt(year.unwrap_or(today.year()), mon + 1, day)?;
    if year.is_none() && d < today {
        return NaiveDate::from_ymd_opt(today.year() + 1, mon + 1, day).map(Hit::Due);
    }
    Some(Hit::Due(d))
}

static RULES: LazyLock<Vec<(Regex, RuleFn)>> = LazyLock::new(|| {
    let r = |s: &str| Regex::new(&format!("(?i){s}")).expect("quick-add rule");
    vec![
        (r(r"\bevery\s+weekday\b|\bweekdays\b"), (|_, _| Some(Hit::Repeat("weekdays", None))) as RuleFn),
        (r(r"\bevery\s+day\b|\bdaily\b"), |_, _| Some(Hit::Repeat("daily", None))),
        (r(&format!(r"\bevery\s+{WD}\b")), |m, t| Some(Hit::Repeat("weekly", Some(next_weekday(t, weekday(&m[1]), false))))),
        (r(r"\bevery\s+week\b|\bweekly\b"), |_, _| Some(Hit::Repeat("weekly", None))),
        (r(r"\bevery\s+month\b|\bmonthly\b"), |_, _| Some(Hit::Repeat("monthly", None))),
        (r(r"\bevery\s+year\b|\byearly\b|\bannually\b"), |_, _| Some(Hit::Repeat("yearly", None))),
        (r(r"\b(today|tonight|tod)\b"), |_, t| Some(Hit::Due(t))),
        (r(r"\b(tomorrow|tmrw?|tmr)\b"), |_, t| Some(Hit::Due(t + Duration::days(1)))),
        (r(r"\bnext\s+week\b"), |_, t| Some(Hit::Due(next_weekday(t + Duration::days(1), Weekday::Mon, false)))),
        (r(r"\b(this\s+)?weekend\b"), |_, t| Some(Hit::Due(next_weekday(t, Weekday::Sat, false)))),
        (r(r"\bin\s+(\d{1,3}|a|an|one|two|three)\s+(day|week|month)s?\b"), |m, t| {
            let n: u32 = match m[1].to_lowercase().as_str() {
                "a" | "an" | "one" => 1,
                "two" => 2,
                "three" => 3,
                s => s.parse().ok()?,
            };
            Some(Hit::Due(match m[2].to_lowercase().as_str() {
                "month" => t.checked_add_months(Months::new(n))?,
                "week" => t + Duration::days(7 * n as i64),
                _ => t + Duration::days(n as i64),
            }))
        }),
        (r(&format!(r"\b(next\s+)?{WD}\b")), |m, t| Some(Hit::Due(next_weekday(t, weekday(&m[2]), m.get(1).is_some())))),
        (r(&format!(r"\b{MON}\.?\s+(\d{{1,2}})(?:st|nd|rd|th)?(?:,?\s+(\d{{4}}))?\b")), |m, t| {
            month_day(t, month(&m[1]), m[2].parse().ok()?, m.get(3).and_then(|y| y.as_str().parse().ok()))
        }),
        (r(&format!(r"\b(\d{{1,2}})(?:st|nd|rd|th)?\s+{MON}\b")), |m, t| month_day(t, month(&m[2]), m[1].parse().ok()?, None)),
        (r(r"\b(\d{4})-(\d{2})-(\d{2})\b"), |m, _| {
            NaiveDate::from_ymd_opt(m[1].parse().ok()?, m[2].parse().ok()?, m[3].parse().ok()?).map(Hit::Due)
        }),
        (r(r"\b(\d{1,2})/(\d{1,2})(?:/(\d{2,4}))?\b"), |m, t| {
            let mon: u32 = m[1].parse().ok()?;
            let year = m.get(3).and_then(|y| {
                let y = y.as_str();
                if y.len() == 2 { format!("20{y}").parse().ok() } else { y.parse().ok() }
            });
            month_day(t, mon.checked_sub(1)?, m[2].parse().ok()?, year)
        }),
    ]
});

static TAG: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(^|\s)#([\p{L}\p{N}_-]+)").unwrap());
static STAR: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(^|\s)!{1,3}\s").unwrap());

pub fn parse(text: &str, today: NaiveDate) -> Parsed {
    let mut out = Parsed::default();
    let rest = format!(" {text} ");
    let mut rest = TAG
        .replace_all(&rest, |c: &Captures| {
            let tag = c[2].to_lowercase();
            if !out.tags.contains(&tag) {
                out.tags.push(tag);
            }
            c[1].to_string()
        })
        .into_owned();
    if let Some(m) = STAR.find(&rest) {
        out.starred = true;
        rest.replace_range(m.range(), " ");
    }
    for (re, f) in RULES.iter() {
        let Some(c) = re.captures(&rest) else { continue };
        let Some(hit) = f(&c, today) else { continue };
        let range = c.get(0).unwrap().range();
        match hit {
            Hit::Repeat(rule, due) => {
                if out.repeat.is_some() {
                    continue;
                }
                out.repeat = Some(rule);
                if out.due.is_none() {
                    out.due = due;
                }
            }
            Hit::Due(d) => {
                if out.due.is_some() {
                    continue;
                }
                out.due = Some(d);
            }
        }
        rest.replace_range(range, " ");
    }
    if out.repeat.is_some() && out.due.is_none() {
        out.due = Some(today);
    }
    out.title = rest.split_whitespace().collect::<Vec<_>>().join(" ");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    // A Tuesday.
    fn t() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 11, 17).unwrap()
    }
    fn d(m: u32, day: u32) -> Option<NaiveDate> {
        NaiveDate::from_ymd_opt(2026, m, day)
    }

    #[test]
    fn weekday_tag_star() {
        let p = parse("Quiz 4 fri #cs2110 !", t());
        assert_eq!(p, Parsed { title: "Quiz 4".into(), due: d(11, 20), repeat: None, tags: vec!["cs2110".into()], starred: true });
    }

    #[test]
    fn relative_dates() {
        assert_eq!(parse("call mom tomorrow", t()).due, d(11, 18));
        assert_eq!(parse("pay rent in 2 weeks", t()).due, d(12, 1));
        assert_eq!(parse("plan trip next week", t()).due, d(11, 23));
        assert_eq!(parse("next tue standup", t()).due, d(11, 24));
        assert_eq!(parse("today thing", t()).due, d(11, 17));
    }

    #[test]
    fn absolute_dates_roll_to_next_year() {
        assert_eq!(parse("renew passport jan 5", t()).due, NaiveDate::from_ymd_opt(2027, 1, 5));
        assert_eq!(parse("party dec 3rd", t()).due, d(12, 3));
        assert_eq!(parse("taxes 4/15", t()).due, NaiveDate::from_ymd_opt(2027, 4, 15));
        assert_eq!(parse("x 2026-12-25", t()).due, d(12, 25));
        assert_eq!(parse("feb 30 is not a day", t()).due, None);
    }

    #[test]
    fn repeats_anchor_due() {
        let p = parse("water plants every sat", t());
        assert_eq!((p.title.as_str(), p.repeat, p.due), ("water plants", Some("weekly"), d(11, 21)));
        let p = parse("stretch daily", t());
        assert_eq!((p.repeat, p.due), (Some("daily"), d(11, 17)));
    }

    #[test]
    fn leaves_plain_words_alone() {
        let p = parse("Read Monday Night Football recap", t());
        // The weekday is still taken (same as the web composer); the rest stays the title.
        assert_eq!(p.title, "Read Night Football recap");
        assert_eq!(parse("buy milk", t()), Parsed { title: "buy milk".into(), ..Default::default() });
        assert!(!parse("Wow! nice", t()).starred);
    }
}
