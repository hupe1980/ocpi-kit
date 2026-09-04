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
    /// The cost of this stretch, **excluding** tax.
    ///
    /// When the Tariff says `tax_included: YES` this is the amount with the tax taken back out,
    /// not the amount the price component states. [`tax_basis`](Self::tax_basis) says which.
    pub cost: Number,
    /// The tax owed on [`cost`](Self::cost).
    ///
    /// Carried rather than recomputed from the percentage, because the two are not the same
    /// number once rounding is involved: on a tax-inclusive tariff the tax is the difference
    /// between the stated gross amount and the net one, so that `cost + tax` is exactly what the
    /// price component said.
    pub tax: Number,
    /// Whether the price component this segment was billed at stated a gross or a net amount.
    pub tax_basis: TaxBasis,
    /// Which Tariff Element priced it.
    pub applied: AppliedComponent,
}

/// Whether the amounts in a Tariff already contain the tax that is owed on them.
///
/// > *`tax_included`: Whether taxes are included in the amounts in this Tariff.*
/// > *YES — Taxes are included in the prices in this Tariff. NO — Taxes are not included, and
/// > will be added on top of the prices in this Tariff. N/A — No taxes are applicable to this
/// > Tariff.*
///
/// This decides what `quantity × price` **means**, so an engine that ignores it overstates a
/// tax-inclusive session by exactly the tax: it treats a gross amount as a net one and then adds
/// the tax on top a second time.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum TaxBasis {
    /// The prices are net; the tax named by each component is added on top. OCPI 2.2.1 has no
    /// other reading, and it is what a 2.2.1 Tariff upgrades to.
    Excluded,
    /// The prices are gross. Where a component names a `vat`, the tax is taken back out of the
    /// amount so that a breakdown still has both totals; where none does, the split is not
    /// derivable and a [`TaxIncludedWithoutRate`](PricingNoteCode::TaxIncludedWithoutRate) note
    /// says so.
    Included,
    /// The Tariff says no tax applies at all, so the two totals are the same number.
    NotApplicable,
    /// Different periods of one session were priced by Tariffs that disagree about this.
    ///
    /// Only ever produced for the breakdown as a whole; every individual segment has one basis.
    Mixed,
}

impl fmt::Display for TaxBasis {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Excluded => "taxes excluded from the tariff's prices",
            Self::Included => "taxes included in the tariff's prices",
            Self::NotApplicable => "no taxes applicable",
            Self::Mixed => "mixed: the tariffs disagree about whether prices include tax",
        })
    }
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
    /// Whether this segment is **reserved** time rather than consumed time.
    ///
    /// OCPI has no reservation dimension — reserved time is priced as `TIME` by an element
    /// carrying a `reservation` restriction — so without this the two are indistinguishable in
    /// the audit trail, and a CDR's `total_reservation_cost` cannot be checked against anything.
    #[serde(default)]
    pub reservation: bool,
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
///
/// The tax lines of a breakdown always sum to `total_incl_vat - total_excl_vat`. That invariant
/// is what makes the breakdown filable: a document whose tax lines disagree with its own totals
/// is not something a tax authority or a disputing partner can act on.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TaxLine {
    /// The VAT percentage this line covers.
    ///
    /// `None` for tax that could not be attributed to a rate — which happens when a
    /// `min_price.after_taxes` raises the inclusive total of a session that had no VAT to
    /// apportion it to. The 2.3.0 wire type
    /// [`TaxAmount`](crate::v2_3_0::types::TaxAmount) has the same optionality, for the same
    /// reason: an amount of tax is a fact, a rate is an explanation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub percentage: Option<Number>,
    /// The amount the percentage was applied to.
    pub taxable: Number,
    /// The tax owed.
    pub amount: Number,
}

