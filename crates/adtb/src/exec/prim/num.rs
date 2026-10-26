use std::cmp::Ordering;

use super::super::{Error, Fault};
use super::{Val, Width, mistyped};

const DIGITS38: u128 = 10u128.pow(38);

/// A rounding mode of the prelude's `Rounding`, by constructor position.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rounding {
    HalfEven,
    HalfUp,
    HalfDown,
    Up,
    Down,
    Ceiling,
    Floor,
}

impl Rounding {
    pub fn of(v: &Val) -> Result<Rounding, Error> {
        Ok(match v {
            Val::Ctor(0, _) => Rounding::HalfEven,
            Val::Ctor(1, _) => Rounding::HalfUp,
            Val::Ctor(2, _) => Rounding::HalfDown,
            Val::Ctor(3, _) => Rounding::Up,
            Val::Ctor(4, _) => Rounding::Down,
            Val::Ctor(5, _) => Rounding::Ceiling,
            Val::Ctor(6, _) => Rounding::Floor,
            other => return Err(mistyped("a Rounding", other)),
        })
    }

    /// Returns whether an inexact quotient's magnitude moves away from zero, given its sign, whether
    /// its truncated magnitude is odd, and how its remainder compares to half the divisor.
    fn away(self, neg: bool, odd: bool, half: Ordering) -> bool {
        match self {
            Rounding::Down => false,
            Rounding::Up => true,
            Rounding::Ceiling => !neg,
            Rounding::Floor => neg,
            Rounding::HalfUp => half != Ordering::Less,
            Rounding::HalfDown => half == Ordering::Greater,
            Rounding::HalfEven => match half {
                Ordering::Less => false,
                Ordering::Greater => true,
                Ordering::Equal => odd,
            },
        }
    }

    /// Returns `n / d` rounded, `d > 0`.
    pub fn div(self, n: i128, d: i128) -> i128 {
        let (q, r) = (n / d, n % d);
        if r == 0 {
            return q;
        }
        let half = (r.unsigned_abs() * 2).cmp(&d.unsigned_abs());
        if self.away(n < 0, q % 2 != 0, half) {
            if n < 0 { q - 1 } else { q + 1 }
        } else {
            q
        }
    }

    /// Returns `x` rounded to an integer.
    fn float(self, x: f64) -> f64 {
        let t = x.trunc();
        let frac = (x - t).abs();
        if frac == 0.0 {
            return t;
        }
        let half = frac.partial_cmp(&0.5).expect("a finite fraction");
        let odd = t % 2.0 != 0.0;
        if self.away(x < 0.0, odd, half) {
            t + x.signum()
        } else {
            t
        }
    }
}

/// An unsigned integer of any size, little-endian limbs.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Big(Vec<u64>);

impl Big {
    fn of(v: u128) -> Big {
        Big(vec![v as u64, (v >> 64) as u64]).trim()
    }

    fn trim(mut self) -> Big {
        while self.0.last() == Some(&0) {
            self.0.pop();
        }
        self
    }

    fn times_pow10(mut self, k: u32) -> Big {
        for _ in 0..k {
            let mut carry = 0u128;
            for limb in &mut self.0 {
                let p = u128::from(*limb) * 10 + carry;
                *limb = p as u64;
                carry = p >> 64;
            }
            if carry != 0 {
                self.0.push(carry as u64);
            }
        }
        self
    }

    fn bits(&self) -> usize {
        self.0
            .last()
            .map_or(0, |l| 64 * self.0.len() - l.leading_zeros() as usize)
    }

    fn bit(&self, i: usize) -> bool {
        self.0.get(i / 64).is_some_and(|l| l >> (i % 64) & 1 == 1)
    }

    fn shl1(&mut self, low: bool) {
        let mut carry = u64::from(low);
        for limb in &mut self.0 {
            let next = *limb >> 63;
            *limb = *limb << 1 | carry;
            carry = next;
        }
        if carry != 0 {
            self.0.push(carry);
        }
    }

    /// Subtracts `b`, `b <= self`.
    fn sub(&mut self, b: &Big) {
        let mut borrow = 0u64;
        for i in 0..self.0.len() {
            let (d, o1) = self.0[i].overflowing_sub(b.0.get(i).copied().unwrap_or(0));
            let (d, o2) = d.overflowing_sub(borrow);
            self.0[i] = d;
            borrow = u64::from(o1 || o2);
        }
        let t = std::mem::take(&mut self.0);
        *self = Big(t).trim();
    }

    fn cmp(&self, b: &Big) -> Ordering {
        self.0
            .len()
            .cmp(&b.0.len())
            .then_with(|| self.0.iter().rev().cmp(b.0.iter().rev()))
    }

