//! 构造几何:点/轴/坐标系标记(世界坐标,随视口叠加显示)。
//! 求值借 kernel 的解析查询(圆柱轴、圆弧心),放置变换经 `Node::world` 入世界。

use solidcraft_geom::Vec3;
use solidcraft_kernel::{FaceSurface, face_surface};

use crate::scene::Doc;

/// 一个构造元素。
#[derive(Clone, Debug)]
pub enum Construct {
    /// 点(顶点/圆心/任意点)。
    Point { name: String, p: Vec3 },
    /// 轴线:中心点 + 单位方向 + 半长。
    Axis { name: String, origin: Vec3, dir: Vec3, len: f64 },
    /// 坐标系:原点 + 三个单位轴(右手法则)。
    Csys { name: String, origin: Vec3, x: Vec3, y: Vec3, z: Vec3 },
}

impl Construct {
    pub fn name(&self) -> &str {
        match self {
            Construct::Point { name, .. } | Construct::Axis { name, .. } | Construct::Csys { name, .. } => name,
        }
    }

    /// 该元素上一点(标签定位)。
    pub fn anchor(&self) -> Vec3 {
        match self {
            Construct::Point { p, .. } => *p,
            Construct::Axis { origin, dir, len, .. } => *origin + *dir * *len,
            Construct::Csys { origin, .. } => *origin,
        }
    }
}

/// P/A/C + 序号自动命名(同类现有最大号 +1)。
pub fn next_name(existing: &[Construct], prefix: &str) -> String {
    let n = existing
        .iter()
        .filter_map(|c| c.name().strip_prefix(prefix).and_then(|s| s.parse::<u32>().ok()))
        .max()
        .unwrap_or(0)
        + 1;
    format!("{prefix}{n}")
}

// ---------------- 世界变换(B-rep 局部 → 烘焙后的世界) ----------------

pub fn xform_point(m: &[[f64; 4]; 4], p: Vec3) -> Vec3 {
    Vec3::new(
        m[0][0] * p.x + m[1][0] * p.y + m[2][0] * p.z + m[3][0],
        m[0][1] * p.x + m[1][1] * p.y + m[2][1] * p.z + m[3][1],
        m[0][2] * p.x + m[1][2] * p.y + m[2][2] * p.z + m[3][2],
    )
}

/// 方向只取 3×3(STEP 放置是刚体),再归一。
pub fn xform_dir(m: &[[f64; 4]; 4], d: Vec3) -> Option<Vec3> {
    Vec3::new(m[0][0] * d.x + m[1][0] * d.y + m[2][0] * d.z, m[0][1] * d.x + m[1][1] * d.y + m[2][1] * d.z, m[0][2] * d.x + m[1][2] * d.y + m[2][2] * d.z).normalized()
}

// ---------------- 边/面的解析几何(世界坐标) ----------------

/// 一条直边:世界折线近似共线时返回 (线上一点, 单位方向);折线弯则 None。
pub fn straight_edge(polyline: &[Vec3]) -> Option<(Vec3, Vec3)> {
    let (&a, &b) = (polyline.first()?, polyline.last()?);
    let dir = (b - a).normalized()?;
    let chord = (b - a).len();
    // 中间点到首尾连线的最大偏差:直线在显示网格公差内。
    let tol = chord.max(1.0) * 1e-3;
    if polyline.iter().any(|p| (*p - a).cross(dir).len() > tol) {
        return None;
    }
    Some((a, dir))
}

/// 圆弧边的圆心与轴(世界):局部 `edge_circle` 经放置变换。
pub fn circle_edge(doc: &Doc, path: &[usize], edge: usize) -> Option<(Vec3, Vec3)> {
    let node = doc.node_at(path)?;
    let body = node.body.as_ref()?;
    let c = solidcraft_kernel::edge_circle(body, edge)?;
    let center = xform_point(&node.world, c.center);
    let axis = xform_dir(&node.world, Vec3::new(c.axis.x, c.axis.y, c.axis.z))?;
    Some((center, axis))
}

/// 圆类面的中心与轴(世界):圆柱/圆锥/圆环;平面与球面 None(球无轴向)。
pub fn round_face_axis(doc: &Doc, path: &[usize], face: u32) -> Option<(Vec3, Vec3)> {
    let node = doc.node_at(path)?;
    let body = node.body.as_ref()?;
    match face_surface(body, face as usize)? {
        FaceSurface::Cylinder { origin, axis, .. } => {
            Some((xform_point(&node.world, origin), xform_dir(&node.world, Vec3::new(axis.x, axis.y, axis.z))?))
        }
        FaceSurface::Cone { apex, axis, .. } => {
            Some((xform_point(&node.world, apex), xform_dir(&node.world, Vec3::new(axis.x, axis.y, axis.z))?))
        }
        FaceSurface::Torus { center, axis, .. } => {
            Some((xform_point(&node.world, center), xform_dir(&node.world, Vec3::new(axis.x, axis.y, axis.z))?))
        }
        FaceSurface::Sphere { .. } | FaceSurface::Plane { .. } => None,
    }
}

/// 圆类面的圆心点(世界):圆柱/圆环给中心,球给球心,平面 None。
pub fn round_face_center(doc: &Doc, path: &[usize], face: u32) -> Option<Vec3> {
    let node = doc.node_at(path)?;
    let body = node.body.as_ref()?;
    match face_surface(body, face as usize)? {
        FaceSurface::Cylinder { origin, .. } | FaceSurface::Torus { center: origin, .. } => Some(xform_point(&node.world, origin)),
        FaceSurface::Cone { apex, .. } => Some(xform_point(&node.world, apex)),
        FaceSurface::Sphere { center, .. } => Some(xform_point(&node.world, center)),
        FaceSurface::Plane { .. } => None,
    }
}

/// 坐标系的 X/Y:世界 X 投影到垂直 Z(退化时用 Y),照 SolidCraft 的 resolve_csys。
pub fn csys_axes(z: Vec3) -> Option<(Vec3, Vec3)> {
    let z = z.normalized()?;
    let x = (Vec3::X - z * Vec3::X.dot(z)).normalized().or_else(|| (Vec3::Y - z * Vec3::Y.dot(z)).normalized())?;
    let y = z.cross(x).normalized()?;
    Some((x, y))
}
