use crate::{
    engine,
    ledger::{self, Counts, Key},
};
use std::{
    collections::BTreeMap,
    fmt, io,
    str::FromStr,
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct Date {
    year: i32,
    month: u32,
    day: u32,
}

impl Date {
    #[must_use]
    pub fn day_number(self) -> i64 {
        let year = i64::from(self.year) - i64::from(self.month <= 2);
        let month = (i64::from(self.month) + 9) % 12;
        365 * year + year.div_euclid(4) - year.div_euclid(100)
            + year.div_euclid(400)
            + (153 * month + 2) / 5
            + i64::from(self.day)
            - 1
    }

    /// The date that many days after 1970-01-01.
    #[must_use]
    pub fn from_unix_days(days: u32) -> Option<Self> {
        let first = |year, month| Self {
            year,
            month,
            day: 1,
        };
        let target = first(1970, 1).day_number() + i64::from(days);
        // No year is shorter than 365 days, so the date cannot fall after this one.
        let latest = 1970 + i32::try_from(days / 365).ok()?;
        let start = (1970..=latest)
            .rev()
            .flat_map(|year| (1..=12).rev().map(move |month| first(year, month)))
            .find(|start| start.day_number() <= target)?;
        Some(Self {
            day: u32::try_from(target - start.day_number() + 1).ok()?,
            ..start
        })
    }

    pub(crate) fn today() -> io::Result<Self> {
        let seconds = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(io::Error::other)?
            .as_secs();
        u32::try_from(seconds / 86400)
            .ok()
            .and_then(Self::from_unix_days)
            .ok_or_else(|| io::Error::other("invalid system date"))
    }
}

impl fmt::Display for Date {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:04}-{:02}-{:02}", self.year, self.month, self.day)
    }
}

impl FromStr for Date {
    type Err = io::Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let invalid = || io::Error::other("invalid date");
        if s.len() != 10
            || !s.bytes().enumerate().all(|(i, b)| {
                if i == 4 || i == 7 {
                    b == b'-'
                } else {
                    b.is_ascii_digit()
                }
            })
        {
            return Err(invalid());
        }
        let year = s[..4].parse().map_err(|_| invalid())?;
        let month = s[5..7].parse().map_err(|_| invalid())?;
        let day = s[8..].parse().map_err(|_| invalid())?;
        let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
        let limit = match month {
            2 => 28 + u32::from(leap),
            4 | 6 | 9 | 11 => 30,
            1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
            _ => return Err(invalid()),
        };
        if year == 0 || !(1..=limit).contains(&day) {
            return Err(invalid());
        }
        Ok(Self { year, month, day })
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct Objection {
    pub extra: usize,
    pub filed: Date,
    pub renewal: Option<Date>,
    pub reason: String,
}

impl Objection {
    #[must_use]
    pub fn live(&self, today: Date) -> bool {
        (0..=30).contains(&(today.day_number() - self.renewal.unwrap_or(self.filed).day_number()))
    }

    // The one spelling of a row, whoever writes it.
    pub(crate) fn line(&self, key: &Key) -> String {
        let renewal = self
            .renewal
            .map_or_else(|| "-".to_owned(), |date| date.to_string());
        format!(
            "{}\t{}\t{}\t{}\t{renewal}\t{}\n",
            key.0, key.1, self.extra, self.filed, self.reason
        )
    }
}

pub type Rows = BTreeMap<Key, Objection>;

#[derive(Debug)]
pub(crate) struct RowError {
    pub(crate) key: Option<Key>,
    line: usize,
    reason: String,
}

impl fmt::Display for RowError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "objections line {}: {}", self.line, self.reason)
    }
}

impl std::error::Error for RowError {}

/// # Errors
/// Reports the first invalid or duplicated row.
pub fn parse(bytes: &[u8], today: Date) -> io::Result<Rows> {
    let mut rows = Rows::new();
    for (i, raw) in bytes.split_inclusive(|&b| b == b'\n').enumerate() {
        let err = |why: &str| {
            let key = std::str::from_utf8(raw).ok().and_then(|line| {
                let mut fields = line.split('\t');
                Some((
                    fields.next()?.escape_debug().to_string(),
                    fields.next()?.escape_debug().to_string(),
                ))
            });
            io::Error::other(RowError {
                key,
                line: i + 1,
                reason: why.into(),
            })
        };
        let raw = raw
            .strip_suffix(b"\n")
            .ok_or_else(|| err("missing newline"))?;
        let line = std::str::from_utf8(raw).map_err(|_| err("invalid UTF-8"))?;
        let fields: Vec<_> = line.split('\t').collect();
        let &[rule, path, count, filed, renewal, reason] = fields.as_slice() else {
            return Err(err("expected six fields"));
        };
        if ledger::invalid_path(path) || !engine::can_allow(engine::registry(), rule) {
            return Err(err("invalid rule or path"));
        }
        let extra = ledger::count_of(count).ok_or_else(|| err("invalid count"))?;
        let filed = filed.parse().map_err(|_| err("invalid filed date"))?;
        let renewal = (renewal != "-")
            .then(|| renewal.parse())
            .transpose()
            .map_err(|_| err("invalid renewal date"))?;
        if filed > today || renewal.is_some_and(|r| r < filed || r > today) {
            return Err(err("date outside permitted range"));
        }
        if reason.trim().is_empty() || reason.contains('\r') {
            return Err(err("invalid reason"));
        }
        if rows
            .insert(
                (rule.into(), path.into()),
                Objection {
                    extra,
                    filed,
                    renewal,
                    reason: reason.into(),
                },
            )
            .is_some()
        {
            return Err(err("duplicate objection"));
        }
    }
    Ok(rows)
}

pub(crate) fn allowances(base: &Counts, rows: &Rows, today: Date) -> Counts {
    let mut allowed = base.clone();
    for (key, row) in rows {
        if row.live(today) {
            let count = allowed.entry(key.clone()).or_default();
            *count = count.saturating_add(row.extra);
        }
    }
    allowed
}
