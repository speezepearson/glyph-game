//! Points, vectors, and line segments on the flat 2-torus T² = ℝ² / ℤ².
//!
//! Every public method on `TorusPoint` is translation-invariant:
//! applying the same shift to every TorusPoint in the inputs gives
//! identical outputs (bit-equal for most ops; tiny f32 noise for the
//! few that round-trip a sum). This is fuzz-tested at the bottom of
//! this module, exhaustively for every public method.
//!
//! Types:
//! - [`TorusPoint`] = `(Coord, Coord)`: a point on T².
//! - [`TorusVec`]: a displacement in the universal cover ℝ². NOT
//!   reduced modulo the lattice, so different vectors that translate
//!   to the same place represent different homotopy classes.
//! - [`TorusSegment`]: `(start, disp)` — a straight line in the
//!   universal cover, projected to the torus.

use crate::coord::Coord;

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct TorusPoint {
    pub x: Coord,
    pub y: Coord,
}

impl TorusPoint {
    /// Construct from any `(x, y)` in ℝ². Wraps into the torus.
    pub fn new(x: f32, y: f32) -> Self {
        Self {
            x: Coord::from_f32(x),
            y: Coord::from_f32(y),
        }
    }

    /// Translate by a `TorusVec` displacement. Equivariant: shifts
    /// compose with translations.
    pub fn translate(self, v: TorusVec) -> Self {
        Self {
            x: self.x.shifted(v.dx),
            y: self.y.shifted(v.dy),
        }
    }

    /// The *shortest* displacement from `self` to `other`. Each
    /// component lies in `[-½, ½)`. Translation-invariant.
    pub fn shortest_to(self, other: TorusPoint) -> TorusVec {
        TorusVec {
            dx: self.x.signed_diff_to(other.x),
            dy: self.y.signed_diff_to(other.y),
        }
    }

    /// Toroidal Euclidean distance. Translation-invariant.
    pub fn distance_to(self, other: TorusPoint) -> f32 {
        let v = self.shortest_to(other);
        (v.dx * v.dx + v.dy * v.dy).sqrt()
    }
}

#[derive(Copy, Clone, Debug, PartialEq)]
pub struct TorusVec {
    pub dx: f32,
    pub dy: f32,
}

impl TorusVec {
    pub fn new(dx: f32, dy: f32) -> Self {
        Self { dx, dy }
    }

    pub fn zero() -> Self {
        Self { dx: 0.0, dy: 0.0 }
    }

    pub fn length(self) -> f32 {
        (self.dx * self.dx + self.dy * self.dy).sqrt()
    }
}

impl std::ops::Add for TorusVec {
    type Output = TorusVec;
    fn add(self, other: TorusVec) -> TorusVec {
        TorusVec::new(self.dx + other.dx, self.dy + other.dy)
    }
}

#[derive(Copy, Clone, Debug)]
pub struct TorusSegment {
    pub start: TorusPoint,
    pub disp: TorusVec,
}

impl TorusSegment {
    /// The (canonical-form) endpoint of the segment on the torus.
    pub fn end(self) -> TorusPoint {
        self.start.translate(self.disp)
    }

    /// Iterate the 3×3 block of integer translates of this segment in
    /// the universal cover, expressed in a frame anchored at
    /// `anchor`. Each item is `((lift_offset_x, lift_offset_y),
    /// ((start_x, start_y), (end_x, end_y)))` — the integer lift
    /// indices for self-pair filtering, followed by the lifted
    /// (start, end) pair in ℝ² with `anchor` at the origin.
    ///
    /// Translation-equivariant: shifting `self.start` and `anchor` by
    /// the same delta gives identical output, because the underlying
    /// `anchor.shortest_to(self.start)` is translation-invariant.
    pub fn lifts_anchored_at(
        self,
        anchor: TorusPoint,
    ) -> impl Iterator<Item = ((i32, i32), ((f32, f32), (f32, f32)))> {
        let v = anchor.shortest_to(self.start);
        let ax = v.dx;
        let ay = v.dy;
        let bx = ax + self.disp.dx;
        let by = ay + self.disp.dy;
        (-1..=1).flat_map(move |i| {
            (-1..=1).map(move |j| {
                let ox = i as f32;
                let oy = j as f32;
                ((i, j), ((ax + ox, ay + oy), (bx + ox, by + oy)))
            })
        })
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
        fn next_point(&mut self) -> TorusPoint {
            TorusPoint::new(self.next_signed_f32(), self.next_signed_f32())
        }
        fn next_vec(&mut self) -> TorusVec {
            TorusVec::new(self.next_signed_f32(), self.next_signed_f32())
        }
        fn next_segment(&mut self) -> TorusSegment {
            TorusSegment {
                start: self.next_point(),
                disp: self.next_vec(),
            }
        }
    }

    // ---------- translation-invariance / equivariance ----------
    //
    // Every public TorusPoint method has a fuzz test below verifying
    // that uniform translation of all inputs leaves the output
    // unchanged (for invariant ops) or shifted by the same amount
    // (for equivariant ops). Tolerances are tight: most pass bit-equal.

