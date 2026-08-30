//! Carrying objects between OCPI versions, with an explicit account of what was lost.
//!
//! A hub that connects a 2.2.1 CPO to a 2.3.0 eMSP has to translate every object that crosses it.
//! The translations are not symmetric: going forward is almost always total, going back means
//! deciding what to do with the fields the older version does not have. Silently dropping them —
//! which is what a hand-written `From` impl does — turns a hub into a data shredder that nobody
//! notices until an invoice is wrong.
//!
//! So both directions return a [`Converted<T>`]: the object, plus a [`Lossy`] report naming every
//! field that could not be carried, by JSON Pointer, with the reason.
//!
//! ```
//! use ocpi_kit::convert::{Downgrade, Upgrade};
//! use ocpi_kit::{v2_2_1, v2_3_0};
//!
//! // 2.2.1 → 2.3.0: `incl_vat` becomes a VAT tax line.
//! let old = v2_2_1::Price::with_vat("5.00".parse().unwrap(), "5.50".parse().unwrap());
//! let new: v2_3_0::Price = old.upgrade().expect_lossless();
//! assert_eq!(new.taxes.len(), 1);
//! assert_eq!(new.after_taxes().to_string(), "5.50");
//!
//! // 2.3.0 → 2.2.1: several named taxes collapse into one `incl_vat`, and that is reported.
//! let mut multi = v2_3_0::Price::new("5.00".parse().unwrap());
//! multi.taxes.push(v2_3_0::TaxAmount::new("GST", None, "0.25".parse().unwrap()).unwrap());
//! multi.taxes.push(v2_3_0::TaxAmount::new("QST", None, "0.50".parse().unwrap()).unwrap());
//! let back = multi.downgrade();
//! let old: v2_2_1::Price = back.value;
//! assert_eq!(old.incl_vat.unwrap().to_string(), "5.75");
//! assert!(!back.lossy.is_empty(), "the tax names did not survive");
//! ```
//!
//! # What the direction of a conversion means
//!
//! * [`Upgrade`] goes to a **newer** version. Where the newer version added a required field, the
//!   default is chosen from the older version's semantics and documented on the impl — for
//!   example a 2.2.1 `Tariff` becomes a 2.3.0 one with `tax_included: NO`, because a 2.2.1
//!   `PriceComponent.price` is *"Price per unit (excl. VAT)"* by definition.
//! * [`Downgrade`] goes to an **older** version, and is where losses accumulate.
//!
//! # Enum values survive both directions
//!
//! Because the enums OCPI 2.3.0 opened are decoded leniently in 2.2.1 too
//! ([`ocpi_lenient_enum!`](crate::ocpi_lenient_enum)), a 2.3.0 `ConnectorType::Mcs` downgrades to
//! a 2.2.1 `ConnectorType::Custom("MCS")` — same string on the wire, no data lost — and upgrades
//! straight back. Only *fields that do not exist* in the older version are ever dropped.

use core::fmt;

pub mod v2_2_1_v2_3_0;
pub mod wire;

/// One piece of information that a conversion could not carry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Loss {
    /// JSON Pointer (RFC 6901) to the value in the **source** object.
    pub pointer: String,
    /// What happened to it, and why.
    pub reason: String,
}

impl fmt::Display for Loss {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let at = if self.pointer.is_empty() { "/" } else { &self.pointer };
        write!(f, "{at}: {}", self.reason)
    }
}

/// Everything a conversion could not carry, in document order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Lossy(Vec<Loss>);

impl Lossy {
    /// An empty report: nothing was lost.
    #[must_use]
    pub fn none() -> Self {
        Self::default()
    }

    /// Whether the conversion was lossless.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// How many pieces of information were lost.
    #[must_use]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// The losses, in document order.
    #[must_use]
    pub fn as_slice(&self) -> &[Loss] {
        &self.0
    }

    /// The losses, in document order.
    pub fn iter(&self) -> core::slice::Iter<'_, Loss> {
        self.0.iter()
    }

    /// Records a loss.
    pub fn record(&mut self, pointer: impl Into<String>, reason: impl Into<String>) {
        self.0.push(Loss { pointer: pointer.into(), reason: reason.into() });
    }

    /// Merges another report in, prefixing each of its pointers with `prefix`.
    ///
    /// Used to lift the losses of a nested object into its parent's coordinates.
    pub fn absorb(&mut self, prefix: &str, other: Self) {
        for loss in other.0 {
            self.0.push(Loss { pointer: format!("{prefix}{}", loss.pointer), reason: loss.reason });
        }
    }

    /// A one-line summary suitable for an OCPI `status_message`.
    ///
    /// A hub can attach this to a forwarded response so the receiving party knows the object was
    /// translated and what did not survive.
    #[must_use]
    pub fn to_status_message(&self) -> Option<String> {
        if self.is_empty() {
            return None;
        }
        Some(format!(
            "version bridged with {} loss(es): {}",
            self.0.len(),
            self.0.iter().map(ToString::to_string).collect::<Vec<_>>().join("; ")
        ))
    }
}

