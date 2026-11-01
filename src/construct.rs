//! 构造几何:点/轴/坐标系标记(世界坐标,随视口叠加显示)。
//! 求值借 kernel 的解析查询(圆柱轴、圆弧心),放置变换经 `Node::world` 入世界。

use solidcraft_geom::Vec3;
use solidcraft_kernel::{FaceSurface, face_surface};

use crate::scene::Doc;

/// 一个构造元素。
#[derive(Clone, Debug)]
pub enum Construct {
    /// 点(顶点/圆心/任意点)。
    Point { name: String, p: Vec3, visible: bool },
    /// 轴线:中心点 + 单位方向 + 半长。
    Axis { name: String, origin: Vec3, dir: Vec3, len: f64, visible: bool },
    /// 坐标系:原点 + 三个单位轴(右手法则);`def` 为关联定义,去关联后 None(数字冻结)。
    Csys { name: String, origin: Vec3, x: Vec3, y: Vec3, z: Vec3, def: Option<CsysDef>, visible: bool },
}

// ---------------- 坐标系的关联定义(照 CATIA:默认关联,可去关联) ----------------

/// 坐标系原点的引用(拾取即引用,绘制前现算)。
#[derive(Clone, Debug, PartialEq)]
pub enum OriginRef {
    /// 网格顶点:节点路径 + 拾取点(解析取该节点距 `at` 最近的顶点)。
    Vertex { path: Vec<usize>, at: Vec3 },
    /// 圆弧/圆边的圆心。
    ArcCenter { path: Vec<usize>, edge: usize, at: Vec3 },
    /// 圆类面(圆柱/圆锥/圆环/球)的中心/锥顶/球心。
    FaceCenter { path: Vec<usize>, face: u32, at: Vec3 },
    /// 面上任意点(解析取拾取点)。
    SurfacePoint { path: Vec<usize>, face: u32, at: Vec3 },
    /// 构造点 P{n}(按名字引用,免受删除引起的下标漂移)。
    ConstructPoint { name: String },
}

/// Z 方向的引用。
#[derive(Clone, Debug, PartialEq)]
pub enum DirRef {
    /// 固定方向(数字输入/世界轴/去关联)。
    Fixed(Vec3),
    /// 直边方向。
    Edge { path: Vec<usize>, edge: usize },
    /// 圆弧/圆边的轴向。
    ArcAxis { path: Vec<usize>, edge: usize },
    /// 圆柱/圆锥/圆环面的轴向。
    FaceAxis { path: Vec<usize>, face: u32 },
    /// 构造轴 A{n}。
    ConstructAxis { name: String },
}

/// 坐标系的关联定义:原点 + Z 方向(X = 世界 X 投影 ⊥ Z,Y 补右手)。
#[derive(Clone, Debug, PartialEq)]
pub struct CsysDef {
    pub origin: OriginRef,
    pub z: DirRef,
}

impl OriginRef {
    /// 列表/对话框里显示的引用描述。
    pub fn describe(&self) -> String {
        let p3 = |p: &Vec3| format!("({:.1}, {:.1}, {:.1})", p.x, p.y, p.z);
        match self {
            OriginRef::Vertex { at, .. } => format!("顶点 {}", p3(at)),
            OriginRef::ArcCenter { .. } => "圆弧圆心".into(),
            OriginRef::FaceCenter { .. } => "面中心".into(),
            OriginRef::SurfacePoint { at, .. } => format!("面上点 {}", p3(at)),
            OriginRef::ConstructPoint { name } => format!("构造点 {name}"),
        }
    }
}

impl DirRef {
    /// 列表/对话框里显示的引用描述。
    pub fn describe(&self) -> String {
        let p3 = |d: &Vec3| format!("({:.2}, {:.2}, {:.2})", d.x, d.y, d.z);
        match self {
            DirRef::Fixed(d) => format!("方向 {}", p3(d)),
            DirRef::Edge { .. } => "直边方向".into(),
            DirRef::ArcAxis { .. } => "圆弧轴向".into(),
            DirRef::FaceAxis { .. } => "圆柱/圆锥轴".into(),
            DirRef::ConstructAxis { name } => format!("构造轴 {name}"),
        }
    }
}

impl Construct {
    pub fn name(&self) -> &str {
        match self {
            Construct::Point { name, .. } | Construct::Axis { name, .. } | Construct::Csys { name, .. } => name,
        }
    }

    /// 是否显示(几何特征列表的眼睛)。
    pub fn visible(&self) -> bool {
        match self {
            Construct::Point { visible, .. } | Construct::Axis { visible, .. } | Construct::Csys { visible, .. } => *visible,
        }
    }

    pub fn set_visible(&mut self, visible: bool) {
        match self {
            Construct::Point { visible: v, .. } | Construct::Axis { visible: v, .. } | Construct::Csys { visible: v, .. } => *v = visible,
        }
    }

    /// 坐标系去关联:丢掉关联定义,数字保留当前值(其他类型无操作)。
    pub fn drop_def(&mut self) {
        if let Construct::Csys { def, .. } = self {
            *def = None;
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

/// 解析原点引用为世界坐标;引用失效(节点/元素被删)返回 None。
pub fn resolve_origin(doc: &Doc, constructs: &[Construct], r: &OriginRef) -> Option<Vec3> {
    match r {
        OriginRef::Vertex { path, at } => {
            // 网格顶点已烘焙成世界坐标:取该节点距拾取点最近的顶点。
            let node = doc.node_at(path)?;
            node.mesh
                .as_ref()?
                .positions
                .iter()
                .copied()
                .min_by(|a, b| (*a - *at).len2().partial_cmp(&(*b - *at).len2()).unwrap_or(std::cmp::Ordering::Equal))
        }
        OriginRef::ArcCenter { path, edge, .. } => circle_edge(doc, path, *edge).map(|(c, _)| c),
        OriginRef::FaceCenter { path, face, .. } => round_face_center(doc, path, *face),
        OriginRef::SurfacePoint { at, .. } => Some(*at),
        OriginRef::ConstructPoint { name } => constructs.iter().find_map(|c| match c {
            Construct::Point { name: n, p, .. } if n == name => Some(*p),
            _ => None,
        }),
    }
}

/// 解析 Z 方向引用为单位向量;引用失效返回 None。
pub fn resolve_dir(doc: &Doc, constructs: &[Construct], r: &DirRef) -> Option<Vec3> {
    match r {
        DirRef::Fixed(d) => d.normalized(),
        DirRef::Edge { path, edge } => {
            // 与 pick_edge 同源:显示网格的边折线。
            let node = doc.node_at(path)?;
            let poly = node.mesh.as_ref()?.edges.get(*edge)?;
            straight_edge(poly).map(|(_, dir)| dir)
        }
        DirRef::ArcAxis { path, edge } => circle_edge(doc, path, *edge).map(|(_, a)| a),
        DirRef::FaceAxis { path, face } => round_face_axis(doc, path, *face).map(|(_, a)| a),
        DirRef::ConstructAxis { name } => constructs.iter().find_map(|c| match c {
            Construct::Axis { name: n, dir, .. } if n == name => dir.normalized(),
            _ => None,
        }),
    }
}

/// 解析整个关联定义为世界系 (原点, X, Y, Z)。
pub fn resolve_csys(doc: &Doc, constructs: &[Construct], def: &CsysDef) -> Option<(Vec3, Vec3, Vec3, Vec3)> {
    let o = resolve_origin(doc, constructs, &def.origin)?;
    let z = resolve_dir(doc, constructs, &def.z)?;
    let (x, y) = csys_axes(z)?;
    Some((o, x, y, z))
}
