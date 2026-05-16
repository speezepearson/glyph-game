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

    /// Iterate the `(2r+1) × (2r+1)` block of integer translates of
    /// this segment in the universal cover, in a frame anchored at
    /// `anchor`. Each item is `((li, lj), ((start_x, start_y),
    /// (end_x, end_y)))` — the integer lift indices for self-pair
    /// filtering, followed by the lifted (start, end) pair in ℝ²
    /// with `anchor` at the origin.
    ///
    /// Pick `r = 1` (3×3) for rendering & hit-testing of segments
    /// with `|disp| < 0.5`. Pick `r = 2` (5×5) when looking for
    /// segment-segment intersections among segments whose lifted
    /// disps can reach ~1 per axis (e.g. chopped sub-edges with
    /// winding ≠ 0). With `|disp| ≤ 1` per axis and signed_diff in
    /// (-0.5, 0.5], an intersection can sit at any lift `|li|, |lj|
    /// ≤ 2`, so 5×5 is the minimum.
    ///
    /// Translation-equivariant: shifting `self.start` and `anchor` by
    /// the same delta gives identical output, because the underlying
    /// `anchor.shortest_to(self.start)` is translation-invariant.
    pub fn lifts_anchored_at(
        self,
        anchor: TorusPoint,
        radius: i32,
    ) -> impl Iterator<Item = ((i32, i32), ((f32, f32), (f32, f32)))> {
        let v = anchor.shortest_to(self.start);
        let ax = v.dx;
        let ay = v.dy;
        let bx = ax + self.disp.dx;
        let by = ay + self.disp.dy;
        (-radius..=radius).flat_map(move |i| {
            (-radius..=radius).map(move |j| {
                let ox = i as f32;
                let oy = j as f32;
                ((i, j), ((ax + ox, ay + oy), (bx + ox, by + oy)))
            })
        })
    }
}

#[cfg(test)]
impl quickcheck::Arbitrary for TorusPoint {
    fn arbitrary(g: &mut quickcheck::Gen) -> Self {
        TorusPoint {
            x: crate::coord::Coord::arbitrary(g),
            y: crate::coord::Coord::arbitrary(g),
        }
    }
    fn shrink(&self) -> Box<dyn Iterator<Item = Self>> {
        Box::new(
            (self.x, self.y)
                .shrink()
                .map(|(x, y)| TorusPoint { x, y }),
        )
    }
}

#[cfg(test)]
impl quickcheck::Arbitrary for TorusVec {
    fn arbitrary(g: &mut quickcheck::Gen) -> Self {
        // Bounded to roughly [-1, 1) per component, matching the
        // realistic input range for a drawn segment (the canvas is 1
        // unit on a side). The chop algorithm's 5×5 lift block covers
        // segments whose lifted disp stays within ~1 in each axis.
        let mk = |g: &mut quickcheck::Gen| (i16::arbitrary(g) as f32) / 32768.0;
        TorusVec::new(mk(g), mk(g))
    }
    fn shrink(&self) -> Box<dyn Iterator<Item = Self>> {
        // Shrink each axis by round-tripping through i16 (matching
        // Arbitrary's representation). Shrinks toward (0, 0), which
        // is a degenerate zero-length disp — quickcheck's per-test
        // skips don't filter those out at the type level, but our
        // chop's `disp²<EPS²` check does.
        let to_i16 = |f: f32| (f * 32768.0).clamp(-32768.0, 32767.0) as i16;
        let from_i16 = |i: i16| (i as f32) / 32768.0;
        Box::new(
            (to_i16(self.dx), to_i16(self.dy))
                .shrink()
                .map(move |(dx, dy)| TorusVec::new(from_i16(dx), from_i16(dy))),
        )
    }
}

#[cfg(test)]
impl quickcheck::Arbitrary for TorusSegment {
    fn arbitrary(g: &mut quickcheck::Gen) -> Self {
        TorusSegment {
            start: TorusPoint::arbitrary(g),
            disp: TorusVec::arbitrary(g),
        }
    }
    fn shrink(&self) -> Box<dyn Iterator<Item = Self>> {
        Box::new(
            (self.start, self.disp)
                .shrink()
                .map(|(start, disp)| TorusSegment { start, disp }),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use quickcheck_macros::quickcheck;

    // ---------- translation-invariance / equivariance properties ----------
    //
    // Every public TorusPoint method has a quickcheck property
    // verifying that uniform translation of all inputs leaves the
    // output unchanged (invariant ops) or shifted by the same delta
    // (equivariant ops). Bit-equal for the integer-arithmetic-backed
    // ops; 1e-6 tolerance for the few that round-trip an f32 sum.

    /// `shortest_to` is bit-equal under uniform translation.
    #[quickcheck]
    fn prop_shortest_to_is_translation_invariant(
        a: TorusPoint,
        b: TorusPoint,
        shift: TorusVec,
    ) -> bool {
        let v = a.shortest_to(b);
        let vs = a.translate(shift).shortest_to(b.translate(shift));
        v.dx.to_bits() == vs.dx.to_bits() && v.dy.to_bits() == vs.dy.to_bits()
    }

    /// `distance_to` is bit-equal under uniform translation.
    #[quickcheck]
    fn prop_distance_to_is_translation_invariant(
        a: TorusPoint,
        b: TorusPoint,
        shift: TorusVec,
    ) -> bool {
        a.distance_to(b).to_bits()
            == a.translate(shift).distance_to(b.translate(shift)).to_bits()
    }

    /// `translate` is equivariant: translation order doesn't matter.
    /// Composed f32 shifts can differ by 1 ULP, so we bound by torus
    /// distance rather than expecting bit-equality.
    #[quickcheck]
    fn prop_translate_is_equivariant(p: TorusPoint, v: TorusVec, shift: TorusVec) -> bool {
        let lhs = p.translate(v).translate(shift);
        let rhs = p.translate(shift).translate(v);
        lhs.distance_to(rhs) < 1e-6
    }

    /// `TorusSegment::end` is equivariant: shifting the start shifts
    /// the end by the same delta.
    #[quickcheck]
    fn prop_segment_end_is_equivariant(s: TorusSegment, shift: TorusVec) -> bool {
        let lhs = s.end().translate(shift);
        let rhs = TorusSegment {
            start: s.start.translate(shift),
            disp: s.disp,
        }
        .end();
        lhs.distance_to(rhs) < 1e-6
    }

    /// `TorusSegment::lifts_anchored_at` is bit-equal under uniform
    /// translation of both `self.start` and `anchor`.
    #[quickcheck]
    fn prop_segment_lifts_are_translation_invariant(
        s: TorusSegment,
        anchor: TorusPoint,
        shift: TorusVec,
    ) -> bool {
        let s_shifted = TorusSegment {
            start: s.start.translate(shift),
            disp: s.disp,
        };
        let anchor_shifted = anchor.translate(shift);
        let lifts_a: Vec<_> = s.lifts_anchored_at(anchor, 2).collect();
        let lifts_b: Vec<_> = s_shifted.lifts_anchored_at(anchor_shifted, 2).collect();
        if lifts_a.len() != lifts_b.len() {
            return false;
        }
        for ((_, ((a_sx, a_sy), (a_ex, a_ey))), (_, ((b_sx, b_sy), (b_ex, b_ey))))
            in lifts_a.iter().zip(lifts_b.iter())
        {
            if a_sx.to_bits() != b_sx.to_bits()
                || a_sy.to_bits() != b_sy.to_bits()
                || a_ex.to_bits() != b_ex.to_bits()
                || a_ey.to_bits() != b_ey.to_bits()
            {
                return false;
            }
        }
        true
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
