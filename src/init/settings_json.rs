use serde::{
    de::{Deserialize, Deserializer, Error, MapAccess, SeqAccess, Visitor},
    ser::{Serialize, SerializeMap, Serializer},
};
use serde_json::Value;
use std::{collections::HashSet, fmt};

pub(super) enum Json {
    Object(Object),
    Array(Vec<Self>),
    Scalar(Value),
}

#[derive(Default)]
pub(super) struct Object(Vec<(String, Json)>);

impl Object {
    pub(super) fn get(&self, key: &str) -> Option<&Json> {
        self.0.iter().find(|(name, _)| name == key).map(|(_, v)| v)
    }

    pub(super) fn values(&self) -> impl Iterator<Item = &Json> {
        self.0.iter().map(|(_, v)| v)
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
                let token: String = map.next_value()?;
                let number = token
                    .parse::<serde_json::Number>()
                    .map_err(A::Error::custom)?;
                return Ok(Json::Scalar(Value::Number(number)));
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
        Ok(Json::Scalar(Value::from(value)))
    }

    fn visit_u64<E>(self, value: u64) -> Result<Json, E> {
        Ok(Json::Scalar(Value::from(value)))
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
