//! Glyph Game — MVP drawing surface on the flat torus.
//!
//! Click and drag on empty space to draw a new line segment. Click on an
//! existing endpoint (small circle) to drag it; click on a segment's
//! interior to drag the whole segment. Right-click on a segment to delete
//! it. The viewport is a square fundamental domain of the torus, and
//! anything that leaves one edge reappears on the opposite edge.

use macroquad::prelude::*;

mod torus;
use torus::{TorusPoint, TorusSegment, TorusVec};

const ENDPOINT_RADIUS: f32 = 6.0;
const HIT_RADIUS_PX: f32 = 10.0;
const LINE_WIDTH: f32 = 2.5;

/// What the user is currently doing with the mouse.
enum Drag {
    /// Drawing a brand-new segment; we own its growing displacement.
    DrawNew { start: TorusPoint, disp: TorusVec },
    /// Modifying segment `idx` in some way.
    Modify { idx: usize, kind: ModifyKind },
}

#[derive(Copy, Clone)]
enum ModifyKind {
    Start,
    End,
    Whole,
}

struct App {
    segments: Vec<TorusSegment>,
    drag: Option<Drag>,
}

impl App {
    fn new() -> Self {
        Self {
            segments: Vec::new(),
            drag: None,
        }
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

        handle_input(&mut app, mouse_px, mouse_torus, in_canvas, prev_mouse_px,
                     canvas_size);

        clear_background(Color::from_rgba(18, 18, 22, 255));
        draw_canvas_frame(canvas_origin, canvas_size);
        draw_scene(&app, canvas_origin, canvas_size);
        draw_hud(&app);

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

    // Right-click: delete the segment under the cursor (if any).
    if is_mouse_button_pressed(MouseButton::Right) && in_canvas {
        if let Some(idx) = pick_segment(&app.segments, mouse_torus) {
            app.segments.remove(idx);
        }
        return;
    }

    if just_pressed_left && in_canvas {
        // Endpoint hit takes priority over segment-body hit.
        let hit_radius = HIT_RADIUS_PX / canvas_size;
        if let Some((idx, kind)) = pick_endpoint(&app.segments, mouse_torus, hit_radius) {
            app.drag = Some(Drag::Modify { idx, kind });
        } else if let Some(idx) = pick_segment(&app.segments, mouse_torus) {
            app.drag = Some(Drag::Modify { idx, kind: ModifyKind::Whole });
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
                Drag::Modify { idx, kind } => {
                    let seg = &mut app.segments[*idx];
                    *seg = match kind {
                        ModifyKind::Start => seg.move_start(mouse_delta),
                        ModifyKind::End => seg.move_end(mouse_delta),
                        ModifyKind::Whole => seg.translated(mouse_delta),
                    };
                }
            }
        }
    }

    if is_mouse_button_released(MouseButton::Left) {
        if let Some(Drag::DrawNew { start, disp }) = app.drag.take() {
            // Discard zero-length scratches.
            if disp.length() > 0.005 {
                app.segments.push(TorusSegment { start, disp });
            }
        }
    }
}

/// Hit-test: return the index and which endpoint of the closest segment
/// whose endpoint is within `radius` of `p` (in torus distance).
fn pick_endpoint(
    segments: &[TorusSegment],
    p: TorusPoint,
    radius: f32,
) -> Option<(usize, ModifyKind)> {
    let mut best: Option<(f32, usize, ModifyKind)> = None;
    for (i, seg) in segments.iter().enumerate() {
        for (kind, ep) in [
            (ModifyKind::Start, seg.start),
            (ModifyKind::End, seg.end()),
        ] {
            let d = p.distance_to(ep);
            if d <= radius && best.map_or(true, |(bd, _, _)| d < bd) {
                best = Some((d, i, kind));
            }
        }
    }
    best.map(|(_, i, k)| (i, k))
}

