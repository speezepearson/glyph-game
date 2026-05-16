//! Glyph Game — MVP drawing surface on the flat torus.
//!
//! Click and drag on empty space to draw a new line segment. Click on an
//! existing endpoint (small circle) to drag it; click on a segment's
//! interior to drag the whole segment. Right-click on a segment to delete
//! it. The viewport is a square fundamental domain of the torus, and
//! anything that leaves one edge reappears on the opposite edge.

use macroquad::prelude::*;

mod coord;
mod glyph;
mod torus_point;
use glyph::{triangulate, Glyph};
use torus_point::{TorusPoint, TorusSegment, TorusVec};

const ENDPOINT_RADIUS: f32 = 6.0;
const HIT_RADIUS_DEFAULT_PX: f32 = 10.0;
const HIT_RADIUS_MIN_PX: f32 = 2.0;
const HIT_RADIUS_MAX_PX: f32 = 40.0;
const LINE_WIDTH: f32 = 2.5;

const SLIDER_WIDTH: f32 = 220.0;
const SLIDER_HEIGHT: f32 = 8.0;
const SLIDER_KNOB_RADIUS: f32 = 8.0;
const SLIDER_MARGIN: f32 = 20.0;

/// What the user is currently doing with the mouse.
enum Drag {
    /// Drawing a brand-new segment; we own its growing displacement.
    DrawNew { start: TorusPoint, disp: TorusVec },
    /// Translating an entire glyph by the cumulative mouse delta.
    MoveGlyph { glyph_idx: usize },
}

struct App {
    glyphs: Vec<Glyph>,
    drag: Option<Drag>,
    hit_radius_px: f32,
    slider_drag: bool,
    /// The torus point sitting at the center of the canvas viewport.
    /// All rendering anchors here: signed_diff_to(viewport_anchor, p)
    /// gives p's position relative to the canvas center, which the
    /// renderer scales into a screen pixel. Stable across frames;
    /// the seam appears at the canvas edges (the viewport's antipode).
    viewport_anchor: TorusPoint,
    /// Past states, most recent on top. `undo` pops one and restores it.
    undo_stack: Vec<Vec<TorusSegment>>,
    /// States that were undone and can be re-applied.
    redo_stack: Vec<Vec<TorusSegment>>,
}

impl App {
    fn new() -> Self {
        Self {
            glyphs: Vec::new(),
            drag: None,
            hit_radius_px: HIT_RADIUS_DEFAULT_PX,
            slider_drag: false,
            // Default to (0.5, 0.5) — the center of the canonical
            // fundamental domain. Picked so the seam lands at the
            // canvas edges (the antipode of the anchor). A future
            // panning UI would change this.
            viewport_anchor: TorusPoint::new(0.5, 0.5),
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
        }
    }

    /// Flatten every glyph's segments into one list. The grouping into
    /// glyphs is recovered from this list on `restore` by reconnecting
    /// touching segments.
    fn snapshot(&self) -> Vec<TorusSegment> {
        self.glyphs.iter().flat_map(|g| g.segments()).collect()
    }

    /// Replace the current glyphs with whatever the given flat segment
    /// list implies, after partitioning into connected components.
    fn restore(&mut self, segments: Vec<TorusSegment>) {
        self.glyphs = glyph::partition_into_glyphs(segments)
            .into_iter()
            .map(Glyph::from_segments)
            .collect();
    }

    /// Snapshot the current state into the undo stack and clear the
    /// redo stack. Call this just before a state-changing action.
    fn commit_action(&mut self) {
        self.undo_stack.push(self.snapshot());
        self.redo_stack.clear();
    }

    fn undo(&mut self) {
        if let Some(prev) = self.undo_stack.pop() {
            let current = self.snapshot();
            self.redo_stack.push(current);
            self.restore(prev);
        }
    }

    fn redo(&mut self) {
        if let Some(next) = self.redo_stack.pop() {
            let current = self.snapshot();
            self.undo_stack.push(current);
            self.restore(next);
        }
    }

    /// Return the canonical position of the nearest vertex within
    /// `radius` of `p` across all glyphs, if any.
    fn snap_target(&self, p: TorusPoint, radius: f32) -> Option<TorusPoint> {
        let mut best: Option<(f32, TorusPoint)> = None;
        for g in &self.glyphs {
            for v in &g.vertices {
                let d = p.distance_to(*v);
                if d <= radius && best.map_or(true, |(bd, _)| d < bd) {
                    best = Some((d, *v));
                }
            }
        }
        best.map(|(_, v)| v)
    }