/// What a [`PricingNote`] is about, without reading the English.
///
/// An invoice-reconciliation pipeline has to be able to *count* these — how many CDRs this month
/// span a price change? — and grepping a sentence is not a way to do that.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum PricingNoteCode {
    /// A dimension was consumed that no Tariff Element prices.
    ///
    /// Not an error: *"there will be no costs for that Tariff Dimension"*. But a session that
    /// charged 40 kWh for nothing is worth looking at.
    NoPriceComponent,
    /// A Charging Period outlasted the Price Component that priced it.
    ///
    /// > *A CPO SHALL at least start (and add) a ChargingPeriod every moment/event that has
    /// > relevance for the total costs of a CDR. … When an energy changes in price after 17:00,
    /// > the CPO has to start a new Charging Period at 17:00.*
    ///
    /// So this is a defect in the **CDR**, not in the tariff: the period should have been split.
    /// Its quantities cannot be apportioned after the fact — nothing in the data says how much
    /// of the energy fell either side of the boundary — so the period is priced at the rate that
    /// applied when it began, and this note says that happened. See
    /// [`PricingEngine`](crate::tariffs::PricingEngine).
    PeriodSpansPriceChange,
    /// The Charging Periods are not in chronological order.
    ///
    /// Nothing in the property tables says they must be, but everything built on them assumes it:
    /// `step_size` is defined in terms of *"the last relevant PriceComponent"*, and a period's
    /// duration is only knowable as the gap to the next one. Out of order, both are wrong.
    ///
    /// The session is still priced — the quantities are all there — but any restriction that
    /// depends on elapsed time is evaluated against a timeline that does not exist.
    PeriodsOutOfOrder,
    /// A `min_price` or `max_price` moved the total, and the tax lines were moved with it.
    TotalClamped,
    /// The tax the tariff describes came out negative.
    ///
    /// No tariff can mean that: a VAT percentage below zero is a malformed
    /// [`PriceComponent`](crate::v2_3_0::tariffs::PriceComponent), which
    /// [`Validate`](crate::types::Validate) reports. The engine does not require validated input,
    /// so rather than publish a session that costs less with tax than without, it holds the
    /// inclusive total at the exclusive one and says why.
    NegativeTax,
    /// Tax was owed that no rate in the session accounts for.
    ///
    /// Raised when a `min_price.after_taxes` lifts the inclusive total of a session whose price
    /// components named no VAT at all. The amount is real; the rate is not knowable.
    UnattributedTax,
    /// The Tariff's prices include tax, and no price component says at what rate.
    ///
    /// This is the ordinary North American shape — *"tax rates are not typically known
    /// beforehand to the CPO, so the `vat` field in the PriceComponent objects is not filled"* —
    /// and it means the two totals cannot be told apart from the Tariff alone. The engine reports
    /// the amount as **both** totals and raises this, rather than inventing a rate or pretending
    /// the gross figure is a net one.
    TaxIncludedWithoutRate,
    /// A price component named a `vat` percentage on a Tariff that says no tax applies.
    ///
    /// `tax_included: N/A` is a statement that there is no tax; a rate beside it is a
    /// contradiction in the Tariff, and the engine follows the Tariff-level field because that is
    /// the one the specification defines as governing.
    TaxRateIgnored,
    /// One session was priced by Tariffs that disagree about whether prices include tax.
    ///
    /// Legal — a CDR may name a different `tariff_id` per Charging Period — and worth a look,
    /// because the two totals then mean something different for different parts of the session.
    MixedTaxBasis,
}

impl PricingNoteCode {
    /// A short, stable, machine-readable slug.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::NoPriceComponent => "no_price_component",
            Self::PeriodsOutOfOrder => "periods_out_of_order",
            Self::PeriodSpansPriceChange => "period_spans_price_change",
            Self::TotalClamped => "total_clamped",
            Self::NegativeTax => "negative_tax",
            Self::UnattributedTax => "unattributed_tax",
            Self::TaxIncludedWithoutRate => "tax_included_without_rate",
            Self::TaxRateIgnored => "tax_rate_ignored",
            Self::MixedTaxBasis => "mixed_tax_basis",
        }
    }
}

impl fmt::Display for PricingNoteCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Something the engine wants the reader of a breakdown to know.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PricingNote {
    /// What kind of note this is.
    pub code: PricingNoteCode,
    /// The moment in the session it concerns, when it concerns one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at: Option<DateTime>,
    /// The same thing in words, for a human reading the breakdown.
    pub message: String,
}

impl PricingNote {
    pub(super) fn new(code: PricingNoteCode, at: Option<DateTime>, message: impl Into<String>) -> Self {
        Self { code, at, message: message.into() }
    }
}

