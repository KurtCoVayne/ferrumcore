//! Capitalized-income amortization — faithful port of `CapitalizedIncomeAmortizationUtil`
//! (`fineract-provider/.../portfolio/loanaccount/util/CapitalizedIncomeAmortizationUtil.java`).
//!
//! The capitalized income added to a loan is recognized (amortized) linearly over the remaining life of the loan:
//! between adjustments the unrecognized amount amortizes day-proportionally toward maturity, each adjustment reduces
//! the unrecognized pool (an over-adjustment is carried as a negative correction), and the tail prorates from the
//! last adjustment to the requested date. Verified against a JVM `BigDecimal` replay of the exact Java arithmetic
//! (`MathContext(12, HALF_EVEN)` division, exact multiplication).

use bigdecimal::BigDecimal;
use bigdecimal::RoundingMode;
use fc_loan_emi::Mc;
use fc_money::Money;
use time::Date;

/// A capitalized-income adjustment transaction (date + amount), as seen by the amortization walk.
#[derive(Clone, Debug)]
pub struct CapitalizedIncomeAdjustment {
    pub date: Date,
    pub amount: Money,
}

/// `calculateTotalAmortizationTillDateEqualAmortization`: the total capitalized income recognized by `till_date`
/// under the EQUAL_AMORTIZATION strategy (the only strategy Java defines). `balance_amount`/`balance_date` are the
/// capitalized-income balance and its start date; adjustments are processed in date order; `Money` wrapping happens
/// at the end exactly like `Money.of(currency, total.add(overAmortizationCorrection))`.
#[must_use]
pub fn calculate_total_amortization_till_date(
    balance_amount: &Money,
    balance_date: Date,
    adjustments: &[CapitalizedIncomeAdjustment],
    maturity_date: Date,
    till_date: Date,
    scale: i64,
    mode: RoundingMode,
) -> Money {
    let mc = Mc::new(12, RoundingMode::HalfEven);
    let mut unrecognized: BigDecimal = balance_amount.amount().clone();
    let mut total = BigDecimal::from(0);
    let mut over_correction = BigDecimal::from(0);

    let mut sorted: Vec<&CapitalizedIncomeAdjustment> = adjustments.iter().collect();
    sorted.sort_by_key(|a| a.date);

    let mut period_start = balance_date;
    for adj in sorted {
        let days_until_maturity = (maturity_date - period_start).whole_days();
        let days_of_period = (adj.date - period_start).whole_days();
        let period_amortization = if days_until_maturity == 0 {
            BigDecimal::from(0)
        } else {
            // Exact multiply, MathContext(12, HALF_EVEN) divide — the Java operation order.
            mc.div(
                &fc_money::decimal::mul(&unrecognized, &BigDecimal::from(days_of_period)),
                &BigDecimal::from(days_until_maturity),
            )
        };
        total = fc_money::decimal::add(&total, &period_amortization);
        unrecognized = fc_money::decimal::sub(
            &fc_money::decimal::sub(&unrecognized, &period_amortization),
            adj.amount.amount(),
        );
        if fc_money::decimal::compare(&unrecognized, &BigDecimal::from(0)) < 0 {
            over_correction = fc_money::decimal::add(&over_correction, &unrecognized);
            unrecognized = BigDecimal::from(0);
        }
        period_start = adj.date;
    }

    if period_start < till_date {
        let days_until_maturity = (maturity_date - period_start).whole_days();
        let days_of_period = (till_date - period_start).whole_days();
        let period_amortization = mc.div(
            &fc_money::decimal::mul(&unrecognized, &BigDecimal::from(days_of_period)),
            &BigDecimal::from(days_until_maturity),
        );
        total = fc_money::decimal::add(&total, &period_amortization);
    } else if balance_date == maturity_date {
        total = fc_money::decimal::add(&total, &unrecognized);
    }

    Money::of(
        scale,
        mode,
        &fc_money::decimal::add(&total, &over_correction),
    )
}
