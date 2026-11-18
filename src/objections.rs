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

    pub(crate) fn today() -> io::Result<Self> {
        let days = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(io::Error::other)?
            .as_secs()
            / 86400;
        let epoch = Self {
            year: 1970,
            month: 1,
            day: 1,
        }
        .day_number();
        let target = epoch + i64::try_from(days).map_err(io::Error::other)?;
        let mut year = 1970 + i32::try_from(days / 365).map_err(io::Error::other)?;
        while (Self {
            year,
            month: 1,
            day: 1,
        })
        .day_number()
            > target
        {
            year -= 1;
        }
        for month in (1..=12).rev() {
            let first = Self {
                year,
                month,
                day: 1,
            };
            if first.day_number() <= target {
                return Ok(Self {
                    day: u32::try_from(target - first.day_number() + 1)
                        .map_err(io::Error::other)?,
                    ..first
                });
            }
        }
        Err(io::Error::other("invalid system date"))
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
        let extra = count.parse().map_err(|_| err("invalid count"))?;
        if count.starts_with('0') || !count.bytes().all(|b| b.is_ascii_digit()) {
            return Err(err("invalid count"));
        }
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
