//! 文档模型:导入 → 节点树(显隐/烘焙变换的显示网格)→ render::Scene。
//! 导入格式照 SolidCraft 的格式注册表(io::formats):B-rep 走 STEP 链
//! (STEP 直读,IGES/FreeCAD/Rhino 先转成 STEP 文本),网格(STL/3MF/OBJ/glTF)
//! 走 mesh_body 建成网格体;DXF/SVG 草稿格式没有对应的草图功能,不收。

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

/// 无外观时的模型默认色(钢蓝灰)。
const DEFAULT_COLOR: Rgb = Rgb(146, 163, 189);

/// 网格类导入的扩展名(照 SolidCraft io::is_mesh_path)。
const MESH_EXTS: [&str; 5] = ["stl", "obj", "3mf", "glb", "gltf"];

/// 文件名(带扩展名;读取器用它认格式)。
fn file_name(path: &str) -> String {
    std::path::Path::new(path)
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string())
}

/// 在后台线程调用:读文件 → kernel 导入 → 逐体细分并烘焙实例变换。
/// B-rep 格式(STEP 直读;IGES/FreeCAD/Rhino 先转成 STEP 文本;CATPart 走本程序
/// 的 catpart 转换支路转成 STEP 文本,并在源文件旁落一份同名 .stp 中间文件——
/// solidcraft 只见到 STP),网格格式
/// (STL/3MF/OBJ/glTF)建成网格体,其余扩展名兜底按 STEP 读(与既往行为一致)。
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
    let name = file_name(path);
    let mut warnings = Vec::new();
    let text = match ext.as_str() {
        "igs" | "iges" => {
            let (step, warns) = solidcraft_kernel::iges_to_step(&decode(bytes))
                .map_err(|e| format!("IGES 转换失败: {e}"))?;
            warnings.extend(warns);
            step
        }
        // FreeCAD/Rhino:cadmpeg 解码后重述为 STEP 文本(照 SolidCraft 的 native_cad seam)。
        "fcstd" | "3dm" => {
            let (step, warns) =
                solidcraft_io::native_cad_to_step(&bytes, &name).map_err(|e| format!("{ext} 转换失败: {e}"))?;
            warnings.extend(warns);
            step
        }
        // CATIA:本程序的转换支路(src/catpart.rs)解码重述为 STEP 文本,
        // 之后与上面同一条 STEP 读取链——solidcraft 不感知 CATIA。
        // 转换结果同时在源文件旁落一份同名 .stp 中间文件(尽力而为)。
        "catpart" => {
            let (step, warns) = crate::catpart::to_step(&bytes, &name).map_err(|e| format!("{ext} 转换失败: {e}"))?;
            warnings.extend(warns);
            if let Some(w) = crate::catpart::write_intermediate(&step, std::path::Path::new(path)) {
                warnings.push(w);
            }
            step
        }
        ext if MESH_EXTS.contains(&ext) => return mesh_doc(&bytes, &name, &ext, path, t),
        _ => decode(bytes),
    };
    let imp = solidcraft_kernel::step_import_shared(&text).map_err(|e| e.to_string())?;
    warnings.extend(imp.warnings.clone());
    let mut doc = Doc {
        name: std::path::Path::new(path)
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.to_string()),
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