    fn place_new_segment(&mut self, seg: TorusSegment) {
        glyph::add_segment(&mut self.glyphs, seg);
    }
}

fn window_conf() -> Conf {
    Conf {
        window_title: "Glyph Game".to_string(),
        window_width: 900,
        window_height: 900,
        high_dpi: true,
        ..Default::default()
    }
}

#[macroquad::main(window_conf)]
async fn main() {
    let mut app = App::new();
    // Track the previous mouse position in *screen pixels* across the
    // frame boundary so we can compute exact deltas (in pixels) and
    // convert into torus space. This avoids any seam-crossing weirdness:
    // the cursor lives in the screen, which is just ℝ².
    let mut prev_mouse_px: Option<Vec2> = None;

    loop {
        let (canvas_origin, canvas_size) = canvas_rect();
        let mouse_px = vec2(mouse_position().0, mouse_position().1);
        let mouse_torus = screen_to_torus(mouse_px, canvas_origin, canvas_size);
        let in_canvas = point_in_canvas(mouse_px, canvas_origin, canvas_size);

        handle_input(
            &mut app,
            mouse_px,
            mouse_torus,
            in_canvas,
            prev_mouse_px,
            canvas_size,
        );

        clear_background(Color::from_rgba(18, 18, 22, 255));
        draw_canvas_frame(canvas_origin, canvas_size);
        draw_scene(&app, canvas_origin, canvas_size);
        draw_hud(&app);
        draw_slider(&app);

        prev_mouse_px = Some(mouse_px);
        next_frame().await;
    }
}

/// The square fundamental-domain viewport, centered in the window.
fn canvas_rect() -> (Vec2, f32) {
    let w = screen_width();
    let h = screen_height();
    let size = w.min(h) * 0.92;
    let origin = vec2((w - size) / 2.0, (h - size) / 2.0);
    (origin, size)
}

fn screen_to_torus(p: Vec2, origin: Vec2, size: f32) -> TorusPoint {
    TorusPoint::new((p.x - origin.x) / size, (p.y - origin.y) / size)
}

fn point_in_canvas(p: Vec2, origin: Vec2, size: f32) -> bool {
    p.x >= origin.x && p.x <= origin.x + size && p.y >= origin.y && p.y <= origin.y + size
}

