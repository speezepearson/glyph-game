//! One torus axis as a `u32` quotient — the seam-free coordinate type.
//!
//! `Coord`'s internal `u32` value is private. The range `0..=u32::MAX`
//! maps to `[0, 1)` on the circle; wrapping is u32's native arithmetic,
//! so the torus has no privileged "0/1 boundary" anywhere in the code.
//!
//! Only translation-invariant relative operations are exposed:
//! [`Coord::signed_diff_to`] and [`Coord::shifted`]. Applying the same
//! shift to every Coord in the inputs of any public method gives
//! identical outputs — bit-for-bit, no f32 noise. This is the property
//! the type is designed to enforce, fuzz-tested below.

/// 2^32 as f32 — exact, since 2^32 is a power of two within f32's
/// dynamic range.
const U_SCALE: f32 = 4_294_967_296.0;

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct Coord {
    u: u32,
}

impl Coord {
    /// Wrap any `f32` into a Coord. The input boundary — converts e.g.
    /// the user's mouse position to a torus position.
    pub fn from_f32(f: f32) -> Self {
        let frac = f.rem_euclid(1.0);
        Self {
            u: (frac * U_SCALE) as u32,
        }
    }

    /// Shift this coord on the torus by an arbitrary signed f32 delta.
    /// Equivariant: `c.shifted(δ)` is `c` moved by `δ`, and shifts
    /// compose: `c.shifted(a).shifted(b) == c.shifted(a + b)`.
    pub fn shifted(self, delta: f32) -> Self {
        let frac = delta.rem_euclid(1.0);
        let offset = (frac * U_SCALE) as u32;
        Self {
            u: self.u.wrapping_add(offset),
        }
    }

    /// Shortest signed difference `other - self`, expressed as an f32
    /// in `[-0.5, 0.5)`. Exactly translation-invariant: shifting
    /// `self` and `other` by the same delta gives the same f32
    /// answer bit-for-bit.
    pub fn signed_diff_to(self, other: Coord) -> f32 {
        let raw = other.u.wrapping_sub(self.u) as i32;
        raw as f32 / U_SCALE
    }
}

#[cfg(test)]
impl quickcheck::Arbitrary for Coord {
    fn arbitrary(g: &mut quickcheck::Gen) -> Self {
        // Uniform over the full torus axis — u32::arbitrary covers
        // the whole circle.
        Self {
            u: u32::arbitrary(g),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use quickcheck::Arbitrary;
    use quickcheck_macros::quickcheck;

    /// Bounded-range f32 wrapper for shift deltas, so quickcheck
    /// produces interesting shifts (~ [-2, 2)) and shrinks toward 0.
    #[derive(Copy, Clone, Debug)]
    struct ShiftDelta(f32);
    impl Arbitrary for ShiftDelta {
        fn arbitrary(g: &mut quickcheck::Gen) -> Self {
            ShiftDelta((i16::arbitrary(g) as f32) / 16384.0)
        }
        fn shrink(&self) -> Box<dyn Iterator<Item = Self>> {
            let i = (self.0 * 16384.0) as i16;
            Box::new(i.shrink().map(|i| ShiftDelta((i as f32) / 16384.0)))
        }
    }

    /// `signed_diff_to` is bit-equal under uniform translation of both
    /// inputs.
    #[quickcheck]
    fn prop_signed_diff_is_translation_invariant(
        a: Coord,
        b: Coord,
        shift: ShiftDelta,
    ) -> bool {
        let s = shift.0;
        a.signed_diff_to(b).to_bits()
            == a.shifted(s).signed_diff_to(b.shifted(s)).to_bits()
    }

    /// Sequential shifts add: `c.shifted(a).shifted(b)` agrees with
    /// `c.shifted(a + b)` modulo the u32 grid resolution (~1e-9 in
    /// f32 distance terms; tolerance bumped for f32 sum noise).
    #[quickcheck]
    fn prop_shift_composes(c: Coord, a: ShiftDelta, b: ShiftDelta) -> bool {
        let lhs = c.shifted(a.0).shifted(b.0);
        let rhs = c.shifted(a.0 + b.0);
        lhs.signed_diff_to(rhs).abs() < 1e-6
    }
}
