//! Exact-decimal core — the Java-`BigDecimal` + `MathContext` equivalent operations (ADR-0005).
//!
//! Validated by the B1 spike (`plan/spikes/decimal`): 0 value mismatches vs Java over 14,520 ops. Division and
//! quantization use an explicit integer round-to-scale (NOT `bigdecimal`'s native `/`), and output is formatted as a
//! Java-`toPlainString` equivalent (no scientific notation; trailing zeros to scale; `-0` → `0`).
use bigdecimal::BigDecimal;
use bigdecimal::RoundingMode;
use bigdecimal::num_bigint::BigInt;

fn zero() -> BigInt {
    BigInt::from(0)
}
fn ten_pow(e: u32) -> BigInt {
    (0..e).fold(BigInt::from(1), |acc, _| acc * BigInt::from(10))
}
fn abs(x: &BigInt) -> BigInt {
    if *x < zero() { -x.clone() } else { x.clone() }
}
fn is_odd(x: &BigInt) -> bool {
    (x % BigInt::from(2)) != zero()
}

/// Correctly-rounded integer division `round(p / q)` under `mode` (Java `BigDecimal.divide` core).
fn round_div(p: &BigInt, q: &BigInt, mode: RoundingMode) -> BigInt {
    let quo = p / q; // truncates toward zero
    let rem = p - &(&quo * q); // sign of p
    if rem == zero() {
        return quo;
    }
    let neg = (*p < zero()) ^ (*q < zero());
    let r2 = abs(&rem) * BigInt::from(2);
    let qa = abs(q);
    let inc = match mode {
        RoundingMode::Down => false,
        RoundingMode::Up => true,
        RoundingMode::Ceiling => !neg,
        RoundingMode::Floor => neg,
        RoundingMode::HalfUp => r2 >= qa,
        RoundingMode::HalfDown => r2 > qa,
        RoundingMode::HalfEven => r2 > qa || (r2 == qa && is_odd(&quo)),
    };
    if inc {
        if neg {
            quo - BigInt::from(1)
        } else {
            quo + BigInt::from(1)
        }
    } else {
        quo
    }
}

/// Java `BigDecimal.setScale(scale, mode)`: rescale `v` to exactly `scale` fractional digits.
#[must_use]
pub fn set_scale(v: &BigDecimal, scale: i64, mode: RoundingMode) -> BigDecimal {
    let (u, exp) = v.as_bigint_and_exponent(); // value = u * 10^(-exp)
    if exp <= scale {
        let pad = u32::try_from(scale - exp).unwrap_or(0);
        BigDecimal::new(u * ten_pow(pad), scale)
    } else {
        let drop = u32::try_from(exp - scale).unwrap_or(0);
        BigDecimal::new(round_div(&u, &ten_pow(drop), mode), scale)
    }
}

/// Exact addition; result scale = max(scale(a), scale(b)) (Java semantics).
#[must_use]
pub fn add(a: &BigDecimal, b: &BigDecimal) -> BigDecimal {
    set_scale(
        &(a.clone() + b.clone()),
        scale_of(a).max(scale_of(b)),
        RoundingMode::HalfEven,
    )
}
/// Exact subtraction; result scale = max(scale(a), scale(b)).
#[must_use]
pub fn sub(a: &BigDecimal, b: &BigDecimal) -> BigDecimal {
    set_scale(
        &(a.clone() - b.clone()),
        scale_of(a).max(scale_of(b)),
        RoundingMode::HalfEven,
    )
}
/// Exact multiplication; result scale = scale(a) + scale(b) (Java semantics).
#[must_use]
pub fn mul(a: &BigDecimal, b: &BigDecimal) -> BigDecimal {
    set_scale(
        &(a.clone() * b.clone()),
        scale_of(a) + scale_of(b),
        RoundingMode::HalfEven,
    )
}
/// `a / b` correctly rounded to `scale` (Java `divide(b, scale, mode)`).
#[must_use]
pub fn div(a: &BigDecimal, b: &BigDecimal, scale: i64, mode: RoundingMode) -> BigDecimal {
    let (an, ae) = a.as_bigint_and_exponent();
    let (bn, be) = b.as_bigint_and_exponent();
    let e = be - ae + scale;
    let (mut p, mut q) = (an, bn);
    if e >= 0 {
        p *= ten_pow(u32::try_from(e).unwrap_or(0));
    } else {
        q *= ten_pow(u32::try_from(-e).unwrap_or(0));
    }
    BigDecimal::new(round_div(&p, &q, mode), scale)
}

/// Value-comparison signum: -1, 0, 1 (Java `compareTo`, scale-insensitive).
#[must_use]
pub fn compare(a: &BigDecimal, b: &BigDecimal) -> i32 {
    match a.cmp(b) {
        std::cmp::Ordering::Less => -1,
        std::cmp::Ordering::Equal => 0,
        std::cmp::Ordering::Greater => 1,
    }
}

fn scale_of(v: &BigDecimal) -> i64 {
    v.as_bigint_and_exponent().1
}

/// Java `BigDecimal.toPlainString()` equivalent: no scientific notation; exactly `scale` fractional digits; `-0`→`0`.
#[must_use]
pub fn to_plain_string(v: &BigDecimal) -> String {
    let (u, scale) = v.as_bigint_and_exponent();
    let neg = u < zero();
    let digits = abs(&u).to_string(); // "0" when zero
    if scale <= 0 {
        let mut s = digits;
        for _ in 0..(-scale) {
            s.push('0');
        }
        sign(neg, &u, s)
    } else {
        let scale_us = usize::try_from(scale).unwrap_or(0);
        let body = if digits.len() <= scale_us {
            let pad = "0".repeat(scale_us - digits.len());
            format!("0.{pad}{digits}")
        } else {
            let split = digits.len() - scale_us;
            format!("{}.{}", &digits[..split], &digits[split..])
        };
        sign(neg, &u, body)
    }
}

fn sign(neg: bool, u: &BigInt, body: String) -> String {
    if neg && *u != zero() {
        format!("-{body}")
    } else {
        body
    }
}