/// Hit-test: which segment's body is the cursor over? We check distance
/// from `p` to each lift of the segment in the universal cover.
fn pick_segment(segments: &[TorusSegment], p: TorusPoint) -> Option<usize> {
    let hit = 0.012_f32; // ~1.2% of viewport
    let mut best: Option<(f32, usize)> = None;
    for (i, seg) in segments.iter().enumerate() {
        let d = distance_point_to_segment(seg, p);
        if d <= hit && best.map_or(true, |(bd, _)| d < bd) {
            best = Some((d, i));
        }
    }
    best.map(|(_, i)| i)
}

/// Shortest distance from a torus point `p` to a torus segment, computed
/// by minimizing over all visible lifts of the segment in the universal
/// cover (with `p` lifted to its canonical representative).
fn distance_point_to_segment(seg: &TorusSegment, p: TorusPoint) -> f32 {
    let px = p.x();
    let py = p.y();
    let mut best = f32::INFINITY;
    // The point also has 9 lifts; iterating the segment's lifts is
    // equivalent (since the relevant invariant is the integer offset
    // between point and segment).
    for ((ax, ay), (bx, by)) in seg.visible_lifts() {
        let d = dist_point_to_seg_2d(px, py, ax, ay, bx, by);
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

    let mut to_draw: Vec<TorusSegment> = app.segments.clone();
    // Also draw the in-progress new segment, if any.
    if let Some(Drag::DrawNew { start, disp }) = &app.drag {
        to_draw.push(TorusSegment {
            start: *start,
            disp: *disp,
        });
    }

    for seg in &to_draw {
        draw_segment_tiled(seg, origin, size, rect_min, rect_max);
    }
    for seg in &to_draw {
        draw_endpoint_tiled(seg.start, origin, size, rect_min, rect_max, ENDPOINT_COLOR);
        draw_endpoint_tiled(seg.end(), origin, size, rect_min, rect_max, ENDPOINT_COLOR);
    }
}

const ENDPOINT_COLOR: Color = Color::new(0.9, 0.85, 0.3, 1.0);
const LINE_COLOR: Color = Color::new(0.85, 0.9, 1.0, 1.0);

fn draw_segment_tiled(seg: &TorusSegment, origin: Vec2, size: f32, rect_min: Vec2, rect_max: Vec2) {
    for ((ax, ay), (bx, by)) in seg.visible_lifts() {
        let pa = origin + vec2(ax, ay) * size;
        let pb = origin + vec2(bx, by) * size;
        if let Some((c0, c1)) = clip_line_to_rect(pa, pb, rect_min, rect_max) {
            draw_line(c0.x, c0.y, c1.x, c1.y, LINE_WIDTH, LINE_COLOR);
        }
    }
}

fn draw_endpoint_tiled(p: TorusPoint, origin: Vec2, size: f32, rect_min: Vec2, rect_max: Vec2, color: Color) {
    for i in -1..=1 {
        for j in -1..=1 {
            let sx = origin.x + (p.x() + i as f32) * size;
            let sy = origin.y + (p.y() + j as f32) * size;
            // Don't draw far-away copies.
            if sx + ENDPOINT_RADIUS < rect_min.x || sx - ENDPOINT_RADIUS > rect_max.x {
                continue;
            }
            if sy + ENDPOINT_RADIUS < rect_min.y || sy - ENDPOINT_RADIUS > rect_max.y {
                continue;
            }
            draw_circle(sx, sy, ENDPOINT_RADIUS, color);
            draw_circle_lines(sx, sy, ENDPOINT_RADIUS, 1.5, Color::from_rgba(20, 20, 30, 255));
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

fn draw_hud(app: &App) {
    let n = app.segments.len();
    let line = format!(
        "segments: {n}    left-drag: draw / move    right-click: delete"
    );
    draw_text(&line, 12.0, 22.0, 18.0, Color::from_rgba(180, 180, 200, 255));
}
