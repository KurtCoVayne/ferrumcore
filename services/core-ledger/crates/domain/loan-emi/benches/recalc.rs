//! Criterion benchmarks for the recalc engine's hot paths (Track B baseline — see the perf plan).
//!
//! Shapes chosen to exercise the known bottlenecks: (1) fresh schedule generation (annuity + cent-adjust +
//! outstanding recompute), (2) a second disbursement (the `copy_without_paid_amounts` + merge + `check_and_adjust`
//! deep-clone path), and (3) a sequential payment loop (the O(n²)-ish getter-recompute cascade that makes the
//! 12/24-period suite tests multi-minute). Period counts are kept modest so the pre-optimization baseline completes;
//! compare before/after with `cargo bench -p fc-loan-emi`.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // bench code

use bigdecimal::{BigDecimal, RoundingMode};
use criterion::{Criterion, criterion_group, criterion_main};
use fc_loan_emi::recalc::{Model, new_declining_balance};
use fc_money::Money;
use std::hint::black_box;
use std::str::FromStr;
use time::{Date, Duration, Month};

fn bd(s: &str) -> BigDecimal {
    BigDecimal::from_str(s).unwrap()
}

fn m(s: &str) -> Money {
    Money::of(2, RoundingMode::HalfEven, &bd(s))
}

/// `n` consecutive monthly periods starting 2024-01-01 (day-1 months, like the suite's schedules).
fn monthly_periods(n: usize) -> Vec<(Date, Date)> {
    let mut dates = Vec::with_capacity(n + 1);
    let mut d = Date::from_calendar_date(2024, Month::January, 1).unwrap();
    dates.push(d);
    for _ in 0..n {
        // First day of the next month.
        let next = d.replace_day(28).unwrap() + Duration::days(5);
        d = next.replace_day(1).unwrap();
        dates.push(d);
    }
    dates.windows(2).map(|w| (w[0], w[1])).collect()
}

fn fresh_model(periods: usize) -> Model {
    new_declining_balance(
        &monthly_periods(periods),
        &bd("1000"),
        &bd("7.0"),
        1,
        2,
        RoundingMode::HalfEven,
    )
}

// History: the pre-optimization baseline (2026-07-01) measured generate_declining_6 ≈ 10.8 ms and
// generate_declining_12 ≈ 8.48 s — the recursive getter graph re-derived the whole predecessor chain per access
// (EXPONENTIAL in period count). The iterative forward pass (`Model::interest_pair`) brought these to ≈ 517 µs and
// ≈ 1.79 ms (−95% / −99.98%), restoring polynomial scaling; the 24-period shapes below became measurable then.

fn bench_generation(c: &mut Criterion) {
    for periods in [6usize, 12, 24] {
        c.bench_function(&format!("generate_declining_{periods}"), |b| {
            b.iter(|| black_box(fresh_model(black_box(periods))));
        });
    }
}

fn bench_second_disbursement(c: &mut Criterion) {
    for periods in [6usize, 24] {
        let bounds = monthly_periods(periods);
        let mid = bounds[periods / 2].0;
        c.bench_function(&format!("second_disbursement_{periods}"), |b| {
            b.iter_batched(
                || fresh_model(periods),
                |mut model| {
                    model.add_disbursement(mid, &m("500"));
                    black_box(model)
                },
                criterion::BatchSize::SmallInput,
            );
        });
    }
}

fn bench_payment_loop(c: &mut Criterion) {
    // Pay every period's scheduled principal+interest in sequence on its due date — the shape that used to make the
    // pay-all-upfront suite tests multi-minute at larger period counts.
    for periods in [6usize, 24] {
        let bounds = monthly_periods(periods);
        c.bench_function(&format!("pay_all_periods_{periods}"), |b| {
            b.iter_batched(
                || fresh_model(periods),
                |mut model| {
                    for (i, &(from, due)) in bounds.iter().enumerate() {
                        let principal = model.period_due_principal(i);
                        let interest = model.period_due_interest(i);
                        model.pay_principal(from, due, due, &principal);
                        model.pay_interest(from, due, due, &interest);
                    }
                    black_box(model)
                },
                criterion::BatchSize::SmallInput,
            );
        });
    }
}

criterion_group! {
    name = benches;
    config = Criterion::default().sample_size(10);
    targets = bench_generation, bench_second_disbursement, bench_payment_loop
}
criterion_main!(benches);
