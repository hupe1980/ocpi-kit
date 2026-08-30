//! PATCH: JSON Merge Patch with the two rules OCPI adds on top.
//!
//! > *A PATCH request must only specify the object's identifier (if needed to identify this
//! > object) and the fields to be updated. Any fields (both required or optional) that are left
//! > out remain unchanged.*
//!
//! That is [RFC 7396 JSON Merge Patch](https://datatracker.ietf.org/doc/html/rfc7396), plus:
//!
//! 1. `last_updated` is required in every PATCH — a patch without it is
//!    [`StatusCode::INVALID_PARAMETERS`](super::StatusCode::INVALID_PARAMETERS), the spec's own
//!    example of what 2001 means.
//! 2. The result must still be a valid object, so a patch that nulls a required field is refused
//!    rather than applied.
//!
//! Spec: 2.3.0 §transport_and_format_patch, §status_codes_2xxx_client_errors

use core::fmt;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::types::{DateTime, Validate, Violations};

use super::envelope::OcpiError;
use super::status::StatusCode;

/// A partial update to an OCPI object.
///
/// Held as a JSON object rather than as a per-field `Option` struct, because that is what a merge
/// patch is: the difference between "absent" and "present and null" is the whole semantics, and
/// a struct of `Option`s cannot express it.
///
/// ```
/// use ocpi_kit::transport::Patch;
/// use ocpi_kit::v2_3_0::locations::Evse;
///
/// let patch: Patch<Evse> = serde_json::from_str(
///     r#"{"status":"CHARGING","last_updated":"2019-06-24T12:39:09Z"}"#,
/// ).unwrap();
/// assert!(patch.last_updated().is_some());
/// assert!(patch.touches("status"));
/// ```
///
/// Spec: 2.3.0 §transport_and_format_patch
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(transparent, bound = "")]
pub struct Patch<T> {
    body: Value,
    #[serde(skip)]
    _target: core::marker::PhantomData<fn() -> T>,
}

impl<T> Patch<T> {
    /// Wraps a JSON value as a patch.
    #[must_use]
    pub fn from_value(body: Value) -> Self {
        Self { body, _target: core::marker::PhantomData }
    }

    /// Builds a patch by serialising `value`, keeping only the fields it writes.
    ///
    /// # Errors
    ///
    /// Propagates the `serde_json` error if `value` cannot be serialised.
    pub fn from_partial<P: Serialize>(value: &P) -> Result<Self, serde_json::Error> {
        Ok(Self::from_value(serde_json::to_value(value)?))
    }

    /// The patch as a JSON value.
    #[must_use]
    pub const fn as_value(&self) -> &Value {
        &self.body
    }

    /// Consumes the patch and yields the JSON value.
    #[must_use]
    pub fn into_value(self) -> Value {
        self.body
    }

    /// Whether the patch writes `field` at the top level, including writing it to `null`.
    #[must_use]
    pub fn touches(&self, field: &str) -> bool {
        self.body.as_object().is_some_and(|o| o.contains_key(field))
    }

    /// The `last_updated` the patch carries, if any.
    #[must_use]
    pub fn last_updated(&self) -> Option<DateTime> {
        self.body.get("last_updated")?.as_str()?.parse().ok()
    }

    /// Re-types this patch to the object it is meant to be applied to.
    ///
    /// A merge patch is untyped on the wire, so a server extractor produces a
    /// `Patch<serde_json::Value>`; this names the type it will be applied to, which is what
    /// [`Patch::apply`] needs in order to check the result.
    #[must_use]
    pub fn retype<U>(self) -> Patch<U> {
        Patch::from_value(self.body)
    }

    /// The fields the patch writes, at the top level.
    #[must_use]
    pub fn fields(&self) -> Vec<&str> {
        self.body.as_object().map(|o| o.keys().map(String::as_str).collect()).unwrap_or_default()
    }
}

