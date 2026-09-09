//! Calendar candidates inspired by Mozc date_rewriter (see docs/reconversion-date.md).
//! Pure calendar logic is separated from the local clock for deterministic tests.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DateTime {
    pub year: i32,
    pub month: u32,
    pub day: u32,
    pub hour: u32,
    pub minute: u32,
    pub second: u32,
}

impl DateTime {
    fn shift_days(mut self, offset: i32) -> Self {
        for _ in 0..offset.unsigned_abs() {
            if offset > 0 {
                self.day += 1;
                if self.day > month_days(self.year, self.month) {
                    self.day = 1;
                    self.month += 1;
                    if self.month == 13 {
                        self.month = 1;
                        self.year += 1;
                    }
                }
            } else if self.day > 1 {
                self.day -= 1;
            } else {
                if self.month == 1 {
                    self.month = 12;
                    self.year -= 1;
                } else {
                    self.month -= 1;
                }
                self.day = month_days(self.year, self.month);
            }
        }
        self
    }
    fn weekday(self) -> usize {
        // Sakamoto's Gregorian calendar algorithm; Sunday = 0.
        let offsets = [0, 3, 2, 5, 0, 3, 5, 1, 4, 6, 2, 4];
        let y = self.year - i32::from(self.month < 3);
        (y + y / 4 - y / 100 + y / 400 + offsets[self.month as usize - 1] + self.day as i32)
            .rem_euclid(7) as usize
    }
}
fn month_days(y: i32, m: u32) -> u32 {
    match m {
        4 | 6 | 9 | 11 => 30,
        2 => {
            if y % 4 == 0 && (y % 100 != 0 || y % 400 == 0) {
                29
            } else {
                28
            }
        }
        _ => 31,
    }
}

#[cfg(windows)]
fn local_now() -> DateTime {
    #[repr(C)]
    #[derive(Default)]
    struct SystemTime {
        year: u16,
        month: u16,
        weekday: u16,
        day: u16,
        hour: u16,
        minute: u16,
        second: u16,
        millis: u16,
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetLocalTime(time: *mut SystemTime);
    }
    let mut t = SystemTime::default();
    unsafe {
        GetLocalTime(&mut t);
    }
    DateTime {
        year: t.year.into(),
        month: t.month.into(),
        day: t.day.into(),
        hour: t.hour.into(),
        minute: t.minute.into(),
        second: t.second.into(),
    }
}

pub fn candidates(reading: &str) -> Vec<String> {
    #[cfg(windows)]
    {
        candidates_at(reading, local_now())
    }
    #[cfg(not(windows))]
    {
        let _ = reading;
        Vec::new()
    }
}

fn era_date(d: DateTime) -> Option<String> {
    for (start, name) in [
        ((2019, 5, 1), "令和"),
        ((1989, 1, 8), "平成"),
        ((1926, 12, 25), "昭和"),
        ((1912, 7, 30), "大正"),
        ((1868, 1, 25), "明治"),
    ] {
        if (d.year, d.month, d.day) >= start {
            let n = d.year - start.0 + 1;
            return Some(format!(
                "{}{}年{}月{}日",
                name,
                if n == 1 { "元".into() } else { n.to_string() },
                d.month,
                d.day
            ));
        }
    }
    None
}
fn date_formats(d: DateTime) -> Vec<String> {
    let mut out = vec![
        format!("{:04}/{:02}/{:02}", d.year, d.month, d.day),
        format!("{:04}-{:02}-{:02}", d.year, d.month, d.day),
        format!("{}年{}月{}日", d.year, d.month, d.day),
    ];
    if let Some(era) = era_date(d) {
        out.push(era);
    }
    out.push(format!(
        "{}曜日",
        ["日", "月", "火", "水", "木", "金", "土"][d.weekday()]
    ));
    out
}

