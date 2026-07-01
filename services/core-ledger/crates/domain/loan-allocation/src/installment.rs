//! [`Installment`] — a faithful port of the per-component arithmetic in `LoanRepaymentScheduleInstallment`.
//!
//! Each of the four payable components (principal / interest / fee / penalty) tracks a *charged* amount and the amounts
//! *accounted for* (paid / waived / written off); `outstanding = charged − accounted`. The `pay_*` atoms consume
//! `min(remaining, outstanding)` into the *paid* bucket; `waive_*` consume into the *waived* bucket; `unpay_*` reverse
//! the *paid* bucket (refund). All arithmetic is currency-scale [`Money`].

use bigdecimal::BigDecimal;
use bigdecimal::RoundingMode;
use fc_money::Money;
use time::Date;

/// One repayment-schedule installment's four payable components, each with its charged and accounted-for amounts.
#[derive(Clone, Debug)]
pub struct Installment {
    scale: i64,
    mode: RoundingMode,
    /// Period end date — distinguishes in-advance (`txn < due`) from due/late within the in-advance handler.
    pub due_date: Date,

    principal: BigDecimal,
    principal_completed: BigDecimal,
    principal_written_off: BigDecimal,

    interest_charged: BigDecimal,
    interest_paid: BigDecimal,
    interest_waived: BigDecimal,
    interest_written_off: BigDecimal,

    fee_charges_charged: BigDecimal,
    fee_charges_paid: BigDecimal,
    fee_charges_waived: BigDecimal,
    fee_charges_written_off: BigDecimal,

    penalty_charges: BigDecimal,
    penalty_charges_paid: BigDecimal,
    penalty_charges_waived: BigDecimal,
    penalty_charges_written_off: BigDecimal,
}

impl Installment {
    /// Build an installment from its charged component amounts (principal, interest, fee, penalty), all accounted-for
    /// buckets zero — matching the original `LoanRepaymentScheduleInstallment` test constructor.
    #[must_use]
    pub fn new(
        scale: i64,
        mode: RoundingMode,
        due_date: Date,
        principal: BigDecimal,
        interest: BigDecimal,
        fee_charges: BigDecimal,
        penalty_charges: BigDecimal,
    ) -> Self {
        let z = || BigDecimal::from(0);
        Self {
            scale,
            mode,
            due_date,
            principal,
            principal_completed: z(),
            principal_written_off: z(),
            interest_charged: interest,
            interest_paid: z(),
            interest_waived: z(),
            interest_written_off: z(),
            fee_charges_charged: fee_charges,
            fee_charges_paid: z(),
            fee_charges_waived: z(),
            fee_charges_written_off: z(),
            penalty_charges,
            penalty_charges_paid: z(),
            penalty_charges_waived: z(),
            penalty_charges_written_off: z(),
        }
    }

    /// Wrap a raw decimal as [`Money`] at this installment's currency scale/mode.
    #[must_use]
    pub fn money(&self, v: &BigDecimal) -> Money {
        Money::of(self.scale, self.mode, v)
    }
    /// A zero amount at this installment's currency scale/mode.
    #[must_use]
    pub fn zero(&self) -> Money {
        Money::zero(self.scale, self.mode)
    }

    /// `principal − (completed + written off)` (Java `getPrincipalOutstanding`).
    #[must_use]
    pub fn principal_outstanding(&self) -> Money {
        let accounted = self
            .money(&self.principal_completed)
            .plus(&self.money(&self.principal_written_off));
        self.money(&self.principal).minus(&accounted)
    }
    /// `interestCharged − (paid + waived + written off)` (Java `getInterestOutstanding`).
    #[must_use]
    pub fn interest_outstanding(&self) -> Money {
        let accounted = self
            .money(&self.interest_paid)
            .plus(&self.money(&self.interest_waived))
            .plus(&self.money(&self.interest_written_off));
        self.money(&self.interest_charged).minus(&accounted)
    }
    /// `feeChargesCharged − (paid + waived + written off)` (Java `getFeeChargesOutstanding`).
    #[must_use]
    pub fn fee_charges_outstanding(&self) -> Money {
        let accounted = self
            .money(&self.fee_charges_paid)
            .plus(&self.money(&self.fee_charges_waived))
            .plus(&self.money(&self.fee_charges_written_off));
        self.money(&self.fee_charges_charged).minus(&accounted)
    }
    /// `penaltyCharges − (paid + waived + written off)` (Java `getPenaltyChargesOutstanding`).
    #[must_use]
    pub fn penalty_charges_outstanding(&self) -> Money {
        let accounted = self
            .money(&self.penalty_charges_paid)
            .plus(&self.money(&self.penalty_charges_waived))
            .plus(&self.money(&self.penalty_charges_written_off));
        self.money(&self.penalty_charges).minus(&accounted)
    }

