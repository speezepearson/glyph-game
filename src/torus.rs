//! Geometry on the flat 2-torus T² = ℝ² / ℤ².
//!
//! # The seam-free coordinate type
//!
//! [`Coord`] is a single torus axis represented as `u32`. The range
//! `0..=u32::MAX` maps to `[0, 1)` on the circle. The mapping wraps
//! naturally: `u32` overflow *is* the modulo-1 reduction, with no
//! special "seam at 0/1" code path anywhere. Two operations that
//! should agree on the torus do agree, exactly — there's no f32
//! representational asymmetry between "0.0001" and "0.9999".
//!
//! `Coord`'s internal value is private and only relative operations
//! (shortest signed difference, translate-by-delta) are exposed. All
//! such operations are *translation-invariant*: applying the same
//! shift to every Coord in the inputs gives identical outputs. This
//! is the property the type is designed to enforce.
//!
//! `to_f32` exists for boundary rendering and intersection-finding,
//! and gives a canonical representative in `[0, 1]`. It is *not*
//! translation-invariant on its own — the caller is responsible for
//! choosing a viewport / picking lifts.
//!
//! # Other types
//!
//! - [`TorusPoint`] = `(Coord, Coord)`.
//! - [`TorusVec`] is a displacement in ℝ² in the universal cover. It
//!   is *not* reduced modulo the lattice; different vectors that
//!   translate to the same place represent different homotopy classes.
//! - [`TorusSegment`] is `(start, disp)`: a straight line in the
//!   universal cover, projected down to the torus.

// 2^32 as f32 — exact, since 2^32 is a power of two within f32's
// dynamic range.
const U_SCALE: f32 = 4_294_967_296.0;

/// One torus axis. Internal representation is `u32` with `0..=u32::MAX`
/// mapping to `[0, 1)`; wrapping is u32 native arithmetic, so the
/// torus has no privileged "0/1 boundary".
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct Coord {
    u: u32,
}

impl Coord {
    /// Convert any `f32` in (-∞, ∞) to a Coord, wrapping into the
    /// torus axis. (Used at the input boundary, e.g. when the user's
    /// mouse position becomes a torus position.)
    pub fn from_f32(f: f32) -> Self {
        let frac = f.rem_euclid(1.0);
        // f.rem_euclid(1.0) can return values up to 1.0 due to f32
        // rounding; the saturating `as u32` cast then maps that to
        // u32::MAX, which represents "just before 0" — fine.
        Self {
            u: (frac * U_SCALE) as u32,
        }
    }

    /// Canonical f32 representative in `[0, 1]`. Returned values are
    /// in increasing order with `self.u`. This is **not**
    /// translation-invariant — it commits to a specific lift. Use only
    /// at output boundaries (rendering, intersection finding) where
    /// the caller has already chosen a viewport.
    pub fn to_f32(self) -> f32 {
        self.u as f32 / U_SCALE
    }

    /// Shift this coord on the torus by an arbitrary signed f32 delta
    /// (positive or negative, magnitude unbounded — the delta wraps
    /// into u32 arithmetic).
    pub fn shifted(self, delta: f32) -> Self {
        // Convert delta to a signed u32 offset (mod 2^32) and add.
        // delta.rem_euclid(1.0) lands in [0, 1.0]; reinterpret in u32
        // wrapping arithmetic. The sign of the original delta is
        // absorbed by rem_euclid into the wrap direction.
        let frac = delta.rem_euclid(1.0);
        let offset = (frac * U_SCALE) as u32;
        Self {
            u: self.u.wrapping_add(offset),
        }
    }

    /// Shortest signed difference `other - self`, expressed as an f32
    /// in `(-0.5, 0.5]` (or `[-0.5, 0.5]` at the half-circle boundary).
    /// Translation-invariant: shifting `self` and `other` by the same
    /// amount gives the same result exactly.
    pub fn signed_diff_to(self, other: Coord) -> f32 {
        // The u32 wrapping subtraction encodes the signed shortest
        // delta modulo 2^32; reinterpret as i32 to get a value in
        // [-2^31, 2^31), then divide by 2^32 to get a fraction in
        // [-0.5, 0.5).
        let raw = other.u.wrapping_sub(self.u) as i32;
        raw as f32 / U_SCALE
    }
}

/// A point on the flat torus.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct TorusPoint {
    pub x: Coord,
    pub y: Coord,
}

impl TorusPoint {
    /// Construct from arbitrary `(x, y)` in ℝ². Wraps into the torus.
    pub fn new(x: f32, y: f32) -> Self {
        Self {
            x: Coord::from_f32(x),
            y: Coord::from_f32(y),
        }
    }

    /// Canonical `(x, y)` representative in `[0, 1] × [0, 1]`. Use at
    /// output boundaries only; see [`Coord::to_f32`].
    pub fn to_xy(self) -> (f32, f32) {
        (self.x.to_f32(), self.y.to_f32())
    }

    /// Translate by a displacement vector.
    pub fn translate(self, v: TorusVec) -> Self {
        Self {
            x: self.x.shifted(v.dx),
            y: self.y.shifted(v.dy),
        }
    }

