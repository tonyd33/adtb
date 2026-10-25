use std::cmp::Ordering;

use adtb_lang::types::Base;

use crate::big::Big;
use crate::eval::Fault;

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
    /// Returns the mode of a constructor of the prelude sum `Rounding`.
    pub fn from_ctor(c: u32) -> Rounding {
        match c {
            0 => Rounding::HalfEven,
            1 => Rounding::HalfUp,
            2 => Rounding::HalfDown,
            3 => Rounding::Up,
            4 => Rounding::Down,
            5 => Rounding::Ceiling,
            6 => Rounding::Floor,
            _ => panic!("`Rounding` has seven constructors, given {c}"),
        }
    }
}

/// Returns `n / d` rounded by `r`, for a nonzero `d`.
pub fn round_div(n: &Big, d: &Big, r: Rounding) -> Big {
    let (q, rem) = n.divrem(d);
    if rem.is_zero() {
        return q;
    }
    let neg = n.is_neg() != d.is_neg();
    let half = rem.abs().shl1().cmp(&d.abs());
    let away = match r {
        Rounding::Up => true,
        Rounding::Down => false,
        Rounding::Ceiling => !neg,
        Rounding::Floor => neg,
        Rounding::HalfUp => half != Ordering::Less,
        Rounding::HalfDown => half == Ordering::Greater,
        Rounding::HalfEven => half == Ordering::Greater || half == Ordering::Equal && q.is_odd(),
    };
    if !away {
        return q;
    }
    q.add(&Big::from_i128(if neg { -1 } else { 1 }))
}

/// Returns the unscaled value when it has at most 38 digits.
pub fn digits38(b: &Big) -> Option<i128> {
    if b.abs() < Big::pow10(38) {
        b.to_i128()
    } else {
        None
    }
}

fn scaled(a: i128, from: u8, to: u8) -> Big {
    Big::from_i128(a).mul(&Big::pow10((to - from) as u32))
}

pub fn dec_add(a: i128, s: u8, b: i128, t: u8, sub: bool) -> Result<(i128, u8), Fault> {
    let u = s.max(t);
    let (x, y) = (scaled(a, s, u), scaled(b, t, u));
    let r = if sub { x.sub(&y) } else { x.add(&y) };
    digits38(&r)
        .map(|v| (v, u))
        .ok_or(Fault::ArithmeticOverflow)
}

pub fn dec_mul(a: i128, s: u8, b: i128, t: u8) -> Result<(i128, u8), Fault> {
    let r = Big::from_i128(a).mul(&Big::from_i128(b));
    digits38(&r)
        .map(|v| (v, s + t))
        .ok_or(Fault::ArithmeticOverflow)
}

/// `a / b` rounded to scale `u`.
pub fn dec_div(a: i128, s: u8, b: i128, t: u8, u: u8, r: Rounding) -> Result<i128, Fault> {
    if b == 0 {
        return Err(Fault::DivisionByZero);
    }
    let n = Big::from_i128(a).mul(&Big::pow10(u as u32 + t as u32));
    let d = Big::from_i128(b).mul(&Big::pow10(s as u32));
    digits38(&round_div(&n, &d, r)).ok_or(Fault::ArithmeticOverflow)
}

/// `a` at scale `s` rounded to scale `u <= s`.
pub fn dec_round(a: i128, s: u8, u: u8, r: Rounding) -> Big {
    round_div(&Big::from_i128(a), &Big::pow10((s - u) as u32), r)
}

pub fn dec_rescale(a: i128, s: u8, u: u8) -> Result<i128, Fault> {
    digits38(&scaled(a, s, u)).ok_or(Fault::ArithmeticOverflow)
}

/// Returns the exact value of `f` times `10^s`, rounded to an integer by `r`.
pub fn float_scaled(f: f64, s: u8, r: Rounding) -> Result<Big, Fault> {
    if !f.is_finite() {
        return Err(Fault::NotFinite);
    }
    if f == 0.0 {
        return Ok(Big::zero());
    }
    let bits = f.to_bits();
    let exp = ((bits >> 52) & 0x7ff) as i32;
    let frac = bits & ((1 << 52) - 1);
    let (m, e) = if exp == 0 {
        (frac, -1074)
    } else {
        (frac | 1 << 52, exp - 1075)
    };
    let m = Big::from_u64(m);
    let m = if f < 0.0 { m.neg() } else { m };
    let n = m.mul(&Big::pow10(s as u32));
    if e >= 0 {
        Ok(n.mul(&Big::pow2(e as u32)))
    } else {
        Ok(round_div(&n, &Big::pow2((-e) as u32), r))
    }
}

pub fn int_range(b: Base) -> Option<(i64, i64)> {
    Some(match b {
        Base::I8 => (i8::MIN as i64, i8::MAX as i64),
        Base::I16 => (i16::MIN as i64, i16::MAX as i64),
        Base::I32 => (i32::MIN as i64, i32::MAX as i64),
        Base::I64 => (i64::MIN, i64::MAX),
        _ => return None,
    })
}

/// Returns `v` when it lies in the range of integer type `b`.
pub fn in_range(v: i128, b: Base) -> Option<i64> {
    let (lo, hi) = int_range(b)?;
    (lo as i128 <= v && v <= hi as i128).then_some(v as i64)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(n: i128, d: i128, m: Rounding) -> i128 {
        round_div(&Big::from_i128(n), &Big::from_i128(d), m)
            .to_i128()
            .unwrap()
    }

    #[test]
    fn rounding_modes() {
        use Rounding::*;
        assert_eq!(2, r(5, 2, HalfEven));
        assert_eq!(4, r(7, 2, HalfEven));
        assert_eq!(-2, r(-5, 2, HalfEven));
        assert_eq!(3, r(5, 2, HalfUp));
        assert_eq!(-3, r(-5, 2, HalfUp));
        assert_eq!(2, r(5, 2, HalfDown));
        assert_eq!(3, r(7, 3, Up));
        assert_eq!(-3, r(-7, 3, Up));
        assert_eq!(2, r(7, 3, Down));
        assert_eq!(3, r(7, 3, Ceiling));
        assert_eq!(-2, r(-7, 3, Ceiling));
        assert_eq!(-3, r(-7, 3, Floor));
        assert_eq!(2, r(7, 3, HalfEven));
    }

    #[test]
    fn floats_scale_exactly() {
        let b = float_scaled(0.1, 20, Rounding::Down).unwrap();
        assert_eq!(Some(10000000000000000555), b.to_i128());
        let b = float_scaled(-2.5, 0, Rounding::HalfEven).unwrap();
        assert_eq!(Some(-2), b.to_i128());
        assert_eq!(
            Err(Fault::NotFinite),
            float_scaled(f64::NAN, 0, Rounding::Down)
        );
    }

    #[test]
    fn decimal_division_rounds_at_the_named_scale() {
        assert_eq!(Ok(333), dec_div(100, 2, 300, 2, 3, Rounding::HalfEven));
        assert_eq!(Ok(667), dec_div(200, 2, 300, 2, 3, Rounding::HalfEven));
        assert_eq!(
            Err(Fault::DivisionByZero),
            dec_div(1, 0, 0, 0, 0, Rounding::Up)
        );
        assert_eq!(
            Err(Fault::ArithmeticOverflow),
            dec_add(10i128.pow(37) * 9, 0, 10i128.pow(37) * 9, 0, false)
        );
    }
}