pub fn candidates_at(reading: &str, now: DateTime) -> Vec<String> {
    let days = match reading {
        "きょう" => Some(0),
        "あした" | "あす" => Some(1),
        "きのう" | "さくじつ" => Some(-1),
        "おととい" | "おとつい" | "いっさくじつ" => Some(-2),
        "さきおととい" => Some(-3),
        "あさって" | "みょうごにち" => Some(2),
        "しあさって" => Some(3),
        _ => None,
    };
    if let Some(days) = days {
        return date_formats(now.shift_days(days));
    }
    for (weekday, names) in [
        (0, ["にちよう", "にちようび"]),
        (1, ["げつよう", "げつようび"]),
        (2, ["かよう", "かようび"]),
        (3, ["すいよう", "すいようび"]),
        (4, ["もくよう", "もくようび"]),
        (5, ["きんよう", "きんようび"]),
        (6, ["どよう", "どようび"]),
    ] {
        if names.contains(&reading) {
            return date_formats(now.shift_days((weekday - now.weekday() as i32).rem_euclid(7)));
        }
    }
    let years = match reading {
        "ことし" => Some(0),
        "らいねん" => Some(1),
        "さくねん" | "きょねん" => Some(-1),
        "おととし" => Some(-2),
        "さらいねん" => Some(2),
        _ => None,
    };
    if let Some(n) = years {
        let y = now.year + n;
        let mut out = vec![format!("{y}年")];
        // A transition year can contain two eras; include both instead of guessing.
        for (start, end, name) in [
            (2019, 9999, "令和"),
            (1989, 2019, "平成"),
            (1926, 1989, "昭和"),
            (1912, 1926, "大正"),
            (1868, 1912, "明治"),
        ] {
            if y >= start && y <= end {
                let n = y - start + 1;
                out.push(format!(
                    "{}{}年",
                    name,
                    if n == 1 { "元".into() } else { n.to_string() }
                ));
            }
        }
        return out;
    }
    let months = match reading {
        "こんげつ" => Some(0),
        "らいげつ" => Some(1),
        "せんげつ" => Some(-1),
        "せんせんげつ" => Some(-2),
        "さらいげつ" => Some(2),
        _ => None,
    };
    if let Some(n) = months {
        let m = now.year * 12 + now.month as i32 - 1 + n;
        return vec![
            format!("{}月", m.rem_euclid(12) + 1),
            format!("{}年{}月", m.div_euclid(12), m.rem_euclid(12) + 1),
        ];
    }
    match reading {
        "いま" | "じこく" => vec![
            format!("{:02}:{:02}", now.hour, now.minute),
            format!("{}時{}分", now.hour, now.minute),
            format!("{:02}:{:02}:{:02}", now.hour, now.minute, now.second),
        ],
        "にちじ" | "なう" => vec![
            format!(
                "{:04}/{:02}/{:02} {:02}:{:02}",
                now.year, now.month, now.day, now.hour, now.minute
            ),
            format!(
                "{}年{}月{}日 {}時{}分",
                now.year, now.month, now.day, now.hour, now.minute
            ),
        ],
        _ => Vec::new(),
    }
}

/// Dynamic candidates must not become stale learned dates on the next day.
pub fn is_dynamic(reading: &str, surface: &str) -> bool {
    let sample = DateTime {
        year: 2026,
        month: 9,
        day: 9,
        hour: 12,
        minute: 34,
        second: 56,
    };
    !candidates_at(reading, sample).is_empty()
        && (surface.chars().any(|c| c.is_ascii_digit())
            || surface.ends_with("曜日")
            || surface.contains("元年"))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn at(y: i32, m: u32, d: u32) -> DateTime {
        DateTime {
            year: y,
            month: m,
            day: d,
            hour: 0,
            minute: 5,
            second: 9,
        }
    }
    #[test]
    fn leap_days_and_year_rollover() {
        assert_eq!(candidates_at("あした", at(2024, 2, 28))[0], "2024/02/29");
        assert_eq!(candidates_at("あした", at(2100, 2, 28))[0], "2100/03/01");
        assert_eq!(candidates_at("きのう", at(2026, 1, 1))[0], "2025/12/31");
        assert_eq!(candidates_at("らいげつ", at(2026, 12, 31))[1], "2027年1月");
    }
    #[test]
    fn era_boundary_and_weekdays() {
        assert!(candidates_at("きょう", at(2019, 4, 30)).contains(&"平成31年4月30日".into()));
        assert!(candidates_at("きょう", at(2019, 5, 1)).contains(&"令和元年5月1日".into()));
        assert_eq!(
            candidates_at("きょう", at(2026, 9, 9)).last().unwrap(),
            "水曜日"
        );
        assert_eq!(candidates_at("げつようび", at(2026, 9, 9))[0], "2026/09/14");
    }
    #[test]
    fn time_and_exact_match_only() {
        assert_eq!(candidates_at("いま", at(2026, 9, 9))[0], "00:05");
        assert!(candidates_at("きょうと", at(2026, 9, 9)).is_empty());
        assert!(is_dynamic("きょう", "2025/01/01"));
        assert!(!is_dynamic("きょう", "今日"));
    }
}
