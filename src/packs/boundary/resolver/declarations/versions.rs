use super::{Charge, Manifest};
use std::cmp::Ordering;

const TYPESCRIPT: [u64; 3] = [5, 9, 3];

pub(super) fn paths<'a>(
    manifest: &'a Manifest,
    charge: &Charge<'_>,
) -> Result<Option<&'a Manifest>, String> {
    let Some(versions) = manifest
        .get("typesVersions")
        .and_then(serde_json::Value::as_object)
    else {
        return Ok(None);
    };
    let mut selected = None;
    let mut rank = u32::MAX;
    for (range, target) in versions {
        charge()?;
        if matches(range) {
            let index = array_index(range).unwrap_or(u32::MAX);
            if selected.is_none() || index < rank {
                selected = Some(target);
                rank = index;
            }
        }
    }
    Ok(selected.and_then(serde_json::Value::as_object))
}

fn array_index(key: &str) -> Option<u32> {
    let index: u32 = key.parse().ok()?;
    (index != u32::MAX && index.to_string() == key).then_some(index)
}

pub(super) fn matches(range: &str) -> bool {
    let mut matched = range.trim().is_empty();
    for alternative in range.trim().split("||").filter(|part| !part.is_empty()) {
        let words = alternative.split_whitespace();
        let result = if let [lower, "-", upper] = words.clone().collect::<Vec<_>>().as_slice() {
            compare(">=", lower)
                .zip(compare("<=", upper))
                .map(|(low, high)| low && high)
        } else {
            let mut all_comparators = true;
            for word in words {
                let end = word
                    .find(|c: char| !matches!(c, '<' | '>' | '=' | '^' | '~'))
                    .unwrap_or(word.len());
                let (operator, version) = word.split_at(end);
                let Some(result) = compare(operator, version) else {
                    return false;
                };
                all_comparators &= result;
            }
            Some(all_comparators && !alternative.trim().is_empty())
        };
        let Some(result) = result else {
            return false;
        };
        matched |= result;
    }
    matched
}

struct Partial {
    number: [u64; 3],
    specified: usize,
    prerelease: bool,
}

impl Partial {
    fn parse(text: &str) -> Option<Self> {
        let (text, build) = text
            .split_once('+')
            .map_or((text, None), |(v, b)| (v, Some(b)));
        let (text, prerelease) = text
            .split_once('-')
            .map_or((text, None), |(v, p)| (v, Some(p)));
        let mut version = Self {
            number: [0; 3],
            specified: 0,
            prerelease: prerelease.is_some(),
        };
        for (index, part) in text.split('.').enumerate() {
            let target = version.number.get_mut(index)?;
            if matches!(part, "*" | "x" | "X") {
                continue;
            }
            if part.is_empty()
                || !part.bytes().all(|c| c.is_ascii_digit())
                || (part.len() > 1 && part.starts_with('0'))
            {
                return None;
            }
            let number = part.parse().ok()?;
            if version.specified == index {
                *target = number;
                version.specified += 1;
            }
        }
        if (prerelease.is_some() || build.is_some()) && text.split('.').count() != 3
            || prerelease.is_some_and(|value| !identifiers(value, true))
            || build.is_some_and(|value| !identifiers(value, false))
        {
            return None;
        }
        Some(version)
    }

    fn upper(&self, digits: usize) -> bool {
        TYPESCRIPT.get(..digits) <= self.number.get(..digits)
    }
}

fn identifiers(text: &str, prerelease: bool) -> bool {
    text.split('.').all(|part| {
        let mut bytes = part.bytes();
        let Some(first) = bytes.next() else {
            return false;
        };
        part.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-')
            && (!prerelease
                || !first.is_ascii_digit()
                || (bytes.all(|c| c.is_ascii_digit()) && (first != b'0' || part.len() == 1)))
    })
}

fn compare(operator: &str, text: &str) -> Option<bool> {
    let version = Partial::parse(text)?;
    if version.specified == 0 {
        return match operator {
            "<" | ">" => Some(false),
            "" | "=" | ">=" | "<=" | "~" | "^" => Some(true),
            _ => None,
        };
    }
    let order = TYPESCRIPT.cmp(&version.number).then(if version.prerelease {
        Ordering::Greater
    } else {
        Ordering::Equal
    });
    let partial = version.specified < 3;
    Some(match operator {
        "" | "=" if partial => order.is_ge() && version.upper(version.specified),
        "" | "=" => order.is_eq(),
        "<" => order.is_lt(),
        ">=" => order.is_ge(),
        "<=" if partial => version.upper(version.specified),
        ">" if partial => !version.upper(version.specified),
        "<=" => order.is_le(),
        ">" => order.is_gt(),
        "~" => order.is_ge() && version.upper(version.specified.min(2)),
        "^" => {
            let digits = match version.number {
                [0, 0, _] => version.specified,
                [0, _, _] => version.specified.min(2),
                _ => 1,
            };
            order.is_ge() && version.upper(digits)
        }
        _ => return None,
    })
}
