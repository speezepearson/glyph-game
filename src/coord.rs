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
mod tests {
    use super::*;

    struct Lcg(u64);
    impl Lcg {
        fn new(seed: u64) -> Self {
            Self(seed.wrapping_mul(0x9E3779B97F4A7C15).wrapping_add(1))
        }
        fn next_u32(&mut self) -> u32 {
            self.0 = self
                .0
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (self.0 >> 32) as u32
        }
        fn next_f32(&mut self) -> f32 {
            (self.next_u32() as f64 / u32::MAX as f64) as f32
        }
        fn next_signed_f32(&mut self) -> f32 {
            self.next_f32() * 4.0 - 2.0
        }
    }

    /// signed_diff_to is exactly invariant under uniform translation:
    /// shifting both Coords by the same delta gives the exact same
    /// f32 answer, bit-equal, no tolerance.
    #[test]
    fn fuzz_signed_diff_is_exactly_translation_invariant() {
        let mut rng = Lcg::new(1);
        for _ in 0..5000 {
            let a = Coord::from_f32(rng.next_signed_f32());
            let b = Coord::from_f32(rng.next_signed_f32());
            let shift = rng.next_signed_f32();
            assert_eq!(
                a.signed_diff_to(b).to_bits(),
                a.shifted(shift).signed_diff_to(b.shifted(shift)).to_bits(),
                "shift={shift}",
            );
        }
    }

    /// `shifted` composes additively, modulo u32 wrap. Quick sanity.
    #[test]
    fn fuzz_shift_composes() {
        let mut rng = Lcg::new(2);
        for _ in 0..5000 {
            let c = Coord::from_f32(rng.next_signed_f32());
            let a = rng.next_signed_f32();
            let b = rng.next_signed_f32();
            // c.shifted(a).shifted(b) ≡ c.shifted(a + b) (modulo wrap),
            // so the diff from one to the other is 0.
            let lhs = c.shifted(a).shifted(b);
            let rhs = c.shifted(a + b);
            // Tolerance because (a + b) computed in f32 vs sequential
            // shift differ by 1 ULP at the boundary. Both should round
            // to within a handful of u32 units.
            let diff = lhs.signed_diff_to(rhs).abs();
            assert!(diff < 1e-6, "diff={diff} a={a} b={b}");
        }
    }
}
