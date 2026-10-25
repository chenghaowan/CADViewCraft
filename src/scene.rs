//! 文档模型:STEP 导入 → 节点树(显隐/烘焙变换的显示网格)→ render::Scene。

use std::sync::Arc;
use std::time::Instant;

use solidcraft_geom::Mesh;
use solidcraft_geom::Vec3;
use solidcraft_kernel::{ImportedBody, StepNode};
use solidcraft_render::{Rgb, Scene, SceneLine, SceneMesh};

/// 模型树节点。装配实例变换在加载时烘焙进网格顶点,因此渲染侧是扁平的。
pub struct Node {
    pub name: String,
    pub visible: bool,
    pub mesh: Option<Arc<Mesh>>,
    /// kernel 体(测量用;网格体/失败细分时为 None)。与 mesh 同源。
    pub body: Option<Arc<solidcraft_kernel::Body>>,
    /// 该体的累计放置变换(列主序 4×4):body 的 B-rep 查询(圆柱轴/圆心)是局部
    /// 坐标,经它变换到与 mesh 一致的世界坐标。
    pub world: [[f64; 4]; 4],
    pub color: Rgb,
    pub children: Vec<Node>,
}

impl Node {
    /// 深度优先遍历(含自身)。
    pub fn walk(&self, f: &mut impl FnMut(&Node)) {
        f(self);
        for c in &self.children {
            c.walk(f);
        }
    }
}

pub struct Doc {
    pub name: String,
    pub schema: String,
    pub roots: Vec<Node>,
    pub warnings: Vec<String>,
    pub load_ms: u128,
}

impl Doc {
    /// 路径(每层 child 下标)指向的节点。
    pub fn node_at(&self, path: &[usize]) -> Option<&Node> {
        let mut nodes: &[Node] = &self.roots;
        let mut found = None;
        for (i, idx) in path.iter().enumerate() {
            found = nodes.get(*idx);
            nodes = match found {
                Some(n) => &n.children,
                None => return None,
            };
            if i + 1 == path.len() {
                return found;
            }
        }
        found
    }

    /// 路径指向节点的可见性(路径不存在返回 None)。
    pub fn node_visible(&self, path: &[usize]) -> Option<bool> {
        self.node_at(path).map(|n| n.visible)
    }

    pub fn triangles(&self) -> usize {
        let mut n = 0;
        self.walk_all(&mut |node| {
            if let Some(m) = &node.mesh {
                n += m.triangles.len();
            }
        });
        n
    }

    pub fn bodies(&self) -> usize {
        let mut n = 0;
        self.walk_all(&mut |node| n += usize::from(node.mesh.is_some()));
        n
    }

    fn walk_all(&self, f: &mut impl FnMut(&Node)) {
        for r in &self.roots {
            r.walk(f);
        }
    }

    /// 世界包围盒(min, max),烘焙后的网格顶点范围。
    pub fn bbox(&self) -> Option<(Vec3, Vec3)> {
        let mut bb: Option<(Vec3, Vec3)> = None;
        self.walk_all(&mut |node| {
            let Some(m) = &node.mesh else { return };
            for p in &m.positions {
                let (min, max) = bb.get_or_insert((*p, *p));
                min.x = min.x.min(p.x);
                min.y = min.y.min(p.y);
                min.z = min.z.min(p.z);
                max.x = max.x.max(p.x);
                max.y = max.y.max(p.y);
                max.z = max.z.max(p.z);
            }
        });
        bb
    }
}

/// 无外观时的模型默认色(Mayo 风格的钢蓝灰)。
const DEFAULT_COLOR: Rgb = Rgb(146, 163, 189);
const EDGE_COLOR: Rgb = Rgb(64, 68, 76);

/// 在后台线程调用:读文件(IGES 先转 STEP)→ kernel 导入 → 逐体细分并烘焙实例变换。
pub fn load(path: &str) -> Result<Doc, String> {
    let t = Instant::now();
    let bytes = std::fs::read(path).map_err(|e| format!("{path}: {e}"))?;
    // kernel 收文本;非 UTF-8 时按 Latin-1 逐字节映射(工业文件的注释区常见本地编码)。
    let decode = |b: Vec<u8>| match String::from_utf8(b) {
        Ok(s) => s,
        Err(e) => e.into_bytes().into_iter().map(|c| c as char).collect(),
    };
    let ext = std::path::Path::new(path)
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    let mut iges_warnings = Vec::new();
    let text = match ext.as_str() {
        "igs" | "iges" => {
            let (step, warns) = solidcraft_kernel::iges_to_step(&decode(bytes))
                .map_err(|e| format!("IGES 转换失败: {e}"))?;
            iges_warnings = warns;
            step
        }
        _ => decode(bytes),
    };
    let imp = solidcraft_kernel::step_import_shared(&text).map_err(|e| e.to_string())?;
    let name = std::path::Path::new(path)
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string());
    let mut warnings = imp.warnings.clone();
    warnings.extend(iges_warnings);
    let mut doc = Doc {
        name,
        schema: imp.schema.clone(),
        roots: Vec::new(),
        warnings,
        load_ms: t.elapsed().as_millis(),
    };
    for root in &imp.tree {
        doc.roots.push(convert(root, &imp.bodies, IDENTITY));
    }
    doc.load_ms = t.elapsed().as_millis();
    Ok(doc)
}

const IDENTITY: [[f64; 4]; 4] = [
    [1.0, 0.0, 0.0, 0.0],
    [0.0, 1.0, 0.0, 0.0],
    [0.0, 0.0, 1.0, 0.0],
    [0.0, 0.0, 0.0, 1.0],
];

