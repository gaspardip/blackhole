use serde::de::{Deserialize, Deserializer, Error, IgnoredAny, MapAccess, SeqAccess, Visitor};
use serde_json::Value;
use std::{collections::HashSet, fmt, iter, ops::Range};

pub(super) enum Json {
    Object(Object),
    Array(Vec<Self>),
    Scalar(Value),
    Number(String),
}

#[derive(Default)]
pub(super) struct Object(Vec<(String, Json)>);

impl Object {
    pub(super) fn get(&self, key: &str) -> Option<&Json> {
        self.0.iter().find(|(name, _)| name == key).map(|(_, v)| v)
    }

    pub(super) fn get_mut(&mut self, key: &str) -> Option<&mut Json> {
        self.0
            .iter_mut()
            .find(|(name, _)| name == key)
            .map(|(_, v)| v)
    }

    pub(super) fn entries(&self) -> impl Iterator<Item = (&str, &Json)> {
        self.0.iter().map(|(key, value)| (key.as_str(), value))
    }

    pub(super) fn entry(&mut self, key: &str, default: impl FnOnce() -> Json) -> &mut Json {
        let index = self.0.iter().position(|(name, _)| name == key);
        let index = index.unwrap_or_else(|| {
            self.0.push((key.to_owned(), default()));
            self.0.len() - 1
        });
        &mut self.0[index].1
    }
}

impl Json {
    pub(super) fn parse(bytes: &[u8]) -> Result<Self, serde_json::Error> {
        // Skipping values checks number syntax without evaluating it, so 1e400 stays valid.
        serde_json::from_slice::<IgnoredAny>(bytes)?;
        let ranges: Vec<_> = number_tokens(bytes).collect();
        // Same-length placeholders keep error positions; the original tokens are restored below.
        let mut masked = bytes.to_vec();
        for range in &ranges {
            masked[range.clone()].fill(b' ');
            masked[range.start] = b'0';
        }
        let mut doc: Self = serde_json::from_slice(&masked)?;
        doc.restore_numbers(&mut ranges.into_iter().map(|range| &bytes[range]));
        Ok(doc)
    }

    // serde_json writes number tokens verbatim only with arbitrary_precision, which changes every Value parse.
    pub(super) fn to_vec_pretty(&self) -> serde_json::Result<Vec<u8>> {
        let mut out = Vec::new();
        self.write_pretty(&mut out, 0)?;
        Ok(out)
    }

    fn write_pretty(&self, out: &mut Vec<u8>, depth: usize) -> serde_json::Result<()> {
        let (open, close, items): (u8, u8, Vec<_>) = match self {
            Self::Scalar(value) => return serde_json::to_writer(out, value),
            Self::Number(token) => {
                out.extend_from_slice(token.as_bytes());
                return Ok(());
            }
            Self::Array(items) => (b'[', b']', items.iter().map(|v| (None, v)).collect()),
            Self::Object(Object(entries)) => (
                b'{',
                b'}',
                entries.iter().map(|(k, v)| (Some(k), v)).collect(),
            ),
        };
        out.push(open);
        for (i, (key, value)) in items.iter().enumerate() {
            out.extend_from_slice(if i == 0 { b"\n" } else { b",\n" });
            out.resize(out.len() + 2 * (depth + 1), b' ');
            if let Some(key) = key {
                serde_json::to_writer(&mut *out, key)?;
                out.extend_from_slice(b": ");
            }
            value.write_pretty(out, depth + 1)?;
        }
        if !items.is_empty() {
            out.push(b'\n');
            out.resize(out.len() + 2 * depth, b' ');
        }
        out.push(close);
        Ok(())
    }