    /// `shortest_to` is exactly translation-invariant component-wise.
    #[test]
    fn fuzz_shortest_to_is_translation_invariant() {
        let mut rng = Lcg::new(11);
        for _ in 0..5000 {
            let a = rng.next_point();
            let b = rng.next_point();
            let shift = rng.next_vec();
            let v = a.shortest_to(b);
            let vs = a.translate(shift).shortest_to(b.translate(shift));
            assert_eq!(v.dx.to_bits(), vs.dx.to_bits());
            assert_eq!(v.dy.to_bits(), vs.dy.to_bits());
        }
    }

    /// `distance_to` is exactly translation-invariant.
    #[test]
    fn fuzz_distance_to_is_translation_invariant() {
        let mut rng = Lcg::new(12);
        for _ in 0..5000 {
            let a = rng.next_point();
            let b = rng.next_point();
            let shift = rng.next_vec();
            assert_eq!(
                a.distance_to(b).to_bits(),
                a.translate(shift).distance_to(b.translate(shift)).to_bits()
            );
        }
    }

    /// `translate` is equivariant: translating, then shifting, equals
    /// shifting, then translating.
    #[test]
    fn fuzz_translate_is_equivariant() {
        let mut rng = Lcg::new(13);
        for _ in 0..5000 {
            let p = rng.next_point();
            let v = rng.next_vec();
            let shift = rng.next_vec();
            // (p.translate(v)).translate(shift) ≡ (p.translate(shift)).translate(v)
            let lhs = p.translate(v).translate(shift);
            let rhs = p.translate(shift).translate(v);
            // Tolerance because shifted compositions can differ by 1
            // ULP in f32. Bound the disagreement in Coord units.
            assert!(lhs.distance_to(rhs) < 1e-6);
        }
    }

    /// `TorusSegment::end` is equivariant: shifting the start shifts
    /// the end by the same delta.
    #[test]
    fn fuzz_segment_end_is_equivariant() {
        let mut rng = Lcg::new(14);
        for _ in 0..5000 {
            let s = rng.next_segment();
            let shift = rng.next_vec();
            let lhs = s.end().translate(shift);
            let rhs = TorusSegment {
                start: s.start.translate(shift),
                disp: s.disp,
            }
            .end();
            assert!(lhs.distance_to(rhs) < 1e-6);
        }
    }

    /// `TorusSegment::lifts_anchored_at` is exactly translation-
    /// invariant: shifting `self.start` AND `anchor` by the same
    /// delta gives bit-equal output.
    #[test]
    fn fuzz_segment_lifts_are_translation_invariant() {
        let mut rng = Lcg::new(15);
        for _ in 0..2000 {
            let s = rng.next_segment();
            let anchor = rng.next_point();
            let shift = rng.next_vec();
            let s_shifted = TorusSegment {
                start: s.start.translate(shift),
                disp: s.disp,
            };
            let anchor_shifted = anchor.translate(shift);
            let lifts_a: Vec<_> = s.lifts_anchored_at(anchor).collect();
            let lifts_b: Vec<_> = s_shifted.lifts_anchored_at(anchor_shifted).collect();
            assert_eq!(lifts_a.len(), lifts_b.len());
            for (((_, ((a_sx, a_sy), (a_ex, a_ey))), (_, ((b_sx, b_sy), (b_ex, b_ey)))))
                in lifts_a.iter().zip(lifts_b.iter())
            {
                assert_eq!(a_sx.to_bits(), b_sx.to_bits());
                assert_eq!(a_sy.to_bits(), b_sy.to_bits());
                assert_eq!(a_ex.to_bits(), b_ex.to_bits());
                assert_eq!(a_ey.to_bits(), b_ey.to_bits());
            }
        }
    }

    // ---------- spot checks ----------

    #[test]
    fn point_wrap_distance_is_zero() {
        // (1.3, -0.2) wraps to (0.3, 0.8) on the torus.
        let a = TorusPoint::new(1.3, -0.2);
        let b = TorusPoint::new(0.3, 0.8);
        assert!(a.distance_to(b) < 1e-5);
    }

    #[test]
    fn shortest_goes_across_seam() {
        let a = TorusPoint::new(0.9, 0.5);
        let b = TorusPoint::new(0.1, 0.5);
        let v = a.shortest_to(b);
        assert!((v.dx - 0.2).abs() < 1e-5);
        assert!(v.dy.abs() < 1e-5);
    }

    #[test]
    fn distance_is_symmetric() {
        let a = TorusPoint::new(0.1, 0.2);
        let b = TorusPoint::new(0.7, 0.9);
        assert!((a.distance_to(b) - b.distance_to(a)).abs() < 1e-6);
    }

    #[test]
    fn segment_end_consistent_with_translate() {
        let seg = TorusSegment {
            start: TorusPoint::new(0.4, 0.4),
            disp: TorusVec::new(0.7, 0.7),
        };
        // start + disp = (1.1, 1.1), wraps to (0.1, 0.1).
        let expected = TorusPoint::new(0.1, 0.1);
        assert!(seg.end().distance_to(expected) < 1e-5);
    }
}
