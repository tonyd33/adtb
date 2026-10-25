use std::cmp::Ordering;

/// A signed integer of any size.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Big {
    neg: bool,
    /// Little-endian limbs with no trailing zero; zero is empty and never negative.
    mag: Vec<u32>,
}

fn trim(mut v: Vec<u32>) -> Vec<u32> {
    while v.last() == Some(&0) {
        v.pop();
    }
    v
}

fn cmp_mag(a: &[u32], b: &[u32]) -> Ordering {
    a.len()
        .cmp(&b.len())
        .then_with(|| a.iter().rev().cmp(b.iter().rev()))
}

fn add_mag(a: &[u32], b: &[u32]) -> Vec<u32> {
    let mut out = Vec::with_capacity(a.len().max(b.len()) + 1);
    let mut carry = 0u64;
    for i in 0..a.len().max(b.len()) {
        let s = *a.get(i).unwrap_or(&0) as u64 + *b.get(i).unwrap_or(&0) as u64 + carry;
        out.push(s as u32);
        carry = s >> 32;
    }
    out.push(carry as u32);
    trim(out)
}

/// Returns `a - b` for `a >= b`.
fn sub_mag(a: &[u32], b: &[u32]) -> Vec<u32> {
    let mut out = Vec::with_capacity(a.len());
    let mut borrow = 0i64;
    for (i, &x) in a.iter().enumerate() {
        let mut d = x as i64 - *b.get(i).unwrap_or(&0) as i64 - borrow;
        borrow = 0;
        if d < 0 {
            d += 1 << 32;
            borrow = 1;
        }
        out.push(d as u32);
    }
    trim(out)
}

fn mul_mag(a: &[u32], b: &[u32]) -> Vec<u32> {
    let mut out = vec![0u32; a.len() + b.len()];
    for (i, &x) in a.iter().enumerate() {
        let mut carry = 0u64;
        for (j, &y) in b.iter().enumerate() {
            let t = out[i + j] as u64 + x as u64 * y as u64 + carry;
            out[i + j] = t as u32;
            carry = t >> 32;
        }
        out[i + b.len()] = carry as u32;
    }
    trim(out)
}

fn bit(a: &[u32], i: usize) -> bool {
    a[i / 32] >> (i % 32) & 1 == 1
}

fn shl1_or(a: &mut Vec<u32>, low: bool) {
    let mut carry = low as u32;
    for limb in a.iter_mut() {
        let next = *limb >> 31;
        *limb = *limb << 1 | carry;
        carry = next;
    }
    if carry != 0 {
        a.push(carry);
    }
}

/// Returns the quotient and remainder of `a / b` for a nonzero `b`.
fn divrem_mag(a: &[u32], b: &[u32]) -> (Vec<u32>, Vec<u32>) {
    let mut q = vec![0u32; a.len()];
    let mut r: Vec<u32> = Vec::new();
    for i in (0..a.len() * 32).rev() {
        shl1_or(&mut r, bit(a, i));
        r = trim(r);
        if cmp_mag(&r, b) != Ordering::Less {
            r = sub_mag(&r, b);
            q[i / 32] |= 1 << (i % 32);
        }
    }
    (trim(q), r)
}

impl Big {
    pub fn zero() -> Big {
        Big {
            neg: false,
            mag: Vec::new(),
        }
    }

    fn make(neg: bool, mag: Vec<u32>) -> Big {
        let mag = trim(mag);
        Big {
            neg: neg && !mag.is_empty(),
            mag,
        }
    }

    pub fn from_i128(v: i128) -> Big {
        let mut m = v.unsigned_abs();
        let mut mag = Vec::new();
        while m != 0 {
            mag.push(m as u32);
            m >>= 32;
        }
        Big::make(v < 0, mag)
    }

    pub fn from_u64(v: u64) -> Big {
        Big::make(false, vec![v as u32, (v >> 32) as u32])
    }

    pub fn to_i128(&self) -> Option<i128> {
        if self.mag.len() > 4 {
            return None;
        }
        let mut m: u128 = 0;
        for &limb in self.mag.iter().rev() {
            m = m << 32 | limb as u128;
        }
        if self.neg {
            if m <= i128::MAX as u128 + 1 {
                Some((m as i128).wrapping_neg())
            } else {
                None
            }
        } else {
            i128::try_from(m).ok()
        }
    }