    /// Paid-to-date getters (used by `unpay_*`).
    #[must_use]
    pub fn principal_completed(&self) -> Money {
        self.money(&self.principal_completed)
    }

    /// The charged principal (Java `getPrincipal`).
    #[must_use]
    pub fn principal(&self) -> Money {
        self.money(&self.principal)
    }

    /// Replace the charged principal (Java `updatePrincipal` / `setPrincipal`) — re-amortization zeroes a past
    /// installment's remaining principal by setting the charge to what was already completed.
    pub fn update_principal(&mut self, principal: &Money) {
        self.principal = principal.amount().clone();
    }

    /// Add to the charged principal (Java `addToPrincipal(date, amount)`, the schedule-mutation flavor used by
    /// re-amortization — distinct from the chargeback credited-amount side-tracking).
    pub fn add_to_principal(&mut self, amount: &Money) {
        self.principal = self.money(&self.principal).plus(amount).amount().clone();
    }

    // ----- PAY (into the paid bucket) -------------------------------------------------------------------------------

    fn consume(amount: &Money, due: &Money) -> Money {
        if amount.is_greater_than_or_equal_to(due) {
            due.clone()
        } else {
            amount.clone()
        }
    }

    /// Consume `min(amount, penaltyOutstanding)` into the penalty *paid* bucket (Java `payPenaltyChargesComponent`).
    pub fn pay_penalty(&mut self, amount: &Money) -> Money {
        if amount.is_zero() {
            return self.zero();
        }
        let portion = Self::consume(amount, &self.penalty_charges_outstanding());
        self.penalty_charges_paid = self
            .money(&self.penalty_charges_paid)
            .plus(&portion)
            .amount()
            .clone();
        portion
    }

    /// Consume `min(amount, feeOutstanding)` into the fee *paid* bucket (Java `payFeeChargesComponent`).
    pub fn pay_fee(&mut self, amount: &Money) -> Money {
        if amount.is_zero() {
            return self.zero();
        }
        let portion = Self::consume(amount, &self.fee_charges_outstanding());
        self.fee_charges_paid = self
            .money(&self.fee_charges_paid)
            .plus(&portion)
            .amount()
            .clone();
        portion
    }

    /// Consume `min(amount, interestOutstanding)` into the interest *paid* bucket (Java `payInterestComponent`).
    pub fn pay_interest(&mut self, amount: &Money) -> Money {
        if amount.is_zero() {
            return self.zero();
        }
        let portion = Self::consume(amount, &self.interest_outstanding());
        self.interest_paid = self
            .money(&self.interest_paid)
            .plus(&portion)
            .amount()
            .clone();
        portion
    }

    /// Consume `min(amount, principalOutstanding)` into the principal *completed* bucket (Java `payPrincipalComponent`).
    pub fn pay_principal(&mut self, amount: &Money) -> Money {
        if amount.is_zero() {
            return self.zero();
        }
        let portion = Self::consume(amount, &self.principal_outstanding());
        self.principal_completed = self
            .money(&self.principal_completed)
            .plus(&portion)
            .amount()
            .clone();
        portion
    }

    // ----- WRITE OFF (set the written-off bucket to the outstanding) ------------------------------------------------

