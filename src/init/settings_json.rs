use serde::{
    de::{Deserialize, Deserializer, Error, MapAccess, SeqAccess, Visitor},
    ser::{Serialize, SerializeMap, Serializer},
};
use serde_json::{Number, Value};
use std::{collections::HashSet, fmt, iter};

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

    pub(super) fn remove(&mut self, key: &str) -> Option<Json> {
        let index = self.0.iter().position(|(name, _)| name == key)?;
        Some(self.0.remove(index).1)
    }
}

impl Json {
    pub(super) fn parse(bytes: &[u8]) -> Result<Self, serde_json::Error> {
        let mut doc: Self = serde_json::from_slice(bytes)?;
        let mut tokens = number_tokens(bytes);
        doc.restore_numbers(&mut tokens);
        Ok(doc)
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

impl Serialize for Json {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Scalar(value) => value.serialize(serializer),
            // The JSON parser validated these tokens before their original spellings were restored.
            Self::Number(token) => {
                Number::from_string_unchecked(token.clone()).serialize(serializer)
            }
            Self::Array(items) => items.serialize(serializer),
            Self::Object(object) => {
                let mut map = serializer.serialize_map(Some(object.0.len()))?;
                for (key, value) in &object.0 {
                    map.serialize_entry(key, value)?;
                }
                map.end()
            }
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
        while let Some(key) = map.next_key::<Key>()? {
            let Key::Object(key) = key else {
                return map.next_value().map(Json::Number);
            };
            if !keys.insert(key.clone()) {
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

    fn visit_i64<E>(self, value: i64) -> Result<Json, E> {
        Ok(Json::Number(value.to_string()))
    }

    fn visit_u64<E>(self, value: u64) -> Result<Json, E> {
        Ok(Json::Number(value.to_string()))
    }
}

enum Key {
    Object(String),
    Number,
}

impl<'de> Deserialize<'de> for Key {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        // JSON object keys dispatch to visit_some; an arbitrary-precision number supplies a synthetic string key.
        deserializer.deserialize_option(KeyVisitor)
    }
}

struct KeyVisitor;
impl<'de> Visitor<'de> for KeyVisitor {
    type Value = Key;
    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a JSON object key")
    }
    fn visit_some<D: Deserializer<'de>>(self, d: D) -> Result<Key, D::Error> {
        let key = String::deserialize(d)?;
        if key.starts_with("$serde_json::private::") {
            return Err(serde::de::Error::custom(super::INVALID_HOOKS));
        }
        Ok(Key::Object(key))
    }
    fn visit_borrowed_str<E: serde::de::Error>(self, _: &'de str) -> Result<Key, E> {
        Ok(Key::Number)
    }
}

// The parser normalizes exponent spellings, so recover number tokens from the already validated input.
fn number_tokens(input: &[u8]) -> impl Iterator<Item = &[u8]> {
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
                    return Some(&input[start..i]);
                }
                _ => {}
            }
        }
        None
    })
}
