//! Wires the real `fc-loan-emi` progressive interest engine behind the [`EmiCalculator`] seam that the Advanced
//! payment-allocation processor delegates to.
//!
//! The Advanced processor was ported against a trait *seam* ([`crate::advanced::EmiCalculator`]) so its orchestration
//! could be verified by mocking the calculator. This adapter closes that seam with the actual engine: it wraps a
//! [`fc_loan_emi::recalc::Model`] (the authoritative repayment / interest-period schedule) and forwards the three seam
//! operations to it. The processor's minimal [`ScheduleModel`] view (just the maturity date) is ignored — the wrapped
//! engine carries the real schedule state, exactly as the original `ProgressiveLoanInterestScheduleModel` does.

use crate::advanced::{EmiCalculator, ScheduleModel};
use fc_loan_emi::recalc::Model;
use fc_money::Money;
use time::Date;

/// The production [`EmiCalculator`] backed by the `fc-loan-emi` engine. Construct it from a fully-built
/// [`fc_loan_emi::recalc::Model`] (via that crate's constructors) and hand `&mut` it to the Advanced processor.
#[derive(Clone, Debug)]
pub struct ProgressiveEmiCalculator {
    model: Model,
}

impl ProgressiveEmiCalculator {
    /// Wrap a fully-constructed progressive interest schedule model.
    #[must_use]
    pub fn new(model: Model) -> Self {
        Self { model }
    }

    /// Borrow the underlying engine model (e.g. to read the resulting schedule after allocation).
    #[must_use]
    pub fn model(&self) -> &Model {
        &self.model
    }

    /// Consume the adapter, returning the underlying engine model.
    #[must_use]
    pub fn into_model(self) -> Model {
        self.model
    }
}

impl EmiCalculator for ProgressiveEmiCalculator {
    fn add_disbursement(&mut self, _model: &ScheduleModel, date: Date, amount: &Money) {
        self.model.add_disbursement(date, amount);
    }

    fn pay_principal(
        &mut self,
        _model: &ScheduleModel,
        period_from: Date,
        period_due: Date,
        pay_date: Date,
        amount: &Money,
    ) {
        self.model
            .pay_principal(period_from, period_due, pay_date, amount);
    }

    fn pay_interest(
        &mut self,
        _model: &ScheduleModel,
        period_from: Date,
        period_due: Date,
        pay_date: Date,
        amount: &Money,
    ) {
        self.model
            .pay_interest(period_from, period_due, pay_date, amount);
    }
}