impl<'a> IntoIterator for &'a Lossy {
    type Item = &'a Loss;
    type IntoIter = core::slice::Iter<'a, Loss>;
    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl IntoIterator for Lossy {
    type Item = Loss;
    type IntoIter = std::vec::IntoIter<Loss>;
    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}

impl fmt::Display for Lossy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, loss) in self.0.iter().enumerate() {
            if i > 0 {
                f.write_str("; ")?;
            }
            write!(f, "{loss}")?;
        }
        Ok(())
    }
}

/// The result of a version conversion: the object, and what it cost.
#[derive(Clone, Debug, PartialEq)]
pub struct Converted<T> {
    /// The converted object.
    pub value: T,
    /// Everything that could not be carried across.
    pub lossy: Lossy,
}

impl<T> Converted<T> {
    /// A conversion that lost nothing.
    #[must_use]
    pub fn lossless(value: T) -> Self {
        Self { value, lossy: Lossy::none() }
    }

    /// A conversion with a report.
    #[must_use]
    pub fn new(value: T, lossy: Lossy) -> Self {
        Self { value, lossy }
    }

    /// Whether nothing was lost.
    #[must_use]
    pub fn is_lossless(&self) -> bool {
        self.lossy.is_empty()
    }

    /// The object, discarding the report.
    ///
    /// Named to be conspicuous: reaching for this is how a hub silently loses data.
    #[must_use]
    pub fn ignore_losses(self) -> T {
        self.value
    }

    /// The object, or the report if anything was lost.
    ///
    /// # Errors
    ///
    /// Returns the [`Lossy`] report when the conversion was not lossless.
    pub fn into_lossless(self) -> Result<T, Lossy> {
        if self.lossy.is_empty() { Ok(self.value) } else { Err(self.lossy) }
    }

    /// The object, panicking if anything was lost. For tests and examples.
    ///
    /// # Panics
    ///
    /// Panics when the conversion lost something.
    #[must_use]
    pub fn expect_lossless(self) -> T {
        assert!(self.lossy.is_empty(), "conversion was not lossless: {}", self.lossy);
        self.value
    }

    /// Applies `f` to the value, keeping the report.
    pub fn map<U, F: FnOnce(T) -> U>(self, f: F) -> Converted<U> {
        Converted { value: f(self.value), lossy: self.lossy }
    }
}

impl<T> core::ops::Deref for Converted<T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.value
    }
}

/// Converts an object to a **newer** OCPI version.
///
/// Where the newer version introduced a required field, the impl documents the default it picks
/// and the spec text that justifies it.
pub trait Upgrade<T> {
    /// Converts to the newer version.
    fn upgrade(self) -> Converted<T>;
}

/// Converts an object to an **older** OCPI version.
///
/// This is where information goes missing; every impl reports what it dropped.
pub trait Downgrade<T> {
    /// Converts to the older version.
    fn downgrade(self) -> Converted<T>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn losses_are_lifted_into_the_parents_coordinates() {
        let mut child = Lossy::none();
        child.record("/help_phone", "not present in OCPI 2.2.1");
        let mut parent = Lossy::none();
        parent.absorb("/evses/0", child);
        assert_eq!(parent.as_slice()[0].pointer, "/evses/0/help_phone");
    }

    #[test]
    fn a_lossless_conversion_unwraps_and_a_lossy_one_does_not() {
        let clean: Converted<u8> = Converted::lossless(7);
        assert!(clean.is_lossless());
        assert_eq!(clean.into_lossless().unwrap(), 7);

        let mut lossy = Lossy::none();
        lossy.record("/x", "dropped");
        let dirty = Converted::new(7u8, lossy);
        assert!(dirty.clone().into_lossless().is_err());
        assert_eq!(dirty.ignore_losses(), 7);
    }

    #[test]
    fn a_report_renders_as_a_status_message() {
        assert_eq!(Lossy::none().to_status_message(), None);
        let mut lossy = Lossy::none();
        lossy.record("/help_phone", "not present in OCPI 2.2.1");
        let message = lossy.to_status_message().unwrap();
        assert!(message.contains("1 loss(es)") && message.contains("/help_phone"), "{message}");
    }
}