impl<T> Patch<T>
where
    T: Serialize + serde::de::DeserializeOwned + Validate,
{
    /// Applies this patch to `target`, returning the updated object.
    ///
    /// The whole operation is checked before anything is returned:
    ///
    /// * a patch without `last_updated` is refused with `2001`, as the spec's own example says;
    /// * the merged object must still deserialise into `T`, so a patch that removes a required
    ///   field is refused rather than producing a half-object;
    /// * the merged object must still satisfy [`Validate`], so a patch cannot turn a conformant
    ///   object into a non-conformant one.
    ///
    /// # Errors
    ///
    /// Returns [`OcpiError::Decode`] with a `2001`-shaped message when any of those fail.
    pub fn apply(&self, target: &T) -> Result<T, OcpiError> {
        if self.last_updated().is_none() {
            return Err(OcpiError::Decode {
                path: "/last_updated".to_owned(),
                message: format!("a PATCH must carry `last_updated` ({})", StatusCode::INVALID_PARAMETERS),
            });
        }

        let mut merged = serde_json::to_value(target)
            .map_err(|e| OcpiError::Decode { path: "/".to_owned(), message: e.to_string() })?;
        merge(&mut merged, &self.body);

        let updated: T = serde_json::from_value(merged).map_err(|e| OcpiError::Decode {
            path: "/".to_owned(),
            message: format!(
                "the patched object is no longer a valid object: {e}; \
                 a PATCH may not remove a required field"
            ),
        })?;

        updated.validate().map_err(|violations: Violations| OcpiError::Decode {
            path: violations.as_slice().first().map_or("/", |v| v.pointer.as_str()).to_owned(),
            message: format!("the patched object no longer conforms: {violations}"),
        })?;

        Ok(updated)
    }
}

impl<T> fmt::Display for Patch<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.body)
    }
}

/// Applies an RFC 7396 JSON Merge Patch in place.
///
/// > *If the patch is anything other than an object, the result will always be to replace the
/// > entire target with the entire patch. Also, it is not possible to patch part of a target that
/// > is not an object … null values in the merge patch are given special meaning to indicate the
/// > removal of existing values in the target.*
///
/// This is exported because a hub needs it to apply a patch it is forwarding without knowing the
/// object's type.
pub fn merge(target: &mut Value, patch: &Value) {
    let Some(patch_object) = patch.as_object() else {
        *target = patch.clone();
        return;
    };
    if !target.is_object() {
        *target = Value::Object(Map::new());
    }
    let target_object = target.as_object_mut().expect("just replaced with an object");
    for (key, value) in patch_object {
        if value.is_null() {
            target_object.remove(key);
        } else {
            merge(target_object.entry(key.clone()).or_insert(Value::Null), value);
        }
    }
}

/// What a client should do after a PATCH that the peer refused.
///
/// > *In case a PATCH request fails, the client is expected to call the GET method to check the
/// > state of the object in the other party's system. If the object doesn't exist, the client
/// > should do a PUT.*
///
/// Spec: 2.3.0 §transport_and_format_patch
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PatchFallback {
    /// GET the object to see what the peer has.
    GetThenReconcile,
    /// The object does not exist at the peer; PUT the whole object.
    PutWholeObject,
}

/// Decides the fallback for a failed PATCH from the error the peer returned.
#[must_use]
pub fn patch_fallback(error: &OcpiError) -> PatchFallback {
    match error {
        OcpiError::NotFound(_) => PatchFallback::PutWholeObject,
        _ => PatchFallback::GetThenReconcile,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn merge_follows_rfc_7396() {
        // The RFC's own example table.
        let cases = [
            (json!({"a": "b"}), json!({"a": "c"}), json!({"a": "c"})),
            (json!({"a": "b"}), json!({"b": "c"}), json!({"a": "b", "b": "c"})),
            (json!({"a": "b"}), json!({"a": null}), json!({})),
            (json!({"a": "b", "b": "c"}), json!({"a": null}), json!({"b": "c"})),
            (json!({"a": [{"b": "c"}]}), json!({"a": [1]}), json!({"a": [1]})),
            (json!({"a": {"b": "c"}}), json!({"a": {"b": "d"}}), json!({"a": {"b": "d"}})),
            (json!({"a": [{"b": "c"}]}), json!({"a": "replaced"}), json!({"a": "replaced"})),
        ];
        for (mut target, patch, expected) in cases {
            merge(&mut target, &patch);
            assert_eq!(target, expected);
        }
    }

    #[test]
    fn merging_a_non_object_patch_replaces_the_target() {
        let mut target = json!({"a": 1});
        merge(&mut target, &json!("scalar"));
        assert_eq!(target, json!("scalar"));
    }

    #[test]
    fn the_fallback_matches_the_spec_advice() {
        assert_eq!(
            patch_fallback(&OcpiError::NotFound("no such EVSE".into())),
            PatchFallback::PutWholeObject
        );
        assert_eq!(patch_fallback(&OcpiError::Transport("timeout".into())), PatchFallback::GetThenReconcile);
    }

    #[test]
    fn a_patch_reports_the_fields_it_writes_including_nulls() {
        let patch: Patch<()> = Patch::from_value(json!({"status": "CHARGING", "name": null}));
        assert!(patch.touches("status") && patch.touches("name"));
        assert!(!patch.touches("id"));
        let mut fields = patch.fields();
        fields.sort_unstable();
        assert_eq!(fields, vec!["name", "status"]);
    }
}