    /// Write off the outstanding principal (Java `writeOffOutstandingPrincipal`): set the written-off amount to the
    /// current outstanding and return it.
    pub fn write_off_principal(&mut self) -> Money {
        let due = self.principal_outstanding();
        self.principal_written_off = due.amount().clone();
        due
    }
    /// Write off the outstanding interest (Java `writeOffOutstandingInterest`).
    pub fn write_off_interest(&mut self) -> Money {
        let due = self.interest_outstanding();
        self.interest_written_off = due.amount().clone();
        due
    }
    /// Write off the outstanding fee charges (Java `writeOffOutstandingFeeCharges`).
    pub fn write_off_fee(&mut self) -> Money {
        let due = self.fee_charges_outstanding();
        self.fee_charges_written_off = due.amount().clone();
        due
    }
    /// Write off the outstanding penalty charges (Java `writeOffOutstandingPenaltyCharges`).
    pub fn write_off_penalty(&mut self) -> Money {
        let due = self.penalty_charges_outstanding();
        self.penalty_charges_written_off = due.amount().clone();
        due
    }

    // ----- WAIVE (into the waived bucket) ---------------------------------------------------------------------------

    /// Consume `min(amount, penaltyOutstanding)` into the penalty *waived* bucket (Java `waivePenaltyChargesComponent`).
    pub fn waive_penalty(&mut self, amount: &Money) -> Money {
        if amount.is_zero() {
            return self.zero();
        }
        let portion = Self::consume(amount, &self.penalty_charges_outstanding());
        self.penalty_charges_waived = self
            .money(&self.penalty_charges_waived)
            .plus(&portion)
            .amount()
            .clone();
        portion
    }

    /// Consume `min(amount, feeOutstanding)` into the fee *waived* bucket (Java `waiveFeeChargesComponent`).
    pub fn waive_fee(&mut self, amount: &Money) -> Money {
        if amount.is_zero() {
            return self.zero();
        }
        let portion = Self::consume(amount, &self.fee_charges_outstanding());
        self.fee_charges_waived = self
            .money(&self.fee_charges_waived)
            .plus(&portion)
            .amount()
            .clone();
        portion
    }

    /// Consume `min(amount, interestOutstanding)` into the interest *waived* bucket (Java `waiveInterestComponent`).
    pub fn waive_interest(&mut self, amount: &Money) -> Money {
        if amount.is_zero() {
            return self.zero();
        }
        let portion = Self::consume(amount, &self.interest_outstanding());
        self.interest_waived = self
            .money(&self.interest_waived)
            .plus(&portion)
            .amount()
            .clone();
        portion
    }

    // ----- UNPAY (reverse the paid bucket; refund) ----------------------------------------------------------------
    // Faithful to the original: no zero short-circuit; deduct min(amount, paid) from the paid bucket.

    fn reverse(amount: &Money, completed: &Money) -> Money {
        if amount.is_greater_than_or_equal_to(completed) {
            completed.clone()
        } else {
            amount.clone()
        }
    }

    /// Reverse `min(amount, penaltyPaid)` out of the penalty *paid* bucket (Java `unpayPenaltyChargesComponent`).
    pub fn unpay_penalty(&mut self, amount: &Money) -> Money {
        let completed = self.money(&self.penalty_charges_paid);
        let deducted = Self::reverse(amount, &completed);
        self.penalty_charges_paid = completed.minus(&deducted).amount().clone();
        deducted
    }

    /// Reverse `min(amount, feePaid)` out of the fee *paid* bucket (Java `unpayFeeChargesComponent`).
    pub fn unpay_fee(&mut self, amount: &Money) -> Money {
        let completed = self.money(&self.fee_charges_paid);
        let deducted = Self::reverse(amount, &completed);
        self.fee_charges_paid = completed.minus(&deducted).amount().clone();
        deducted
    }

    /// Reverse `min(amount, interestPaid)` out of the interest *paid* bucket (Java `unpayInterestComponent`).
    pub fn unpay_interest(&mut self, amount: &Money) -> Money {
        let completed = self.money(&self.interest_paid);
        let deducted = Self::reverse(amount, &completed);
        self.interest_paid = completed.minus(&deducted).amount().clone();
        deducted
    }

    /// Reverse `min(amount, principalCompleted)` out of the principal *completed* bucket (Java
    /// `unpayPrincipalComponent`).
    pub fn unpay_principal(&mut self, amount: &Money) -> Money {
        let completed = self.money(&self.principal_completed);
        let deducted = Self::reverse(amount, &completed);
        self.principal_completed = completed.minus(&deducted).amount().clone();
        deducted
    }
}
