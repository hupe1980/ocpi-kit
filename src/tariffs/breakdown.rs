//! The auditable output of a pricing run.
//!
//! The point of a breakdown is that somebody can *check* it: a dispute over a €12 session is
//! settled by pointing at which Tariff Element priced which quantity, not by re-running the
//! engine and hoping.

use core::fmt;

use serde::{Deserialize, Serialize};

use crate::types::{DateTime, Number};
use crate::v2_3_0::tariffs::TariffDimensionType;

/// What one dimension of a session cost, and how that was arrived at.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DimensionCost {
    /// The dimension priced.
    pub dimension: TariffDimensionType,
    /// The quantity actually measured, before `step_size` was applied.
    pub measured: Number,
    /// The quantity billed, after `step_size` was applied.
    pub billed: Number,
    /// The cost, excluding any VAT named on the price components.
    pub cost: Number,
    /// The VAT owed on that cost.
    pub vat: Number,
    /// Each stretch of the session that was priced at one rate.
    pub segments: Vec<PricedSegment>,
}

impl DimensionCost {
    /// The cost including VAT.
    #[must_use]
    pub fn cost_with_vat(&self) -> Number {
        self.cost + self.vat
    }

    /// Whether `step_size` changed the billed quantity.
    #[must_use]
    pub fn was_quantised(&self) -> bool {
        self.billed != self.measured
    }
}

/// One stretch of a session priced at a single rate.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PricedSegment {
    /// When this stretch began.
    pub start: DateTime,
    /// The quantity billed in this stretch.
    pub quantity: Number,
    /// The unit price applied.
    pub price: Number,
    /// The VAT percentage applied, if the price component named one.
    pub vat_percentage: Option<Number>,
    /// The cost of this stretch, excluding VAT.
    pub cost: Number,
    /// Which Tariff Element priced it.
    pub applied: AppliedComponent,
}

/// The exact Price Component that priced a segment.
///
/// This is the audit trail: a `tariff_id` plus an index into its `elements` names one row of one
/// object the CPO published, which can be quoted back at them.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppliedComponent {
    /// The `Tariff.id` the component came from.
    pub tariff_id: String,
    /// The index of the Tariff Element within that Tariff's `elements`.
    pub element_index: usize,
    /// The index of the Price Component within that element's `price_components`.
    pub component_index: usize,
    /// Why this element was selected — which restrictions it satisfied — in words.
    pub because: String,
}

impl fmt::Display for AppliedComponent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "tariff {} element {} component {}",
            self.tariff_id, self.element_index, self.component_index
        )
    }
}

/// One tax line of the result.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TaxLine {
    /// The VAT percentage this line covers.
    pub percentage: Number,
    /// The amount the percentage was applied to.
    pub taxable: Number,
    /// The tax owed.
    pub amount: Number,
}

/// Why the total was clamped.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PriceLimitApplied {
    /// The session cost less than `Tariff.min_price`, so the minimum was charged.
    Minimum,
    /// The session cost more than `Tariff.max_price`, so the maximum was charged.
    Maximum,
}

/// The complete, auditable result of pricing a session.
///
/// ```
/// # use ocpi_kit::tariffs::CostBreakdown;
/// # fn show(breakdown: &CostBreakdown) {
/// for dimension in &breakdown.dimensions {
///     println!(
///         "{:>12} {:>8} billed (measured {:>8}) = {}",
///         dimension.dimension, dimension.billed, dimension.measured, dimension.cost,
///     );
///     for segment in &dimension.segments {
///         println!("             via {}", segment.applied);
///     }
/// }
/// println!("total {} ({} incl. VAT)", breakdown.total_excl_vat, breakdown.total_incl_vat);
/// # }
/// ```
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CostBreakdown {
    /// What each dimension cost.
    pub dimensions: Vec<DimensionCost>,
    /// The total excluding VAT, after any `min_price`/`max_price` clamp.
    pub total_excl_vat: Number,
    /// The total including VAT, after any clamp.
    pub total_incl_vat: Number,
    /// The VAT owed, grouped by percentage.
    pub taxes: Vec<TaxLine>,
    /// Whether a `min_price` or `max_price` changed the total.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit_applied: Option<PriceLimitApplied>,
    /// Anything the engine could not price, and why.
    ///
    /// A session whose Tariff has no Price Component for a dimension it consumed is not an error
    /// — *"there will be no costs for that Tariff Dimension"* — but it is worth surfacing.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
}

impl CostBreakdown {
    /// The cost of one dimension, if it was priced.
    #[must_use]
    pub fn dimension(&self, dimension: TariffDimensionType) -> Option<&DimensionCost> {
        self.dimensions.iter().find(|d| d.dimension == dimension)
    }

    /// The total for one dimension, or zero if it was not priced.
    #[must_use]
    pub fn dimension_total(&self, dimension: TariffDimensionType) -> Number {
        self.dimension(dimension).map_or(Number::ZERO, |d| d.cost)
    }

    /// The total VAT across all dimensions.
    #[must_use]
    pub fn total_vat(&self) -> Number {
        self.taxes.iter().map(|t| t.amount).sum()
    }

    /// Every Price Component that contributed, in the order they were applied.
    pub fn applied_components(&self) -> impl Iterator<Item = &AppliedComponent> {
        self.dimensions.iter().flat_map(|d| d.segments.iter().map(|s| &s.applied))
    }

    /// The result as an OCPI 2.3.0 [`Price`](crate::v2_3_0::types::Price).
    ///
    /// The VAT lines become [`TaxAmount`](crate::v2_3_0::types::TaxAmount)s named `VAT`, one per
    /// distinct percentage, which is what a Tariff's per-component `vat` fields describe.
    #[cfg(feature = "v2_3_0")]
    #[must_use]
    pub fn to_price_v2_3_0(&self) -> crate::v2_3_0::types::Price {
        crate::v2_3_0::types::Price {
            before_taxes: self.total_excl_vat,
            taxes: self
                .taxes
                .iter()
                .map(|t| crate::v2_3_0::types::TaxAmount {
                    name: crate::types::OcpiText::new_lenient("VAT"),
                    account_number: None,
                    percentage: Some(t.percentage),
                    amount: t.amount,
                    extensions: crate::types::Extensions::new(),
                })
                .collect(),
            extensions: crate::types::Extensions::new(),
        }
    }

    /// The result as an OCPI 2.2.1 [`Price`](crate::v2_2_1::types::Price).
    #[cfg(feature = "v2_2_1")]
    #[must_use]
    pub fn to_price_v2_2_1(&self) -> crate::v2_2_1::types::Price {
        crate::v2_2_1::types::Price {
            excl_vat: self.total_excl_vat,
            incl_vat: Some(self.total_incl_vat),
            extensions: crate::types::Extensions::new(),
        }
    }
}

impl fmt::Display for CostBreakdown {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for d in &self.dimensions {
            writeln!(
                f,
                "{:<13} {:>10} billed ({:>10} measured)  = {:>10} excl. VAT, {:>10} VAT",
                d.dimension.as_str(),
                d.billed,
                d.measured,
                d.cost,
                d.vat
            )?;
        }
        if let Some(limit) = self.limit_applied {
            writeln!(f, "{limit:?} price limit applied")?;
        }
        write!(
            f,
            "{:<13} {:>10} excl. VAT, {:>10} incl. VAT",
            "TOTAL", self.total_excl_vat, self.total_incl_vat
        )
    }
}