fn handle_input(
    app: &mut App,
    mouse_px: Vec2,
    mouse_torus: TorusPoint,
    in_canvas: bool,
    prev_mouse_px: Option<Vec2>,
    canvas_size: f32,
) {
    let just_pressed_left = is_mouse_button_pressed(MouseButton::Left);
    let just_released_left = is_mouse_button_released(MouseButton::Left);

    // Slider takes priority over canvas interactions.
    let (slider_pos, slider_size) = slider_rect();
    if just_pressed_left && point_in_slider(mouse_px, slider_pos, slider_size) {
        app.slider_drag = true;
    }
    if app.slider_drag {
        let t = ((mouse_px.x - slider_pos.x) / slider_size.x).clamp(0.0, 1.0);
        app.hit_radius_px = HIT_RADIUS_MIN_PX + t * (HIT_RADIUS_MAX_PX - HIT_RADIUS_MIN_PX);
        if just_released_left {
            app.slider_drag = false;
        }
        return;
    }

    let snap_radius = app.hit_radius_px / canvas_size;

    // Keyboard shortcuts: Ctrl/Cmd+Z = undo, Ctrl/Cmd+Shift+Z or
    // Ctrl/Cmd+Y = redo.
    let ctrl = is_key_down(KeyCode::LeftControl)
        || is_key_down(KeyCode::RightControl)
        || is_key_down(KeyCode::LeftSuper)
        || is_key_down(KeyCode::RightSuper);
    let shift = is_key_down(KeyCode::LeftShift) || is_key_down(KeyCode::RightShift);
    if ctrl && is_key_pressed(KeyCode::Z) {
        if shift {
            app.redo();
        } else {
            app.undo();
        }
        return;
    }
    if ctrl && is_key_pressed(KeyCode::Y) {
        app.redo();
        return;
    }

    // Right-click: delete the edge under the cursor (if any). If
    // removing it disconnects the glyph, the remainder is re-
    // partitioned into multiple glyphs.
    if is_mouse_button_pressed(MouseButton::Right) && in_canvas {
        if let Some((gi, ei)) = pick_edge_in_glyphs(&app.glyphs, mouse_torus) {
            app.commit_action();
            glyph::remove_edge(&mut app.glyphs, gi, ei);
        }
        return;
    }

    if just_pressed_left && in_canvas {
        // Priority: clicking on (or near) a vertex starts a draw from
        // that vertex (snapped). Clicking on an edge body but not a
        // vertex drags the whole glyph. Clicking on empty space starts
        // a free-floating draw.
        if let Some(snapped) = app.snap_target(mouse_torus, snap_radius) {
            app.drag = Some(Drag::DrawNew {
                start: snapped,
                disp: TorusVec::zero(),
            });
        } else if let Some((gi, _ei)) = pick_edge_in_glyphs(&app.glyphs, mouse_torus) {
            app.commit_action();
            app.drag = Some(Drag::MoveGlyph { glyph_idx: gi });
        } else {
            app.drag = Some(Drag::DrawNew {
                start: mouse_torus,
                disp: TorusVec::zero(),
            });
        }
    }

    // Mouse delta in torus coords for this frame. We use the screen-pixel
    // delta (which never wraps) and rescale — so dragging across the
    // torus seam works without snapping. We skip the delta on the frame
    // the button was pressed, because it would reflect motion from before
    // the drag began.
    if let (Some(prev), Some(drag)) = (prev_mouse_px, app.drag.as_mut()) {
        if !just_pressed_left {
            let mouse_delta = TorusVec::new(
                (mouse_px.x - prev.x) / canvas_size,
                (mouse_px.y - prev.y) / canvas_size,
            );
            match drag {
                Drag::DrawNew { disp, .. } => {
                    *disp = *disp + mouse_delta;
                }
                Drag::MoveGlyph { glyph_idx } => {
                    app.glyphs[*glyph_idx].translate(mouse_delta);
                }
            }
        }
    }

    if is_mouse_button_released(MouseButton::Left) {
        match app.drag.take() {
            Some(Drag::DrawNew { start, disp }) => {
                // Discard zero-length scratches.
                if disp.length() <= 0.005 {
                    return;
                }
                // Snap the released endpoint to a nearby vertex. The
                // correction is `raw_end → snapped_end` along the
                // shortest path on the torus — exactly what
                // signed_diff_to gives. (Old code did a canonical-rep
                // subtraction plus a manual normalize-by-±1, both
                // dropped in favor of this.)
                let raw_end = start.translate(disp);
                let snapped_end = app
                    .snap_target(raw_end, app.hit_radius_px / canvas_size)
                    .unwrap_or(raw_end);
                let correction = raw_end.shortest_to(snapped_end);
                let final_disp = TorusVec::new(disp.dx + correction.dx, disp.dy + correction.dy);
                app.commit_action();
                app.place_new_segment(TorusSegment {
                    start,
                    disp: final_disp,
                });
            }
            Some(Drag::MoveGlyph { .. }) => {
                // commit_action was called when the drag started; no
                // additional snapshot needed on release.
            }
            None => {}
        }
    }
}

/// Position and size of the hit-radius slider track, in screen pixels.
fn slider_rect() -> (Vec2, Vec2) {
    let w = screen_width();
    let x = w - SLIDER_WIDTH - SLIDER_MARGIN;
    let y = SLIDER_MARGIN;
    (vec2(x, y), vec2(SLIDER_WIDTH, SLIDER_HEIGHT))
}

fn point_in_slider(p: Vec2, track_pos: Vec2, track_size: Vec2) -> bool {
    let pad = SLIDER_KNOB_RADIUS + 2.0;
    p.x >= track_pos.x - pad
        && p.x <= track_pos.x + track_size.x + pad
        && p.y >= track_pos.y - pad
        && p.y <= track_pos.y + track_size.y + pad
}

/// Hit-test edges across all glyphs. Returns (glyph_idx, edge_idx).
fn pick_edge_in_glyphs(glyphs: &[Glyph], p: TorusPoint) -> Option<(usize, usize)> {
    let hit = 0.012_f32; // ~1.2% of viewport
    let mut best: Option<(f32, usize, usize)> = None;
    for (gi, g) in glyphs.iter().enumerate() {
        for ei in 0..g.edges.len() {
            let seg = g.edge_segment(ei);
            let d = distance_point_to_segment(&seg, p);
            if d <= hit && best.map_or(true, |(bd, _, _)| d < bd) {
                best = Some((d, gi, ei));
            }
        }
    }
    best.map(|(_, gi, ei)| (gi, ei))
}

