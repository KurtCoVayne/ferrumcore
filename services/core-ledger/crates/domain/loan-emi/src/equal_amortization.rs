//! Equal-amortization split — faithful port of `EqualAmortizationValues`
//! (`.../loanproduct/calc/data/EqualAmortizationValues.java`) and
//! `ProgressiveEMICalculator.calculateEqualAmortizationValues` (the per-installment equal split used by re-age /
//! re-amortize). All arithmetic is currency-scale [`Money`].

use bigdecimal::BigDecimal;
use fc_money::Money;

/// The result of an equal-amortization split: an equal per-installment `value` plus a last-installment `adjustment`
/// that absorbs the rounding remainder (`EqualAmortizationValues`).
#[derive(Clone, Debug)]
pub struct EqualAmortizationValues {
    pub total_outstanding: Money,
    pub number_of_installments: i64,
    pub value: Money,
    pub adjustment: Money,
}

/// Error from [`EqualAmortizationValues::try_add`] when the installment counts differ (the original throws a
/// `RuntimeException`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InstallmentCountMismatch;

impl EqualAmortizationValues {
    /// `getAdjustedValue()` — the last installment's value (`value + adjustment`).
    #[must_use]
    pub fn adjusted_value(&self) -> Money {
        self.value.plus(&self.adjustment)
    }

    /// `calculateValue(index)` — the value for installment `index` (0-based). The last installment gets the adjusted
    /// value; when the adjusted value is negative (a degenerate split), installments are paid `value` until the total
    /// is consumed, then zero.
    #[must_use]
    pub fn calculate_value(&self, index: i64) -> Money {
        if self.adjusted_value().is_less_than_zero() {
            let consumed = self
                .total_outstanding
                .minus(&self.value.multiplied_by(&BigDecimal::from(index + 1)));
            if consumed.is_less_than_zero() {
                self.zero()
            } else {
                self.value.clone()
            }
        } else if index == self.number_of_installments - 1 {
            self.adjusted_value()
        } else {
            self.value.clone()
        }
    }

    /// `calculateValueBigDecimal(index)`.
    #[must_use]
    pub fn calculate_value_decimal(&self, index: i64) -> BigDecimal {
        self.calculate_value(index).amount().clone()
    }

    /// `add(other)` — combine two splits with matching installment counts (`Err` on mismatch, mirroring the original's
    /// `RuntimeException`).
    pub fn try_add(
        &self,
        other: &EqualAmortizationValues,
    ) -> Result<EqualAmortizationValues, InstallmentCountMismatch> {
        if self.number_of_installments != other.number_of_installments {
            return Err(InstallmentCountMismatch);
        }
        Ok(EqualAmortizationValues {
            total_outstanding: self.total_outstanding.plus(&other.total_outstanding),
            number_of_installments: self.number_of_installments,
            value: self.value.plus(&other.value),
            adjustment: self.adjustment.plus(&other.adjustment),
        })
    }

    fn zero(&self) -> Money {
        self.value.multiplied_by(&BigDecimal::from(0))
    }
}

/// `ProgressiveEMICalculator.calculateEqualAmortizationValues` (the `installmentAmountInMultiplesOf == null` case):
/// `value = totalOutstanding / numberOfInstallments` (rounded to currency scale), `adjustment = totalOutstanding −
/// value × numberOfInstallments`; both zero when the total is not positive.
///
/// The `installmentAmountInMultiplesOf` rounding variant (`Money.roundToMultiplesOf`) is a later slice.
#[must_use]
pub fn calculate_equal_amortization_values(
    total_outstanding: &Money,
    number_of_installments: i64,
) -> EqualAmortizationValues {
    let n = BigDecimal::from(number_of_installments);
    if total_outstanding.is_greater_than_zero() {
        let value = total_outstanding.divided_by(&n);
        let adjustment = total_outstanding.minus(&value.multiplied_by(&n));
        EqualAmortizationValues {
            total_outstanding: total_outstanding.clone(),
            number_of_installments,
            value,
            adjustment,
        }
    } else {
        let zero = total_outstanding.multiplied_by(&BigDecimal::from(0));
        EqualAmortizationValues {
            total_outstanding: total_outstanding.clone(),
            number_of_installments,
            value: zero.clone(),
            adjustment: zero,
        }
    }
}

/// `ProgressiveEMICalculator.calculateAdjustedEqualAmortizationValues` (the `installmentAmountInMultiplesOf == null`
/// case): an equal split of `total` whose per-installment `value` is reduced by `sum_of_other_equal_amortization_values`
/// (the portion already covered by other components), with `adjustment = outstanding − value × numberOfInstallments`.
#[must_use]
pub fn calculate_adjusted_equal_amortization_values(
    outstanding: &Money,
    total: &Money,
    sum_of_other_equal_amortization_values: &Money,
    number_of_installments: i64,
) -> EqualAmortizationValues {
    let calculated = calculate_equal_amortization_values(total, number_of_installments);
    let value = calculated
        .value
        .minus(sum_of_other_equal_amortization_values);
    let adjustment =
        outstanding.minus(&value.multiplied_by(&BigDecimal::from(number_of_installments)));
    EqualAmortizationValues {
        total_outstanding: outstanding.clone(),
        number_of_installments,
        value,
        adjustment,
    }
}