    /// Returns the quotient and remainder of `self / d`, `d > 0`.
    fn divmod(&self, d: &Big) -> (Big, Big) {
        let mut q = Big(vec![0; self.0.len()]);
        let mut r = Big(Vec::new());
        for i in (0..self.bits()).rev() {
            r.shl1(self.bit(i));
            if r.cmp(d) != Ordering::Less {
                r.sub(d);
                q.0[i / 64] |= 1 << (i % 64);
            }
        }
        (q.trim(), r)
    }

    fn to_u128(&self) -> Option<u128> {
        match self.0.as_slice() {
            [] => Some(0),
            [a] => Some(u128::from(*a)),
            [a, b] => Some(u128::from(*a) | u128::from(*b) << 64),
            _ => None,
        }
    }
}

/// Returns decimal `a` at scale `s` divided by `b` at scale `t`, rounded to scale `u`.
pub fn decimal_div(a: i128, s: u8, b: i128, t: u8, u: u8, r: Rounding) -> Result<i128, Fault> {
    if b == 0 {
        return Err(Fault::DivisionByZero);
    }
    let e = i32::from(t) + i32::from(u) - i32::from(s);
    let (mut n, mut d) = (Big::of(a.unsigned_abs()), Big::of(b.unsigned_abs()));
    if e >= 0 {
        n = n.times_pow10(e as u32);
    } else {
        d = d.times_pow10(e.unsigned_abs());
    }
    let (q, rem) = n.divmod(&d);
    let mut q = q
        .to_u128()
        .filter(|q| *q < DIGITS38)
        .ok_or(Fault::ArithmeticOverflow)?;
    let neg = (a < 0) != (b < 0);
    if !rem.0.is_empty() {
        let mut twice = rem;
        twice.shl1(false);
        if r.away(neg, q % 2 == 1, twice.cmp(&d)) {
            q += 1;
        }
    }
    if q >= DIGITS38 {
        return Err(Fault::ArithmeticOverflow);
    }
    let q = q as i128;
    Ok(if neg { -q } else { q })
}

/// Returns float `x` rounded to an integer of width `w`.
pub fn float_to_int(x: f64, r: Rounding, w: Width) -> Result<i64, Fault> {
    if !x.is_finite() {
        return Err(Fault::NotFinite);
    }
    let v = r.float(x);
    if !(-(2f64.powi(63))..2f64.powi(63)).contains(&v) {
        return Err(Fault::OutOfRange);
    }
    w.fit(i128::from(v as i64)).ok_or(Fault::OutOfRange)
}

/// Returns float `x` rounded to a decimal of scale `s`, as its unscaled digits.
pub fn float_to_decimal(x: f64, s: u8, r: Rounding) -> Result<i128, Fault> {
    if !x.is_finite() {
        return Err(Fault::NotFinite);
    }
    let exact = format!("{:.1100}", x.abs());
    let (int, frac) = exact
        .split_once('.')
        .expect("a fixed-point form has a point");
    let (kept, rest) = frac.split_at(usize::from(s));
    let digits = format!("{int}{kept}");
    let digits = digits.trim_start_matches('0');
    if digits.len() > 38 {
        return Err(Fault::OutOfRange);
    }
    let mut q: u128 = if digits.is_empty() {
        0
    } else {
        digits.parse().expect("at most 38 digits")
    };
    let rest = rest.trim_end_matches('0');
    if !rest.is_empty() {
        let half = match rest.as_bytes()[0].cmp(&b'5') {
            Ordering::Equal if rest.len() > 1 => Ordering::Greater,
            o => o,
        };
        if r.away(x < 0.0, q % 2 == 1, half) {
            q += 1;
        }
    }
    if q >= DIGITS38 {
        return Err(Fault::OutOfRange);
    }
    let q = q as i128;
    Ok(if x < 0.0 { -q } else { q })
}

/// Returns the digits of decimal `d` at scale `s`, with exactly `s` fractional digits.
pub fn decimal_text(d: i128, s: u8) -> String {
    let digits = d.unsigned_abs().to_string();
    let s = usize::from(s);
    let digits = format!("{digits:0>width$}", width = s + 1);
    let (int, frac) = digits.split_at(digits.len() - s);
    let sign = if d < 0 { "-" } else { "" };
    if s == 0 {
        format!("{sign}{int}")
    } else {
        format!("{sign}{int}.{frac}")
    }
}

/// Returns the shortest decimal form of `x` that reads back as `x`, or `NaN`, `inf` or `-inf`.
pub fn f64_text(x: f64) -> String {
    format!("{x:?}")
}

pub fn f32_text(x: f32) -> String {
    format!("{x:?}")
}