/// Shortest distance from a torus point `p` to a torus segment.
/// Anchored at `p` itself: `p` sits at (0, 0) in the working frame,
/// and we iterate the segment's 9 lifts in that frame.
fn distance_point_to_segment(seg: &TorusSegment, p: TorusPoint) -> f32 {
    let mut best = f32::INFINITY;
    for (_, ((ax, ay), (bx, by))) in seg.lifts_anchored_at(p) {
        let d = dist_point_to_seg_2d(0.0, 0.0, ax, ay, bx, by);
        if d < best {
            best = d;
        }
    }
    best
}

fn dist_point_to_seg_2d(px: f32, py: f32, ax: f32, ay: f32, bx: f32, by: f32) -> f32 {
    let dx = bx - ax;
    let dy = by - ay;
    let len2 = dx * dx + dy * dy;
    let t = if len2 < 1e-12 {
        0.0
    } else {
        (((px - ax) * dx + (py - ay) * dy) / len2).clamp(0.0, 1.0)
    };
    let cx = ax + t * dx;
    let cy = ay + t * dy;
    ((px - cx).powi(2) + (py - cy).powi(2)).sqrt()
}

// ---------- rendering ----------

fn draw_canvas_frame(origin: Vec2, size: f32) {
    draw_rectangle(origin.x, origin.y, size, size, Color::from_rgba(8, 8, 12, 255));
    draw_rectangle_lines(
        origin.x,
        origin.y,
        size,
        size,
        2.0,
        Color::from_rgba(80, 80, 100, 255),
    );
}

fn draw_scene(app: &App, origin: Vec2, size: f32) {
    let rect_min = origin;
    let rect_max = origin + vec2(size, size);
    let anchor = app.viewport_anchor;

    // Pass 1: outer faces (negative signed area). For each, tint the
    // entire canvas with its color. Inner faces drawn next overwrite
    // the parts that should have a different color.
    for g in &app.glyphs {
        for f in &g.faces {
            if f.polygon.is_empty() || f.signed_area >= 0.0 {
                continue;
            }
            draw_rectangle(origin.x, origin.y, size, size, f.color);
        }
    }
    // Pass 2: inner faces (positive signed area), filled as tiled
    // polygons.
    for g in &app.glyphs {
        for f in &g.faces {
            if f.polygon.is_empty() || f.signed_area <= 0.0 {
                continue;
            }
            draw_face_tiled(
                &f.polygon, f.anchor, anchor, f.color, origin, size, rect_min, rect_max,
            );
        }
    }

    // Pass 3: the edges themselves.
    let mut to_draw: Vec<TorusSegment> = Vec::new();
    for g in &app.glyphs {
        to_draw.extend(g.segments());
    }
    if let Some(Drag::DrawNew { start, disp }) = &app.drag {
        to_draw.push(TorusSegment {
            start: *start,
            disp: *disp,
        });
    }

    for seg in &to_draw {
        draw_segment_tiled(seg, anchor, origin, size, rect_min, rect_max);
    }
    // Pass 4: vertex dots — one per actual DCEL vertex, plus the
    // endpoints of any in-progress draw.
    for g in &app.glyphs {
        for v in &g.vertices {
            draw_endpoint_tiled(*v, anchor, origin, size, rect_min, rect_max, ENDPOINT_COLOR);
        }
    }
    if let Some(Drag::DrawNew { start, disp }) = &app.drag {
        draw_endpoint_tiled(*start, anchor, origin, size, rect_min, rect_max, ENDPOINT_COLOR);
        draw_endpoint_tiled(
            start.translate(*disp),
            anchor,
            origin,
            size,
            rect_min,
            rect_max,
            ENDPOINT_COLOR,
        );
    }
}

const ENDPOINT_COLOR: Color = Color::new(0.9, 0.85, 0.3, 1.0);
const LINE_COLOR: Color = Color::new(0.85, 0.9, 1.0, 1.0);

/// Map a position in the viewport's anchor frame (signed_diff units,
/// i.e. (-0.5, 0.5] for the primary tile) to a screen pixel. The
/// viewport anchor sits at the canvas's center pixel.
fn rel_to_screen(rx: f32, ry: f32, canvas_origin: Vec2, canvas_size: f32) -> Vec2 {
    let center = canvas_origin + vec2(canvas_size, canvas_size) * 0.5;
    center + vec2(rx, ry) * canvas_size
}