    pub fn pow10(n: u32) -> Big {
        let mut out = Big::from_i128(1);
        let ten = Big::from_i128(10);
        for _ in 0..n {
            out = out.mul(&ten);
        }
        out
    }

    pub fn pow2(n: u32) -> Big {
        let mut mag = vec![0u32; n as usize / 32 + 1];
        mag[n as usize / 32] = 1 << (n % 32);
        Big::make(false, mag)
    }

    pub fn is_zero(&self) -> bool {
        self.mag.is_empty()
    }

    pub fn is_neg(&self) -> bool {
        self.neg
    }

    pub fn is_odd(&self) -> bool {
        self.mag.first().is_some_and(|l| l & 1 == 1)
    }

    pub fn neg(&self) -> Big {
        Big::make(!self.neg, self.mag.clone())
    }

    pub fn abs(&self) -> Big {
        Big::make(false, self.mag.clone())
    }

    pub fn add(&self, o: &Big) -> Big {
        if self.neg == o.neg {
            return Big::make(self.neg, add_mag(&self.mag, &o.mag));
        }
        match cmp_mag(&self.mag, &o.mag) {
            Ordering::Less => Big::make(o.neg, sub_mag(&o.mag, &self.mag)),
            _ => Big::make(self.neg, sub_mag(&self.mag, &o.mag)),
        }
    }

    pub fn sub(&self, o: &Big) -> Big {
        self.add(&o.neg())
    }

    pub fn mul(&self, o: &Big) -> Big {
        Big::make(self.neg != o.neg, mul_mag(&self.mag, &o.mag))
    }

    /// Returns the quotient truncated toward zero and the remainder, which has the sign of `self`.
    pub fn divrem(&self, o: &Big) -> (Big, Big) {
        let (q, r) = divrem_mag(&self.mag, &o.mag);
        (Big::make(self.neg != o.neg, q), Big::make(self.neg, r))
    }

    pub fn shl1(&self) -> Big {
        let mut m = self.mag.clone();
        shl1_or(&mut m, false);
        Big::make(self.neg, m)
    }
}

impl Ord for Big {
    fn cmp(&self, o: &Big) -> Ordering {
        match (self.neg, o.neg) {
            (false, true) => Ordering::Greater,
            (true, false) => Ordering::Less,
            (false, false) => cmp_mag(&self.mag, &o.mag),
            (true, true) => cmp_mag(&o.mag, &self.mag),
        }
    }
}

impl PartialOrd for Big {
    fn partial_cmp(&self, o: &Big) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arithmetic_matches_i128() {
        let xs = [
            0i128,
            1,
            -1,
            7,
            -7,
            1 << 40,
            -(1 << 70),
            i64::MAX as i128,
            12345678901234567890,
        ];
        for &a in &xs {
            for &b in &xs {
                let (x, y) = (Big::from_i128(a), Big::from_i128(b));
                assert_eq!(Some(a + b), x.add(&y).to_i128());
                assert_eq!(Some(a - b), x.sub(&y).to_i128());
                assert_eq!(a.cmp(&b), x.cmp(&y));
                if let Some(p) = a.checked_mul(b) {
                    assert_eq!(Some(p), x.mul(&y).to_i128());
                }
                if b != 0 {
                    let (q, r) = x.divrem(&y);
                    assert_eq!(Some(a / b), q.to_i128());
                    assert_eq!(Some(a % b), r.to_i128());
                }
            }
        }
    }

    #[test]
    fn extremes_round_trip() {
        assert_eq!(Some(i128::MIN), Big::from_i128(i128::MIN).to_i128());
        assert_eq!(Some(i128::MAX), Big::from_i128(i128::MAX).to_i128());
        assert_eq!(
            None,
            Big::from_i128(i128::MAX).add(&Big::from_i128(1)).to_i128()
        );
        assert_eq!(
            Big::pow10(40).divrem(&Big::pow10(38)).0,
            Big::from_i128(100)
        );
    }
}
