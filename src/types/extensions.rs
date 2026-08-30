//! `Extensions` — the map that keeps JSON fields this crate has never heard of.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::validate::{Validate, Validator};

/// Undocumented JSON fields found on an OCPI object, preserved verbatim.
///
/// OCPI 2.3.0 is explicit:
///
/// > *An OCPI Platform SHALL NOT reject request or response payloads based on the presence of
/// > JSON object field names that are not documented in this specification.*
/// >
/// > *OCPI implementers are encouraged to extend OCPI with new fields to address needs that are
/// > not foreseen by the specification.*
///
/// Every OCPI object in this crate carries an `extensions` field marked `#[serde(flatten)]`, so
/// a vendor field arrives, survives, and is written back out unchanged. A hub built on this
/// crate can therefore sit between two parties that have agreed on an extension without knowing
/// anything about it — which is the whole point of that paragraph, and the thing generated type
/// sets get wrong.
///
/// Keys are kept in a [`BTreeMap`], so serialisation order is deterministic.
///
/// Every wire object in this crate carries one as a `#[serde(flatten)]` field, so the undocumented
/// members of the object it came from land here and are written straight back:
///
/// ```
/// use ocpi_kit::types::Extensions;
///
/// let json = r#"{"acme_note":"kerbside","nltnm_accuracy_m":3}"#;
/// let extensions: Extensions = serde_json::from_str(json).unwrap();
///
/// assert_eq!(extensions.get::<u32>("nltnm_accuracy_m").unwrap(), Some(3));
/// assert_eq!(serde_json::to_string(&extensions).unwrap(), json);
/// ```
///
/// In place, on a real object:
///
#[cfg_attr(feature = "v2_3_0", doc = "```rust")]
#[cfg_attr(not(feature = "v2_3_0"), doc = "```rust,ignore")]
/// # use ocpi_kit::v2_3_0::locations::GeoLocation;
/// let json = r#"{"latitude":"52.010","longitude":"4.350","nltnm_accuracy_m":3}"#;
/// let geo: GeoLocation = serde_json::from_str(json).unwrap();
/// assert_eq!(geo.extensions.get::<u32>("nltnm_accuracy_m").unwrap(), Some(3));
/// assert_eq!(serde_json::to_string(&geo).unwrap(), json);
/// ```
///
/// Spec: 2.3.0 §transport_and_format — Non-specified JSON fields
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Extensions(BTreeMap<String, serde_json::Value>);

impl Extensions {
    /// An empty set of extensions.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether no undocumented field was present.
    ///
    /// Objects skip serialising their `extensions` field when this is true, so an object that
    /// carried no extensions is written back byte-identically.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// How many undocumented fields are present.
    #[must_use]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// The raw JSON value stored under `key`, if any.
    #[must_use]
    pub fn get_raw(&self, key: &str) -> Option<&serde_json::Value> {
        self.0.get(key)
    }

    /// Deserialises the value stored under `key` into `T`.
    ///
    /// Returns `Ok(None)` when the key is absent, and `Err` when it is present but does not
    /// deserialise into `T`.
    ///
    /// # Errors
    ///
    /// Propagates the `serde_json` error describing why the value did not fit `T`.
    pub fn get<T: serde::de::DeserializeOwned>(&self, key: &str) -> Result<Option<T>, serde_json::Error> {
        self.0.get(key).cloned().map(serde_json::from_value).transpose()
    }

    /// Stores `value` under `key`, replacing any previous value.
    ///
    /// # Errors
    ///
    /// Propagates the `serde_json` error if `value` cannot be serialised.
    pub fn insert<T: Serialize>(
        &mut self,
        key: impl Into<String>,
        value: T,
    ) -> Result<(), serde_json::Error> {
        self.0.insert(key.into(), serde_json::to_value(value)?);
        Ok(())
    }

    /// Removes and returns the raw value stored under `key`.
    pub fn remove(&mut self, key: &str) -> Option<serde_json::Value> {
        self.0.remove(key)
    }

    /// Whether `key` is present.
    #[must_use]
    pub fn contains_key(&self, key: &str) -> bool {
        self.0.contains_key(key)
    }

    /// The undocumented fields, in key order.
    pub fn iter(&self) -> impl Iterator<Item = (&String, &serde_json::Value)> {
        self.0.iter()
    }

    /// The field names, in order.
    pub fn keys(&self) -> impl Iterator<Item = &String> {
        self.0.keys()
    }
}

impl Validate for Extensions {
    // Undocumented fields carry no spec constraints by definition.
    fn validate_in(&self, _v: &mut Validator) {}
}

impl<K: Into<String>, V: Into<serde_json::Value>> FromIterator<(K, V)> for Extensions {
    fn from_iter<I: IntoIterator<Item = (K, V)>>(iter: I) -> Self {
        Self(iter.into_iter().map(|(k, v)| (k.into(), v.into())).collect())
    }
}

impl<'a> IntoIterator for &'a Extensions {
    type Item = (&'a String, &'a serde_json::Value);
    type IntoIter = std::collections::btree_map::Iter<'a, String, serde_json::Value>;
    fn into_iter(self) -> Self::IntoIter {
        self.0.iter()
    }
}

#[cfg(feature = "schema")]
impl schemars::JsonSchema for Extensions {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "Extensions".into()
    }
    fn json_schema(_g: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({
            "type": "object",
            "additionalProperties": true,
            "description": "Undocumented JSON fields, preserved verbatim",
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typed_access_round_trips() {
        let mut ext = Extensions::new();
        ext.insert("nltnm_rank", 7u32).unwrap();
        assert_eq!(ext.get::<u32>("nltnm_rank").unwrap(), Some(7));
        assert_eq!(ext.get::<u32>("absent").unwrap(), None);
        assert!(ext.get::<String>("nltnm_rank").is_err(), "type mismatch is an error");
    }

    #[test]
    fn empty_extensions_are_invisible() {
        let ext = Extensions::new();
        assert!(ext.is_empty());
        assert_eq!(serde_json::to_string(&ext).unwrap(), "{}");
    }
}
