#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod construct;
mod pick;
mod scene;
mod viewport;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::init();
    let mut args = std::env::args().skip(1);
    match args.next().as_deref() {
        Some("--selftest") => {
            let file = args
                .next()
                .expect("usage: cadviewcraft --selftest <file.step> [--png <out.png>] [--section]");
            let mut png = None;
            let mut section = false;
            while let Some(a) = args.next() {
                match a.as_str() {
                    "--png" => png = Some(args.next().expect("--png 需要输出路径")),
                    "--section" => section = true,
                    _ => panic!("未知参数 {a}"),
                }
            }
            selftest(&file, png.as_deref(), section)?;
            return Ok(());
        }
        Some(path) if std::path::Path::new(path).is_file() => {
            run(Some(path.to_string()))?;
            return Ok(());
        }
        _ => {}
    }
    run(None)
}

fn run(open: Option<String>) -> Result<(), Box<dyn std::error::Error>> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1440.0, 900.0])
            .with_min_inner_size([800.0, 600.0])
            .with_title("CADViewCraft"),
        renderer: eframe::Renderer::Wgpu,
        ..Default::default()
    };
    eframe::run_native(
        "CADViewCraft",
        options,
        Box::new(move |cc| {
            install_cjk_font(&cc.egui_ctx);
            // 注册 file:// + SVG 图像加载器(ribbon 图标用)。
            egui_extras::install_image_loaders(&cc.egui_ctx);
            let recent: Vec<String> = cc
                .storage
                .and_then(|s| eframe::get_value(s, "recent_files"))
                .unwrap_or_default();
            let dark_theme: Option<bool> = cc
                .storage
                .and_then(|s| eframe::get_value(s, "dark_theme"))
                .unwrap_or(Some(true));
            let mouse_rotate: u8 = cc
                .storage
                .and_then(|s| eframe::get_value(s, "mouse_rotate"))
                .unwrap_or(0);
            let mouse_pan: u8 = cc
                .storage
                .and_then(|s| eframe::get_value(s, "mouse_pan"))
                .unwrap_or(2);
            let mut vp = viewport::Viewport::new();
            if let Some(rs) = &cc.wgpu_render_state {
                if let Err(e) = vp.install(rs) {
                    log::error!("GPU 管线创建失败: {e}");
                }
            } else {
                log::error!("eframe 未提供 wgpu 渲染状态");
            }
            let mut a = app::CadViewApp::with_viewport(vp, recent, dark_theme, mouse_rotate, mouse_pan);
            if let Some(path) = open {
                a.open_on_start(&path);
            }
            Ok(Box::new(a))
        }),
    )?;
    Ok(())
}

/// 安装系统中文字体(egui 默认字体无 CJK 字形,中文会显示为方框)。
/// 优先微软雅黑(.ttc 集合,FontData.index 取第 0 个 face),依次退到黑体/等线。
fn install_cjk_font(ctx: &egui::Context) {
    for (path, index) in [
        (r"C:\Windows\Fonts\msyh.ttc", 0u32),
        (r"C:\Windows\Fonts\simhei.ttf", 0),
        (r"C:\Windows\Fonts\Deng.ttf", 0),
        (r"C:\Windows\Fonts\simsun.ttc", 0),
    ] {
        let Ok(bytes) = std::fs::read(path) else { continue };
        let mut fonts = egui::FontDefinitions::default();
        let mut data = egui::FontData::from_owned(bytes);
        data.index = index;
        fonts.font_data.insert("cjk".into(), data.into());
        for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
            if let Some(list) = fonts.families.get_mut(&family) {
                list.insert(0, "cjk".into());
            }
        }
        ctx.set_fonts(fonts);
        log::info!("CJK 字体: {path}");
        return;
    }
    log::warn!("未找到系统中文字体,界面中文可能显示为方框");
}

