use std::cmp::Ordering;

/// A signed integer of up to 256 bits of magnitude, enough to rescale and add 38-digit decimals
/// exactly.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Wide {
    neg: bool,
    /// Little-endian limbs.
    mag: [u64; 4],
}

const DIGITS38: u128 = 10u128.pow(38);

impl Wide {
    pub const ZERO: Wide = Wide {
        neg: false,
        mag: [0; 4],
    };

    pub fn from_i128(v: i128) -> Wide {
        let m = v.unsigned_abs();
        Wide {
            neg: v < 0,
            mag: [m as u64, (m >> 64) as u64, 0, 0],
        }
    }

    fn is_zero(&self) -> bool {
        self.mag == [0; 4]
    }

    fn normal(mut self) -> Wide {
        if self.is_zero() {
            self.neg = false;
        }
        self
    }

    /// Returns `self * 10^k`, or `None` past 256 bits.
    pub fn scale(self, k: u32) -> Option<Wide> {
        let mut out = self;
        for _ in 0..k {
            let mut carry = 0u128;
            for limb in &mut out.mag {
                let p = u128::from(*limb) * 10 + carry;
                *limb = p as u64;
                carry = p >> 64;
            }
            if carry != 0 {
                return None;
            }
        }
        Some(out)
    }

    fn cmp_mag(a: &[u64; 4], b: &[u64; 4]) -> Ordering {
        a.iter().rev().cmp(b.iter().rev())
    }

    fn add_mag(a: &[u64; 4], b: &[u64; 4]) -> Option<[u64; 4]> {
        let mut out = [0; 4];
        let mut carry = 0u128;
        for i in 0..4 {
            let s = u128::from(a[i]) + u128::from(b[i]) + carry;
            out[i] = s as u64;
            carry = s >> 64;
        }
        (carry == 0).then_some(out)
    }

    /// Returns `a - b` for `a >= b`.
    fn sub_mag(a: &[u64; 4], b: &[u64; 4]) -> [u64; 4] {
        let mut out = [0; 4];
        let mut borrow = 0i128;
        for i in 0..4 {
            let mut d = i128::from(a[i]) - i128::from(b[i]) - borrow;
            borrow = 0;
            if d < 0 {
                d += 1 << 64;
                borrow = 1;
            }
            out[i] = d as u64;
        }
        out
    }

    pub fn add(self, other: Wide) -> Option<Wide> {
        if self.neg == other.neg {
            let mag = Wide::add_mag(&self.mag, &other.mag)?;
            return Some(Wide { neg: self.neg, mag }.normal());
        }
        let out = match Wide::cmp_mag(&self.mag, &other.mag) {
            Ordering::Less => Wide {
                neg: other.neg,
                mag: Wide::sub_mag(&other.mag, &self.mag),
            },
            _ => Wide {
                neg: self.neg,
                mag: Wide::sub_mag(&self.mag, &other.mag),
            },
        };
        Some(out.normal())
    }

    pub fn neg(self) -> Wide {
        Wide {
            neg: !self.neg,
            mag: self.mag,
        }
        .normal()
    }

    /// Returns the value when it fits an `i128`.
    pub fn to_i128(self) -> Option<i128> {
        if self.mag[2] != 0 || self.mag[3] != 0 {
            return None;
        }
        let m = u128::from(self.mag[0]) | u128::from(self.mag[1]) << 64;
        if self.neg {
            0i128.checked_sub_unsigned(m)
        } else {
            i128::try_from(m).ok()
        }
    }

    /// Returns the value when it has at most 38 decimal digits.
    pub fn digits38(self) -> Option<i128> {
        let v = self.to_i128()?;
        (v.unsigned_abs() < DIGITS38).then_some(v)
    }
}

impl PartialOrd for Wide {
    fn partial_cmp(&self, other: &Wide) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Wide {
    fn cmp(&self, other: &Wide) -> Ordering {
        match (self.neg, other.neg) {
            (false, true) => Ordering::Greater,
            (true, false) => Ordering::Less,
            (false, false) => Wide::cmp_mag(&self.mag, &other.mag),
            (true, true) => Wide::cmp_mag(&other.mag, &self.mag),
        }
    }
}

/// Returns `a * 10^(to - from)` exactly, `to >= from`.
pub fn rescaled(a: i128, from: u8, to: u8) -> Wide {
    Wide::from_i128(a)
        .scale(u32::from(to - from))
        .expect("a 38-digit decimal rescaled by at most 38 digits fits 256 bits")
}

/// Returns the order of decimals `a` at scale `s` and `b` at scale `t`.
pub fn cmp_decimal(a: i128, s: u8, b: i128, t: u8) -> Ordering {
    let m = s.max(t);
    rescaled(a, s, m).cmp(&rescaled(b, t, m))
}
