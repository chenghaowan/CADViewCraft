//! 屏幕拾取:相机射线与烘焙网格求交(Möller–Trumbore,双面)。

use solidcraft_geom::Vec3;
use solidcraft_render::Camera;

use crate::scene::{Doc, Node};

#[derive(Clone)]
pub struct Hit {
    /// 节点路径(每层 child 下标),与模型树/显隐一致。
    pub path: Vec<usize>,
    /// 命中的 B-rep 面号(网格的 tri_face)。
    pub face: u32,
    /// 世界坐标命中点(测量里程碑的输入)。
    #[allow(dead_code)]
    pub point: Vec3,
    #[allow(dead_code)]
    pub dist: f64,
}

/// 在 (x, y) 像素处拾取;相机与尺寸用同一单位(egui 逻辑像素即可)。
/// `clip` 为断面 (法向, d):三点全在剖掉一侧(n·p > d)的三角形不参与命中。
pub fn pick(
    doc: &Doc,
    cam: &Camera,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    clip: Option<(Vec3, f64)>,
) -> Option<Hit> {
    let (origin, dir) = cam.ray(x, y, w, h);
    let mut best: Option<(f64, Vec<usize>, u32, Vec3)> = None;
    walk(&doc.roots, Vec::new(), &mut |node, path| {
        let Some(m) = &node.mesh else { return };
        for (ti, tri) in m.triangles.iter().enumerate() {
            let [ia, ib, ic] = *tri;
            let (Some(a), Some(b), Some(c)) = (
                m.positions.get(ia as usize),
                m.positions.get(ib as usize),
                m.positions.get(ic as usize),
            ) else {
                continue;
            };
            if let Some((n, d)) = clip
                && a.dot(n) > d
                && b.dot(n) > d
                && c.dot(n) > d
            {
                continue;
            }
            if let Some(t) = ray_tri(origin, dir, *a, *b, *c)
                && best.as_ref().is_none_or(|(bt, ..)| t < *bt)
            {
                best = Some((t, path.to_vec(), m.tri_face.get(ti).copied().unwrap_or(0), origin + dir * t));
            }
        }
    });
    let (dist, path, face, point) = best?;
    Some(Hit { path, face, point, dist })
}

fn walk(nodes: &[Node], path: Vec<usize>, f: &mut impl FnMut(&Node, &[usize])) {
    for (i, node) in nodes.iter().enumerate() {
        let mut p = path.clone();
        p.push(i);
        f(node, &p);
        walk(&node.children, p, f);
    }
}

/// 拾取到的网格顶点(世界坐标)。
#[derive(Clone)]
pub struct VertexHit {
    pub point: Vec3,
}

/// 拾取到的边:体路径 + 边号(与 `Body::edges(0.1)` 同序,即显示网格的边序)
/// + 该边的世界折线(直线/圆弧判定用)。
#[derive(Clone)]
pub struct EdgeHit {
    pub path: Vec<usize>,
    pub edge: usize,
    pub polyline: Vec<Vec3>,
}

/// 屏幕距离 ≤ `tol_px` 像素的最近网格顶点(无深度遮挡判定:顶点拾取常规做法)。
pub fn pick_vertex(
    doc: &Doc,
    cam: &Camera,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    radius: f64,
    tol_px: f64,
) -> Option<VertexHit> {
    let mut best: Option<(f64, f64, Vec3)> = None; // (屏幕距离, 相机深度, 顶点)
    walk(&doc.roots, Vec::new(), &mut |node, _path| {
        if !node.visible {
            return;
        }
        let Some(m) = &node.mesh else { return };
        for p in &m.positions {
            let Some((sx, sy, depth)) = cam.to_screen(*p, w, h, radius) else { continue };
            let d = ((sx - x).powi(2) + (sy - y).powi(2)).sqrt();
            if d > tol_px {
                continue;
            }
            let take = best.as_ref().is_none_or(|(bd, bdep, ..)| {
                d < *bd - 0.5 || (d - bd).abs() <= 0.5 && depth < *bdep
            });
            if take {
                best = Some((d, depth, *p));
            }
        }
    });
    let (_, _, point) = best?;
    Some(VertexHit { point })
}

/// 屏幕距离 ≤ `tol_px` 像素的最近边(对每条显示网格边折线的每段做屏幕投影判定)。
pub fn pick_edge(
    doc: &Doc,
    cam: &Camera,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    radius: f64,
    tol_px: f64,
) -> Option<EdgeHit> {
    let mut best: Option<(f64, Vec<usize>, usize, Vec<Vec3>)> = None;
    walk(&doc.roots, Vec::new(), &mut |node, path| {
        if !node.visible {
            return;
        }
        let Some(m) = &node.mesh else { return };
        for (ei, e) in m.edges.iter().enumerate() {
            if e.len() < 2 {
                continue;
            }
            for seg in e.windows(2) {
                let (a, b) = (seg[0], seg[1]);
                let Some(d) = screen_dist_to_seg(cam, x, y, w, h, radius, a, b) else { continue };
                if d > tol_px {
                    continue;
                }
                if best.as_ref().is_none_or(|(bd, ..)| d < *bd) {
                    best = Some((d, path.to_vec(), ei, e.clone()));
                }
            }
        }
    });
    let (_, path, edge, polyline) = best?;
    Some(EdgeHit { path, edge, polyline })
}

/// 点到屏幕上线段 ab 的最近距离(像素)。段两端任一不可投影则 None。
fn screen_dist_to_seg(
    cam: &Camera,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    radius: f64,
    a: Vec3,
    b: Vec3,
) -> Option<f64> {
    let (ax, ay, _) = cam.to_screen(a, w, h, radius)?;
    let (bx, by, _) = cam.to_screen(b, w, h, radius)?;
    let (dx, dy) = (bx - ax, by - ay);
    let len2 = dx * dx + dy * dy;
    let t = if len2 < 1e-9 { 0.0 } else { (((x - ax) * dx + (y - ay) * dy) / len2).clamp(0.0, 1.0) };
    let (sx, sy) = (ax + dx * t, ay + dy * t);
    Some(((sx - x).powi(2) + (sy - y).powi(2)).sqrt())
}

/// 返回射线参数 t(>0 命中)。
fn ray_tri(o: Vec3, d: Vec3, a: Vec3, b: Vec3, c: Vec3) -> Option<f64> {
    let e1 = b - a;
    let e2 = c - a;
    let p = d.cross(e2);
    let det = e1.dot(p);
    if det.abs() < 1e-12 {
        return None;
    }
    let inv = 1.0 / det;
    let s = o - a;
    let u = s.dot(p) * inv;
    if !(0.0..=1.0).contains(&u) {
        return None;
    }
    let q = s.cross(e1);
    let v = d.dot(q) * inv;
    if v < 0.0 || u + v > 1.0 {
        return None;
    }
    let t = e2.dot(q) * inv;
    (t > 1e-6).then_some(t)
}