impl fmt::Display for PricingNote {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.at {
            Some(at) => write!(f, "[{}] at {at}: {}", self.code, self.message),
            None => write!(f, "[{}] {}", self.code, self.message),
        }
    }
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
    /// Whether the tariff's prices already contained the tax.
    ///
    /// [`TaxBasis::Included`] with no rate anywhere means `total_excl_vat` is the **gross**
    /// amount, because nothing in the tariff says how to split it; the accompanying
    /// [`TaxIncludedWithoutRate`](PricingNoteCode::TaxIncludedWithoutRate) note is the signal not
    /// to read it as a net figure.
    pub tax_basis: TaxBasis,
    /// Whether a `min_price` or `max_price` changed the total.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit_applied: Option<PriceLimitApplied>,
    /// Anything the reader of this breakdown should know: see [`PricingNoteCode`].
    ///
    /// Empty is the ordinary case. A note is never an error — the total beside it is the engine's
    /// best answer — but every one of them is a reason to look, and most of them are defects in
    /// the *input* rather than in the tariff.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<PricingNote>,
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
    ///
    /// Always equal to `total_incl_vat - total_excl_vat`; see [`TaxLine`].
    #[must_use]
    pub fn total_vat(&self) -> Number {
        self.taxes.iter().map(|t| t.amount).sum()
    }

    /// The notes carrying `code`.
    pub fn notes_with(&self, code: PricingNoteCode) -> impl Iterator<Item = &PricingNote> {
        self.notes.iter().filter(move |n| n.code == code)
    }

    /// Whether anything about this session needs a human's attention.
    ///
    /// True when the breakdown carries any note at all. A reconciliation pipeline can use this to
    /// split a month's CDRs into the ones that priced cleanly and the ones that did not.
    #[must_use]
    pub fn needs_review(&self) -> bool {
        !self.notes.is_empty()
    }

    /// Every Price Component that contributed, in the order they were applied.
    pub fn applied_components(&self) -> impl Iterator<Item = &AppliedComponent> {
        self.dimensions.iter().flat_map(|d| d.segments.iter().map(|s| &s.applied))
    }

    /// The result as an OCPI 2.3.0 [`Price`](crate::v2_3_0::types::Price).
    ///
    /// The tax lines become [`TaxAmount`](crate::v2_3_0::types::TaxAmount)s named `VAT`, one per
    /// distinct percentage, which is what a Tariff's per-component `vat` fields describe. A line
    /// the engine could not attribute to a rate keeps `percentage: None` and is named `TAX`,
    /// since calling an unattributed amount VAT would be a claim the session does not support.
    ///
    /// **On a tax-inclusive tariff with no rate** ([`TaxBasis::Included`] plus a
    /// [`TaxIncludedWithoutRate`](PricingNoteCode::TaxIncludedWithoutRate) note) `before_taxes`
    /// is the gross amount, because that is the only number the data contains. A CPO filling in
    /// a CDR from this has to supply the tax split from its own accounting.
    #[cfg(feature = "v2_3_0")]
    #[must_use]
    pub fn to_price_v2_3_0(&self) -> crate::v2_3_0::types::Price {
        crate::v2_3_0::types::Price {
            before_taxes: self.total_excl_vat,
            taxes: self
                .taxes
                .iter()
                .map(|t| crate::v2_3_0::types::TaxAmount {
                    name: crate::types::OcpiText::new_lenient(if t.percentage.is_some() {
                        "VAT"
                    } else {
                        "TAX"
                    }),
                    account_number: None,
                    percentage: t.percentage,
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

impl fmt::Display for PriceLimitApplied {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Minimum => "minimum",
            Self::Maximum => "maximum",
        })
    }
}

impl fmt::Display for CostBreakdown {
    /// The whole breakdown, **including its notes**.
    ///
    /// The notes are the part somebody has to act on, so they are not something the default
    /// rendering may quietly leave out. An engine that records a finding and then prints a total
    /// as if nothing happened is worse than one that never looked.
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
        for tax in &self.taxes {
            let rate = tax.percentage.map_or_else(|| "unattributed".to_owned(), |p| format!("{p}%"));
            writeln!(f, "{:<13} {:>10} on {:>10}", format!("VAT {rate}"), tax.amount, tax.taxable)?;
        }
        if let Some(limit) = self.limit_applied {
            writeln!(f, "{:<13} the tariff's {limit} price limit moved the total", "LIMIT")?;
        }
        writeln!(
            f,
            "{:<13} {:>10} excl. VAT, {:>10} incl. VAT",
            "TOTAL", self.total_excl_vat, self.total_incl_vat
        )?;
        for note in &self.notes {
            write!(f, "\n[{}] {}", note.code, note.message)?;
            if let Some(at) = note.at {
                write!(f, " (at {at})")?;
            }
        }
        Ok(())
    }
}