    /// The *shortest* displacement vector from `self` to `other`. Each
    /// component lies in `(-½, ½]`. Translation-invariant.
    pub fn shortest_to(self, other: TorusPoint) -> TorusVec {
        TorusVec {
            dx: self.x.signed_diff_to(other.x),
            dy: self.y.signed_diff_to(other.y),
        }
    }

    /// Toroidal Euclidean distance.
    pub fn distance_to(self, other: TorusPoint) -> f32 {
        let v = self.shortest_to(other);
        (v.dx * v.dx + v.dy * v.dy).sqrt()
    }
}

/// A displacement vector in the universal cover ℝ². NOT reduced modulo
/// the lattice; `TorusVec { dx: 1.7, dy: 0.0 }` and
/// `TorusVec { dx: 0.7, dy: 0.0 }` translate any point to the same
/// place on the torus but represent different homotopy classes.
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

/// A straight-line segment on the torus, stored as (start, lifted disp).
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
    /// the universal cover, expressed as `(start_xy, end_xy)` pairs in
    /// canonical-representative ℝ². Used for rendering and hit-testing
    /// the torus through a `[0,1]²` viewport.
    pub fn visible_lifts(self) -> impl Iterator<Item = ((f32, f32), (f32, f32))> {
        let (ax, ay) = self.start.to_xy();
        let bx = ax + self.disp.dx;
        let by = ay + self.disp.dy;
        (-1..=1).flat_map(move |i| {
            (-1..=1).map(move |j| {
                let ox = i as f32;
                let oy = j as f32;
                ((ax + ox, ay + oy), (bx + ox, by + oy))
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx_eq(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-5
    }

    #[test]
    fn point_wraps_into_unit_square() {
        let p = TorusPoint::new(1.3, -0.2);
        let (x, y) = p.to_xy();
        assert!(approx_eq(x, 0.3));
        assert!(approx_eq(y, 0.8));
    }

    #[test]
    fn shortest_goes_across_seam() {
        let a = TorusPoint::new(0.9, 0.5);
        let b = TorusPoint::new(0.1, 0.5);
        let v = a.shortest_to(b);
        assert!(approx_eq(v.dx, 0.2));
        assert!(approx_eq(v.dy, 0.0));
    }

    #[test]
    fn shortest_distance_is_symmetric() {
        let a = TorusPoint::new(0.1, 0.2);
        let b = TorusPoint::new(0.7, 0.9);
        assert!(approx_eq(a.distance_to(b), b.distance_to(a)));
    }

    #[test]
    fn segment_end_consistent_with_translate() {
        let seg = TorusSegment {
            start: TorusPoint::new(0.4, 0.4),
            disp: TorusVec::new(0.7, 0.7),
        };
        let (ex, ey) = seg.end().to_xy();
        assert!(approx_eq(ex, 0.1));
        assert!(approx_eq(ey, 0.1));
    }

    // ---------- translation-invariance fuzz ----------

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
    /// shifting both Coords by the same delta gives the exact same f32
    /// answer (bit-equal, no tolerance).
    #[test]
    fn fuzz_signed_diff_is_exactly_translation_invariant() {
        let mut rng = Lcg::new(1);
        for _ in 0..5000 {
            let a = Coord::from_f32(rng.next_signed_f32());
            let b = Coord::from_f32(rng.next_signed_f32());
            let shift = rng.next_signed_f32();
            let a_s = a.shifted(shift);
            let b_s = b.shifted(shift);
            let d1 = a.signed_diff_to(b);
            let d2 = a_s.signed_diff_to(b_s);
            assert_eq!(d1.to_bits(), d2.to_bits(), "shift={shift}");
        }
    }

    /// TorusPoint::distance_to is exactly translation-invariant.
    #[test]
    fn fuzz_distance_is_translation_invariant() {
        let mut rng = Lcg::new(2);
        for _ in 0..5000 {
            let a = TorusPoint::new(rng.next_signed_f32(), rng.next_signed_f32());
            let b = TorusPoint::new(rng.next_signed_f32(), rng.next_signed_f32());
            let shift = TorusVec::new(rng.next_signed_f32(), rng.next_signed_f32());
            let a_s = a.translate(shift);
            let b_s = b.translate(shift);
            assert_eq!(a.distance_to(b).to_bits(), a_s.distance_to(b_s).to_bits());
        }
    }

    /// shortest_to is exactly translation-invariant component-wise.
    #[test]
    fn fuzz_shortest_to_is_translation_invariant() {
        let mut rng = Lcg::new(3);
        for _ in 0..5000 {
            let a = TorusPoint::new(rng.next_signed_f32(), rng.next_signed_f32());
            let b = TorusPoint::new(rng.next_signed_f32(), rng.next_signed_f32());
            let shift = TorusVec::new(rng.next_signed_f32(), rng.next_signed_f32());
            let a_s = a.translate(shift);
            let b_s = b.translate(shift);
            let v = a.shortest_to(b);
            let vs = a_s.shortest_to(b_s);
            assert_eq!(v.dx.to_bits(), vs.dx.to_bits());
            assert_eq!(v.dy.to_bits(), vs.dy.to_bits());
        }
    }
}