/// 网格文件(STL/3MF/OBJ/glTF)→ 文档:照 SolidCraft 的 mesh_import_feature 读出
/// MeshData,再按其 timeline 的做法 kernel::mesh_body 建成网格体(渲染、测量、拾取
/// 与 B-rep 体同一条路;网格体没有 B-rep 面,面类构造工具自然不可用)。
fn mesh_doc(bytes: &[u8], name: &str, ext: &str, path: &str, t: Instant) -> Result<Doc, String> {
    let feature = solidcraft_io::mesh_import_feature(bytes, name).map_err(|e| e.to_string())?;
    let solidcraft_doc::FeatureKind::MeshImport { meshes, .. } = feature.kind else {
        return Err(format!("{name}:没有读到网格"));
    };
    let schema = match ext {
        "stl" => "STL",
        "obj" => "OBJ",
        "3mf" => "3MF",
        _ => "glTF",
    };
    let mut doc = Doc {
        name: std::path::Path::new(path)
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.to_string()),
        schema: schema.into(),
        roots: Vec::with_capacity(meshes.len()),
        warnings: feature.warnings,
        load_ms: t.elapsed().as_millis(),
    };
    for m in &meshes {
        let positions: Vec<Vec3> = m
            .positions
            .as_chunks::<3>()
            .0
            .iter()
            .map(|c| Vec3::new(c[0], c[1], c[2]))
            .collect();
        let triangles: Vec<[u32; 3]> = m.triangles.as_chunks::<3>().0.to_vec();
        let body = Arc::new(
            solidcraft_kernel::mesh_body(&positions, &triangles)
                .map_err(|e| format!("网格 `{}`:{e}", m.name))?,
        );
        let mesh = body.tessellate(0.1).ok().map(Arc::new);
        doc.roots.push(Node {
            name: m.name.clone(),
            visible: true,
            mesh,
            body: Some(body),
            world: IDENTITY,
            color: m
                .color
                .map(|c| Rgb((c[0] * 255.0) as u8, (c[1] * 255.0) as u8, (c[2] * 255.0) as u8))
                .unwrap_or(DEFAULT_COLOR),
            children: Vec::new(),
        });
    }
    if doc.roots.is_empty() {
        return Err(format!("{name}:文件里没有网格体"));
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
/// 背景/网格/边线颜色随主题(照 SolidCraft 的 view palette)。
pub fn build_scene(doc: &Doc) -> Scene {
    let t = crate::theme::Tokens::get();
    let rgb = |c: egui::Color32| Rgb(c.r(), c.g(), c.b());
    let edge = if crate::theme::is_dark() { Rgb(16, 16, 18) } else { Rgb(40, 44, 52) };
    let mut sc = Scene {
        background: Some((rgb(t.viewport_top), rgb(t.viewport_bottom))),
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
                color: edge,
                width: 1.0,
                on_top: false,
            });
        }
    });
    let Some((min, max)) = bb else { return sc };
    let center = Vec3::new((min.x + max.x) * 0.5, (min.y + max.y) * 0.5, (min.z + max.z) * 0.5);
    let radius = 0.5 * ((max.x - min.x).powi(2) + (max.y - min.y).powi(2) + (max.z - min.z).powi(2)).sqrt();
    sc.radius = radius.max(1.0);
    sc.grid = ground_grid(min.z, radius, center, rgb(t.grid), rgb(t.grid_major));
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
/// 轴线用 major 色,其余网格线用普通色(随主题)。
fn ground_grid(z: f64, radius: f64, center: Vec3, grid: Rgb, major: Rgb) -> Vec<SceneLine> {
    let step = 10f64.powf(radius.log10().floor()) * (if radius / 10f64.powf(radius.log10().floor()) < 2.0 { 5.0 } else { 1.0 });
    let half = (radius * 1.5 / step).ceil() * step;
    let z0 = z - radius * 0.15;
    let mut lines = Vec::new();
    let mut i = -half;
    while i <= half + 1e-9 {
        let on_axis = i.abs() < 1e-9;
        let color = if on_axis { major } else { grid };
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

#[cfg(test)]
mod tests {
    use super::*;

    /// 二进制 STL 字节:10mm 边的四面体(4 个三角形)。
    fn stl_tetra() -> Vec<u8> {
        let v = [
            [0.0f32, 0.0, 0.0],
            [10.0, 0.0, 0.0],
            [0.0, 10.0, 0.0],
            [0.0, 0.0, 10.0],
        ];
        let tris: [[usize; 3]; 4] = [[0, 2, 1], [0, 1, 3], [1, 2, 3], [0, 3, 2]];
        let mut out = Vec::with_capacity(84 + tris.len() * 50);
        out.extend_from_slice(&[0u8; 80]);
        out.extend_from_slice(&4u32.to_le_bytes());
        for t in tris {
            for f in [0.0f32; 3] {
                out.extend_from_slice(&f.to_le_bytes());
            }
            for vi in t {
                for c in v[vi] {
                    out.extend_from_slice(&c.to_le_bytes());
                }
            }
            out.extend_from_slice(&0u16.to_le_bytes());
        }
        out
    }

    /// OBJ 文本:同一个四面体。
    const OBJ_TETRA: &str = "o tetra\nv 0 0 0\nv 10 0 0\nv 0 10 0\nv 0 0 10\nf 1 3 2\nf 1 2 4\nf 2 3 4\nf 1 4 3\n";

    fn write_tmp(name: &str, bytes: &[u8]) -> String {
        let dir = std::env::temp_dir().join("cvc-import-tests");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(name);
        std::fs::write(&path, bytes).unwrap();
        path.to_string_lossy().into_owned()
    }

    #[test]
    fn stl_import_makes_mesh_bodies() {
        let doc = load(&write_tmp("tetra.stl", &stl_tetra())).unwrap();
        assert_eq!(doc.schema, "STL");
        assert_eq!(doc.bodies(), 1);
        let node = &doc.roots[0];
        assert!(node.body.is_some() && node.mesh.is_some());
        assert_eq!(node.mesh.as_ref().unwrap().triangles.len(), 4);
    }

    #[test]
    fn obj_import_makes_mesh_bodies() {
        let doc = load(&write_tmp("tetra.obj", OBJ_TETRA.as_bytes())).unwrap();
        assert_eq!(doc.schema, "OBJ");
        assert_eq!(doc.bodies(), 1);
        assert_eq!(doc.roots[0].name, "tetra");
        assert_eq!(doc.roots[0].mesh.as_ref().unwrap().triangles.len(), 4);
    }

    /// 真实样例全链路:样例不入库,缺哪个跳哪个。查找顺序:环境变量
    /// `CVC_FIXTURES=<样例目录>`,再仓库本地 `testdata/`(Part1/Part2.CATPart
    /// 就在那里)。
    #[test]
    fn real_fixtures_if_present() {
        let mut dirs = Vec::new();
        if let Ok(dir) = std::env::var("CVC_FIXTURES") {
            dirs.push(std::path::PathBuf::from(dir));
        }
        dirs.push(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata"));
        let cases: [(&str, &str); 9] = [
            ("cylinder.step", "STEP"),
            ("bracket.stl", "STL"),
            ("tetra.obj", "OBJ"),
            ("tetra.3mf", "3MF"),
            ("tetra.gltf", "glTF"),
            ("block.fcstd", "FreeCAD"),
            ("block.3dm", "Rhino"),
            ("Part1.CATPart", "CATIA"),
            ("Part2.CATPart", "CATIA"),
        ];
        for (name, _) in cases {
            let Some(path) = dirs.iter().map(|d| d.join(name)).find(|p| p.is_file()) else { continue };
            let doc = load(path.to_string_lossy().as_ref()).unwrap_or_else(|e| panic!("{name}: {e}"));
            assert!(doc.bodies() > 0, "{name}: 0 bodies");
            let tris = doc.triangles();
            assert!(tris > 0, "{name}: 0 triangles");
            println!("{name}: {} 体 / {tris} 三角形 / schema={}", doc.bodies(), doc.schema);
        }
    }

    /// CATPart 转换在源文件旁落同名 .stp 中间文件:首次写出带本程序标记的 STEP,
    /// 重复打开覆盖自己的产物且无落盘警告,旁有他人同名文件时不覆盖并给警告。
    /// 样例不入库,缺了跳过。
    #[test]
    fn catpart_step_artifact_lands_next_to_source() {
        let mut dirs = Vec::new();
        if let Ok(dir) = std::env::var("CVC_FIXTURES") {
            dirs.push(std::path::PathBuf::from(dir));
        }
        dirs.push(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata"));
        let Some(src) = dirs.iter().map(|d| d.join("Part1.CATPart")).find(|p| p.is_file()) else { return };
        let dir = std::env::temp_dir().join("cvc-import-tests");
        std::fs::create_dir_all(&dir).unwrap();
        let catpart = dir.join("Part1.CATPart");
        std::fs::copy(&src, &catpart).unwrap();
        // 上一轮运行会故意留下"他人"的 Part1.stp,先清掉,别污染本轮断言。
        let stp = dir.join("Part1.stp");
        let _ = std::fs::remove_file(&stp);

        let open = || load(catpart.to_string_lossy().as_ref()).unwrap();
        let doc = open();
        assert!(doc.bodies() > 0);

        let text = std::fs::read_to_string(&stp).unwrap();
        assert!(text.starts_with("ISO-10303-21;"), "落盘的不是 STEP 文本");
        assert!(text.contains("converted by CADViewCraft"), "缺本程序标记");

        // 重复打开:覆盖自己的产物,不产生落盘相关警告。
        let doc2 = open();
        assert_eq!(doc2.bodies(), doc.bodies());
        assert!(!doc2.warnings.iter().any(|w| w.contains("Part1.stp")), "重复打开不应有落盘警告: {:?}", doc2.warnings);

        // 旁有他人同名 .stp:不覆盖,警告说明,导入照常。
        std::fs::write(&stp, "ISO-10303-21;\nHEADER;\nENDSEC;\n/* foreign */\nEND-ISO-10303-21;\n").unwrap();
        let doc3 = open();
        assert_eq!(doc3.bodies(), doc.bodies());
        assert!(doc3.warnings.iter().any(|w| w.contains("未覆盖")), "{:?}", doc3.warnings);
    }
}