/// 无 UI 的全链路自检:导入 → 细分 → 测量(→ 离屏渲染出 PNG,可带断面,验证 Scene→GPU 链路)。
fn selftest(path: &str, png_out: Option<&str>, section: bool) -> Result<(), Box<dyn std::error::Error>> {
    use std::time::Instant;

    let text = std::fs::read_to_string(path)?;
    let t = Instant::now();
    let imp = solidcraft_kernel::step_import_shared(&text)?;
    println!(
        "import: {:?}  schema={} bodies={} tree_roots={} warnings={}",
        t.elapsed(),
        imp.schema,
        imp.bodies.len(),
        imp.tree.len(),
        imp.warnings.len()
    );
    for b in &imp.bodies {
        println!(
            "  body {:?} color={:?} closed={} path={:?}",
            b.name, b.color, b.closed, b.path
        );
    }
    for (i, b) in imp.bodies.iter().enumerate() {
        let t = Instant::now();
        let mesh = b.body.tessellate(0.1)?;
        let faces = mesh.tri_face.iter().copied().max().map_or(0, |m| m + 1);
        println!(
            "  body[{i}] tessellate(0.1mm): {} verts, {} tris, {faces} brep-faces, {} edge-polylines ({:?})",
            mesh.positions.len(),
            mesh.triangles.len(),
            mesh.edges.len(),
            t.elapsed()
        );
        let t = Instant::now();
        let m = solidcraft_kernel::measure(&b.body)?;
        println!(
            "  body[{i}] measure: volume={:.1}mm³ area={:.1}mm² centroid={:?} ({:?})",
            m.volume,
            m.area,
            m.centroid,
            t.elapsed()
        );
    }

    let Some(png_out) = png_out else { return Ok(()) };
    let doc = scene::load(path)?;
    let mut sc = scene::build_scene(&doc);
    let mut cam = solidcraft_render::Camera::default();
    if let Some((min, max)) = doc.bbox() {
        cam.target = solidcraft_geom::Vec3::new(
            (min.x + max.x) * 0.5,
            (min.y + max.y) * 0.5,
            (min.z + max.z) * 0.5,
        );
        cam.distance = (sc.radius * 2.6).max(10.0);
    }

    // 断面:自顶向下剖一半(与视口 uniform 同一约定:n·p > d 剖掉)。
    let clip = section.then(|| {
        let n = solidcraft_geom::Vec3::new(0.0, -1.0, 0.0); // 纵剖,剖面朝向相机
        let mut lo = f64::INFINITY;
        let mut hi = f64::NEG_INFINITY;
        if let Some((min, max)) = doc.bbox() {
            for z in [min.z, max.z] {
                let t = n.dot(solidcraft_geom::Vec3::new(0.0, 0.0, z));
                lo = lo.min(t);
                hi = hi.max(t);
            }
        }
        let d = hi - 0.5 * (hi - lo);
        sc.clip = Some((n, d));
        (n, d)
    });

    let t = Instant::now();
    let capturer = solidcraft_gpu::capture::Capturer::headless().map_err(|e| e.to_string())?;
    let rgba = capturer.rgba(&sc, &cam, 960, 720).map_err(|e| e.to_string())?;
    let bytes = solidcraft_render::png(960, 720, rgba).ok_or("PNG 编码失败")?;
    std::fs::write(png_out, bytes)?;
    println!(
        "offscreen render: {}x720 → {png_out} ({:?}, radius {:.1}mm, section {section})",
        960,
        t.elapsed(),
        sc.radius
    );

    // 拾取自检:图心射线应命中可见面(断面开启时跳过被剖掉的三角形)。
    let (w, h) = (960.0, 720.0);
    let t = Instant::now();
    let hit = crate::pick::pick(&doc, &cam, w / 2.0, h / 2.0, w, h, clip);
    println!(
        "pick center: {:?} ({:?})",
        hit.as_ref()
            .map(|h| (h.path.clone(), h.face, h.point, h.dist)),
        t.elapsed()
    );

    // 体-体距离自检:同一体自测应为 0(验证 body_distance 调用方式)。
    if let Some(hit) = &hit
        && let Some(node) = doc.node_at(&hit.path)
        && let (Some(body), Some(mesh)) = (&node.body, &node.mesh)
    {
        let chord = body.size().max(1.0) * 1e-3;
        let s = solidcraft_kernel::DistanceShape::new(body, mesh, chord);
        let t = Instant::now();
        let d = solidcraft_kernel::body_distance(&s, &s, body.size() * 2.0)
            .ok()
            .flatten()
            .map(|x| x.distance);
        println!("self-distance check: {d:?} ({:?})", t.elapsed());
    }
    Ok(())
}