fn draw_segment_tiled(
    seg: &TorusSegment,
    anchor: TorusPoint,
    canvas_origin: Vec2,
    canvas_size: f32,
    rect_min: Vec2,
    rect_max: Vec2,
) {
    for (_, ((ax, ay), (bx, by))) in seg.lifts_anchored_at(anchor) {
        let pa = rel_to_screen(ax, ay, canvas_origin, canvas_size);
        let pb = rel_to_screen(bx, by, canvas_origin, canvas_size);
        if let Some((c0, c1)) = clip_line_to_rect(pa, pb, rect_min, rect_max) {
            draw_line(c0.x, c0.y, c1.x, c1.y, LINE_WIDTH, LINE_COLOR);
        }
    }
}

fn draw_endpoint_tiled(
    p: TorusPoint,
    anchor: TorusPoint,
    canvas_origin: Vec2,
    canvas_size: f32,
    rect_min: Vec2,
    rect_max: Vec2,
    color: Color,
) {
    let v = anchor.shortest_to(p);
    for i in -1..=1 {
        for j in -1..=1 {
            let s = rel_to_screen(
                v.dx + i as f32,
                v.dy + j as f32,
                canvas_origin,
                canvas_size,
            );
            if s.x + ENDPOINT_RADIUS < rect_min.x || s.x - ENDPOINT_RADIUS > rect_max.x {
                continue;
            }
            if s.y + ENDPOINT_RADIUS < rect_min.y || s.y - ENDPOINT_RADIUS > rect_max.y {
                continue;
            }
            draw_circle(s.x, s.y, ENDPOINT_RADIUS, color);
            draw_circle_lines(s.x, s.y, ENDPOINT_RADIUS, 1.5, Color::from_rgba(20, 20, 30, 255));
        }
    }
}

/// Fill a face by triangulating its boundary polygon and drawing
/// every integer translate that overlaps the canvas. The polygon's
/// coords are offsets from `face_anchor`; we shift into the
/// viewport's frame via `viewport_anchor.shortest_to(face_anchor)`.
fn draw_face_tiled(
    polygon: &[(f32, f32)],
    face_anchor: TorusPoint,
    viewport_anchor: TorusPoint,
    color: Color,
    canvas_origin: Vec2,
    canvas_size: f32,
    rect_min: Vec2,
    rect_max: Vec2,
) {
    let tris = triangulate(polygon);
    if tris.is_empty() {
        return;
    }
    // Polygon in viewport's frame: shift each offset by the
    // face-anchor's offset from the viewport.
    let shift = viewport_anchor.shortest_to(face_anchor);
    let (mut min_x, mut min_y) = (f32::INFINITY, f32::INFINITY);
    let (mut max_x, mut max_y) = (f32::NEG_INFINITY, f32::NEG_INFINITY);
    for &(x, y) in polygon {
        let sx = shift.dx + x;
        let sy = shift.dy + y;
        min_x = min_x.min(sx);
        min_y = min_y.min(sy);
        max_x = max_x.max(sx);
        max_y = max_y.max(sy);
    }
    // We need every integer offset (oi, oj) such that the bbox +
    // (oi, oj) overlaps the canvas's visible (-0.5, 0.5] range.
    let ox_lo = (-0.5 - max_x).ceil() as i32;
    let ox_hi = (0.5 - min_x).floor() as i32;
    let oy_lo = (-0.5 - max_y).ceil() as i32;
    let oy_hi = (0.5 - min_y).floor() as i32;
    let ox_lo = ox_lo.max(-3);
    let ox_hi = ox_hi.min(3);
    let oy_lo = oy_lo.max(-3);
    let oy_hi = oy_hi.min(3);

    for oi in ox_lo..=ox_hi {
        for oj in oy_lo..=oy_hi {
            let ox = oi as f32;
            let oy = oj as f32;
            for tri in &tris {
                let v0 = rel_to_screen(
                    shift.dx + tri[0].0 + ox,
                    shift.dy + tri[0].1 + oy,
                    canvas_origin,
                    canvas_size,
                );
                let v1 = rel_to_screen(
                    shift.dx + tri[1].0 + ox,
                    shift.dy + tri[1].1 + oy,
                    canvas_origin,
                    canvas_size,
                );
                let v2 = rel_to_screen(
                    shift.dx + tri[2].0 + ox,
                    shift.dy + tri[2].1 + oy,
                    canvas_origin,
                    canvas_size,
                );
                let lo_x = v0.x.min(v1.x).min(v2.x);
                let hi_x = v0.x.max(v1.x).max(v2.x);
                let lo_y = v0.y.min(v1.y).min(v2.y);
                let hi_y = v0.y.max(v1.y).max(v2.y);
                if hi_x < rect_min.x || lo_x > rect_max.x || hi_y < rect_min.y || lo_y > rect_max.y
                {
                    continue;
                }
                draw_triangle(v0, v1, v2, color);
            }
        }
    }
}