    fn restore_numbers<'a>(&mut self, tokens: &mut impl Iterator<Item = &'a [u8]>) {
        match self {
            Self::Object(Object(entries)) => {
                for (_, value) in entries {
                    value.restore_numbers(tokens);
                }
            }
            Self::Array(items) => {
                for value in items {
                    value.restore_numbers(tokens);
                }
            }
            Self::Number(token) => {
                *token = String::from_utf8_lossy(tokens.next().expect("validated JSON number"))
                    .into_owned();
            }
            Self::Scalar(_) => {}
        }
    }

    pub(super) fn is_null(&self) -> bool {
        matches!(self, Self::Scalar(Value::Null))
    }

    pub(super) fn is_bool(&self) -> bool {
        matches!(self, Self::Scalar(Value::Bool(_)))
    }

    pub(super) fn as_u64(&self) -> Option<u64> {
        match self {
            Self::Number(token) if token != "-0" => token.parse().ok(),
            _ => None,
        }
    }

    pub(super) fn is_toml_value(&self) -> bool {
        match self {
            Self::Array(values) => values.iter().all(Self::is_toml_value),
            Self::Object(values) => values.entries().all(|(_, value)| value.is_toml_value()),
            _ => !self.is_null() && self.as_u64().is_none_or(|n| i64::try_from(n).is_ok()),
        }
    }

    pub(super) fn object(entries: impl IntoIterator<Item = (&'static str, Self)>) -> Self {
        Self::Object(Object(
            entries
                .into_iter()
                .map(|(k, v)| (k.to_owned(), v))
                .collect(),
        ))
    }

    pub(super) fn string(text: &str) -> Self {
        Self::Scalar(Value::String(text.to_owned()))
    }

    pub(super) fn as_str(&self) -> Option<&str> {
        match self {
            Self::Scalar(value) => value.as_str(),
            _ => None,
        }
    }

    pub(super) fn as_object(&self) -> Option<&Object> {
        match self {
            Self::Object(value) => Some(value),
            _ => None,
        }
    }

    pub(super) fn as_object_mut(&mut self) -> Option<&mut Object> {
        match self {
            Self::Object(value) => Some(value),
            _ => None,
        }
    }

    pub(super) fn as_array(&self) -> Option<&[Self]> {
        match self {
            Self::Array(value) => Some(value),
            _ => None,
        }
    }

    pub(super) fn as_array_mut(&mut self) -> Option<&mut Vec<Self>> {
        match self {
            Self::Array(value) => Some(value),
            _ => None,
        }
    }
}

impl<'de> Deserialize<'de> for Json {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_any(JsonVisitor)
    }
}

struct JsonVisitor;

impl<'de> Visitor<'de> for JsonVisitor {
    type Value = Json;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("JSON with unique object keys")
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Json, A::Error> {
        let mut keys = HashSet::new();
        let mut entries = Vec::new();
        while let Some(key) = map.next_key::<String>()? {
            if key.starts_with("$serde_json::private::") || !keys.insert(key.clone()) {
                return Err(A::Error::custom(super::INVALID_HOOKS));
            }
            entries.push((key, map.next_value()?));
        }
        Ok(Json::Object(Object(entries)))
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Json, A::Error> {
        let mut items = Vec::new();
        while let Some(item) = seq.next_element()? {
            items.push(item);
        }
        Ok(Json::Array(items))
    }

    fn visit_unit<E>(self) -> Result<Json, E> {
        Ok(Json::Scalar(Value::Null))
    }

    fn visit_bool<E>(self, value: bool) -> Result<Json, E> {
        Ok(Json::Scalar(Value::Bool(value)))
    }

    fn visit_str<E>(self, value: &str) -> Result<Json, E> {
        Ok(Json::string(value))
    }

    fn visit_u64<E>(self, _: u64) -> Result<Json, E> {
        Ok(Json::Number(String::new()))
    }
}

fn number_tokens(input: &[u8]) -> impl Iterator<Item = Range<usize>> {
    let mut i = 0;
    iter::from_fn(move || {
        while let Some(&byte) = input.get(i) {
            let start = i;
            i += 1;
            match byte {
                b'"' => {
                    while input[i] != b'"' {
                        i += if input[i] == b'\\' { 2 } else { 1 };
                    }
                    i += 1;
                }
                b'-' | b'0'..=b'9' => {
                    while matches!(
                        input.get(i),
                        Some(b'0'..=b'9' | b'.' | b'e' | b'E' | b'+' | b'-')
                    ) {
                        i += 1;
                    }
                    return Some(start..i);
                }
                _ => {}
            }
        }
        None
    })
}
