# glyph-game

A sketch of a game where the player draws **glyphs** — assemblages of
line segments — whose effects come from the *combinatorial embedding of
the planar graph induced by the drawing*. (Faces, cycles, rotations
around each vertex: the topology, not just the geometry.)

The twist: the drawing surface is a **flat torus** (T² = ℝ² / ℤ²), not
the plane. Glyphs can wrap, link, and produce non-trivial topology in
ways they can't on a sheet of paper. This is currently just the drawing
surface; the glyph-effect mechanics come later.

## How we treat the torus (the principled bit)

Rather than treat wrapping as a rendering hack on top of plain ℝ²
geometry, the geometry types in [`src/torus.rs`](src/torus.rs) are
toroidal from the ground up:

- **`TorusPoint`** — a point on T². Stored as the canonical
  representative in [0, 1)². Constructors reduce modulo ℤ² so it's
  impossible to hold an out-of-range value.
- **`TorusVec`** — a displacement in the universal cover ℝ². Crucially,
  this is *not* reduced modulo ℤ²: `TorusVec { dx: 1.7, .. }` and
  `TorusVec { dx: 0.7, .. }` translate any point to the same place but
  represent different *homotopy classes* of path. That matters for line
  segments.
- **`TorusPoint::shortest_to`** — the unique displacement with each
  component in (−½, ½]. The "obvious" path you'd draw between two
  points. Use this for hit-tests, distance, and initial drags.
- **`TorusSegment`** — `{ start, disp }`, **not** `{ start, end }`.
  Storing the displacement (an ℝ² vector) commits the segment to a
  specific lift in the universal cover, so a segment that wraps stays
  wrapped as the user drags its endpoints. Two torus points have
  infinitely many straight lines between them on T²; we pick one and
  remember which.
- **`TorusSegment::visible_lifts`** — yields integer translates of the
  segment covering the unit-square viewport, used both for rendering
  (each visible copy is clipped to the canvas) and for hit-testing
  (point-to-segment distance is the min over visible lifts).

Operations like "move the start endpoint, keep the end fixed" become
unambiguous given this representation: `seg.move_start(v)` translates
`start` by `v` and absorbs `−v` into `disp`, preserving the segment's
homotopy class as long as the drag is reasonable.

## The current program

A square viewport renders one fundamental domain of T². Anything that
crosses an edge reappears on the opposite edge, in the classic
toroidal-map style. You can:

- **Left-drag on empty canvas** → draw a new line segment.
- **Left-drag an endpoint** (yellow circle) → move just that endpoint.
- **Left-drag a segment body** → translate the whole segment.
- **Right-click a segment** → delete it.

## Running it

Natively (desktop window):

```sh
cargo run --release
```

In a browser via WebAssembly:

```sh
./scripts/build-web.sh
python3 -m http.server --directory web 8080
# then open http://localhost:8080/
```

The web frontend is a single static page (`web/index.html`) that loads
the compiled `glyph_game.wasm` through macroquad's miniquad JS shim.
There's no server-side component.

### Deploying to GitHub Pages

`.github/workflows/deploy-pages.yml` builds the WASM and publishes
`web/` on every push to `main` (and on manual workflow dispatch). To
enable it once: repo **Settings → Pages → Build and deployment →
Source: GitHub Actions**. The deployed URL will appear in the workflow
run summary.

## Why this stack

- **[macroquad](https://macroquad.rs)** for rendering and input: minimal
  API surface, first-class WASM target, a single binary that runs both
  as a desktop window and (via `cargo build --target
  wasm32-unknown-unknown`) in the browser. We're not running a
  websocket server because there's no shared state to coordinate — the
  whole game is client-side.
- **No torus / planar-graph crate.** I looked, and the available
  crates are either heavy general-purpose computational-geometry packages
  or are focused on R³ surfaces. The toroidal-specific logic we need is
  small enough (and central enough to the gameplay) that owning it in
  `src/torus.rs` is clearer than wedging it onto a general library.

## Layout

```
src/
  main.rs    -- macroquad app: input handling, rendering, hit-testing
  torus.rs   -- toroidal geometry: points, vectors, segments
web/
  index.html -- WASM loader
scripts/
  build-web.sh
```