/// Liang–Barsky line clipping against an axis-aligned rectangle.
fn clip_line_to_rect(p0: Vec2, p1: Vec2, rmin: Vec2, rmax: Vec2) -> Option<(Vec2, Vec2)> {
    let d = p1 - p0;
    let mut t_enter = 0.0_f32;
    let mut t_exit = 1.0_f32;
    let edges = [
        (d.x, rmin.x - p0.x, rmax.x - p0.x),
        (d.y, rmin.y - p0.y, rmax.y - p0.y),
    ];
    for (delta, q_min, q_max) in edges {
        if delta.abs() < 1e-9 {
            if q_min > 0.0 || q_max < 0.0 {
                return None;
            }
        } else {
            let t1 = q_min / delta;
            let t2 = q_max / delta;
            let (t_in, t_out) = if t1 < t2 { (t1, t2) } else { (t2, t1) };
            t_enter = t_enter.max(t_in);
            t_exit = t_exit.min(t_out);
            if t_enter > t_exit {
                return None;
            }
        }
    }
    Some((p0 + d * t_enter, p0 + d * t_exit))
}

fn draw_slider(app: &App) {
    let (pos, size) = slider_rect();
    draw_rectangle(pos.x, pos.y, size.x, size.y, Color::from_rgba(40, 40, 50, 255));
    draw_rectangle_lines(
        pos.x,
        pos.y,
        size.x,
        size.y,
        1.0,
        Color::from_rgba(90, 90, 110, 255),
    );
    let t = ((app.hit_radius_px - HIT_RADIUS_MIN_PX)
        / (HIT_RADIUS_MAX_PX - HIT_RADIUS_MIN_PX))
        .clamp(0.0, 1.0);
    let knob_x = pos.x + t * size.x;
    let knob_y = pos.y + size.y * 0.5;
    draw_circle(
        knob_x,
        knob_y,
        SLIDER_KNOB_RADIUS,
        Color::from_rgba(220, 200, 90, 255),
    );
    draw_circle_lines(
        knob_x,
        knob_y,
        SLIDER_KNOB_RADIUS,
        1.5,
        Color::from_rgba(20, 20, 30, 255),
    );
    let label = format!("hit radius: {:>4.1}px", app.hit_radius_px);
    draw_text(
        &label,
        pos.x,
        pos.y + size.y + 18.0,
        16.0,
        Color::from_rgba(180, 180, 200, 255),
    );
}

fn draw_hud(app: &App) {
    let n_glyphs = app.glyphs.len();
    let n_edges: usize = app.glyphs.iter().map(|g| g.edges.len()).sum();
    let n_faces: usize = app.glyphs.iter().map(|g| g.topological_face_count).sum();
    let undo_n = app.undo_stack.len();
    let redo_n = app.redo_stack.len();
    let summary = format!(
        "glyphs: {n_glyphs}  edges: {n_edges}  faces: {n_faces}    left-drag: draw / move    right-click: delete    ctrl-z: undo ({undo_n})  ctrl-shift-z: redo ({redo_n})"
    );
    draw_text(
        &summary,
        12.0,
        22.0,
        18.0,
        Color::from_rgba(180, 180, 200, 255),
    );
    for (i, g) in app.glyphs.iter().enumerate() {
        let v = g.vertices.len();
        let e = g.edges.len();
        let f = g.topological_face_count;
        let line = format!("  glyph {i}: V={v}  E={e}  F={f}");
        let y = 22.0 + 18.0 * (i + 1) as f32;
        draw_text(&line, 12.0, y, 16.0, Color::from_rgba(160, 160, 180, 255));
    }
}