fn convert(node: &StepNode, bodies: &[ImportedBody], parent: [[f64; 4]; 4]) -> Node {
    let m = mul(parent, node.transform);
    let mut children: Vec<Node> = node
        .bodies
        .iter()
        .filter_map(|&bi| {
            let ib = bodies.get(bi)?;
            let mesh = ib.body.tessellate(0.1).ok().map(|t| Arc::new(transform_mesh(&t, &m)));
            Some(Node {
                name: ib.name.clone(),
                visible: true,
                mesh,
                body: Some(Arc::new(ib.body.clone())),
                world: m,
                color: ib.color.map(|c| Rgb((c[0] * 255.0) as u8, (c[1] * 255.0) as u8, (c[2] * 255.0) as u8)).unwrap_or(DEFAULT_COLOR),
                children: Vec::new(),
            })
        })
        .collect();
    for child in &node.children {
        children.push(convert(child, bodies, m));
    }
    Node {
        name: node.name.clone(),
        visible: true,
        mesh: None,
        body: None,
        world: m,
        color: DEFAULT_COLOR,
        children,
    }
}

/// 列主序 4×4 相乘。
fn mul(a: [[f64; 4]; 4], b: [[f64; 4]; 4]) -> [[f64; 4]; 4] {
    let mut out = [[0.0f64; 4]; 4];
    for (c, col) in out.iter_mut().enumerate() {
        for (r, v) in col.iter_mut().enumerate() {
            *v = (0..4).map(|k| a[k][r] * b[c][k]).sum();
        }
    }
    out
}

/// 用变换矩阵移动网格顶点(法线用同一 3×3,STEP 放置是刚体/仿射,再归一)。
fn transform_mesh(m: &Mesh, t: &[[f64; 4]; 4]) -> Mesh {
    let xform = |p: Vec3| -> Vec3 {
        let h = [p.x, p.y, p.z, 1.0];
        Vec3::new(
            (0..4).map(|c| t[c][0] * h[c]).sum::<f64>(),
            (0..4).map(|c| t[c][1] * h[c]).sum::<f64>(),
            (0..4).map(|c| t[c][2] * h[c]).sum::<f64>(),
        )
    };
    Mesh {
        positions: m.positions.iter().map(|&p| xform(p)).collect(),
        normals: m
            .normals
            .iter()
            .map(|&n| xform(n).normalized().unwrap_or(n))
            .collect(),
        triangles: m.triangles.clone(),
        tri_face: m.tri_face.clone(),
        edges: m
            .edges
            .iter()
            .map(|e| e.iter().map(|&p| xform(p)).collect())
            .collect(),
        seams: m.seams.clone(),
        ..Mesh::default()
    }
}

/// 把文档的可见节点压成渲染场景:着色体 + 边线 + 背景网格线 + 地面网格。
pub fn build_scene(doc: &Doc) -> Scene {
    let mut sc = Scene {
        background: Some((Rgb(30, 32, 36), Rgb(46, 49, 55))),
        ..Scene::default()
    };
    let mut bb: Option<(Vec3, Vec3)> = None;
    doc.walk_all(&mut |node| {
        if !node.visible {
            return;
        }
        let Some(mesh) = &node.mesh else { return };
        grow(&mut bb, mesh);
        sc.meshes.push(SceneMesh::solid(Arc::clone(mesh), node.color));
        for e in &mesh.edges {
            if e.len() < 2 {
                continue;
            }
            sc.lines.push(SceneLine {
                points: e.clone(),
                color: EDGE_COLOR,
                width: 1.0,
                on_top: false,
            });
        }
    });
    let Some((min, max)) = bb else { return sc };
    let center = Vec3::new((min.x + max.x) * 0.5, (min.y + max.y) * 0.5, (min.z + max.z) * 0.5);
    let radius = 0.5 * ((max.x - min.x).powi(2) + (max.y - min.y).powi(2) + (max.z - min.z).powi(2)).sqrt();
    sc.radius = radius.max(1.0);
    sc.grid = ground_grid(min.z, radius, center);
    sc
}

fn grow(bb: &mut Option<(Vec3, Vec3)>, m: &Mesh) {
    for p in &m.positions {
        let (min, max) = bb.get_or_insert((*p, *p));
        min.x = min.x.min(p.x);
        min.y = min.y.min(p.y);
        min.z = min.z.min(p.z);
        max.x = max.x.max(p.x);
        max.y = max.y.max(p.y);
        max.z = max.z.max(p.z);
    }
}

/// XY 平面(Z-up)的地面网格:尺度取 10 的幂,使网格线在 20~40 条之间。
fn ground_grid(z: f64, radius: f64, center: Vec3) -> Vec<SceneLine> {
    let step = 10f64.powf(radius.log10().floor()) * (if radius / 10f64.powf(radius.log10().floor()) < 2.0 { 5.0 } else { 1.0 });
    let half = (radius * 1.5 / step).ceil() * step;
    let z0 = z - radius * 0.15;
    let mut lines = Vec::new();
    let mut i = -half;
    while i <= half + 1e-9 {
        let on_axis = i.abs() < 1e-9;
        let color = if on_axis { Rgb(70, 58, 58) } else { Rgb(50, 53, 58) };
        lines.push(SceneLine {
            points: vec![Vec3::new(center.x + i, center.y - half, z0), Vec3::new(center.x + i, center.y + half, z0)],
            color,
            width: 1.0,
            on_top: false,
        });
        lines.push(SceneLine {
            points: vec![Vec3::new(center.x - half, center.y + i, z0), Vec3::new(center.x + half, center.y + i, z0)],
            color,
            width: 1.0,
            on_top: false,
        });
        i += step;
    }
    lines
}
