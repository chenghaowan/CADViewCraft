//! 内嵌 IBM Plex 界面字体 + 系统中文回退(照 SolidCraft fonts.rs 精简)。
//! Plex Sans 变量字体按 wght 400/500/600/700 实例化为四个 family;中文取系统
//! 微软雅黑等,排在 Plex 之后(西文/数字走 Plex,汉字走回退)。

use std::sync::Arc;

use egui::{FontDefinitions, FontFamily, FontId};

const PLEX_SANS: &[u8] = include_bytes!("../assets/fonts/ibm-plex-sans/IBMPlexSans-Variable.ttf");
const PLEX_MONO: &[u8] = include_bytes!("../assets/fonts/ibm-plex-mono/IBMPlexMono-Regular.ttf");

/// (字体数据名, wght 字重)。四个 family 依次:常规/中等/半粗/粗。
const WEIGHTS: [(&str, f32); 4] = [("plex-sans", 400.0), ("ui-medium", 500.0), ("ui-semibold", 600.0), ("ui-bold", 700.0)];

const CJK: &str = "cjk";

fn sans_families() -> [FontFamily; 4] {
    [FontFamily::Proportional, medium_family(), semibold_family(), bold_family()]
}

pub fn medium_family() -> FontFamily {
    FontFamily::Name("ui-medium".into())
}

pub fn semibold_family() -> FontFamily {
    FontFamily::Name("ui-semibold".into())
}

pub fn bold_family() -> FontFamily {
    FontFamily::Name("ui-bold".into())
}

/// 中等(500)字号的 FontId。
pub fn medium(size: f32) -> FontId {
    FontId::new(size, medium_family())
}

/// 半粗(600)字号的 FontId。
pub fn semibold(size: f32) -> FontId {
    FontId::new(size, semibold_family())
}

/// 粗体(700)字号的 FontId。
pub fn bold(size: f32) -> FontId {
    FontId::new(size, bold_family())
}

/// 半粗字号的 RichText(树节点选中名等需要 RichText 的地方用)。
pub fn semibold_text(size: f32, text: impl Into<String>) -> egui::RichText {
    egui::RichText::new(text).family(semibold_family()).size(size)
}

/// Plex 四字重 + Plex Mono + 系统中文回退;装进 ctx 并把 Heading 换成半粗。
pub fn install(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    let defaults = |family: &FontFamily, fonts: &FontDefinitions| {
        fonts.families.get(family).cloned().unwrap_or_default()
    };
    let sans_defaults = defaults(&FontFamily::Proportional, &fonts);
    for ((name, weight), family) in WEIGHTS.into_iter().zip(sans_families()) {
        let mut data = egui::FontData::from_static(PLEX_SANS);
        data.tweak.coords = egui::epaint::text::VariationCoords::new([(b"wght", weight)]);
        fonts.font_data.insert(name.to_owned(), Arc::new(data));
        let mut list = vec![name.to_owned(), CJK.to_owned()];
        list.extend(sans_defaults.iter().cloned());
        fonts.families.insert(family, list);
    }
    fonts.font_data.insert("plex-mono".to_owned(), Arc::new(egui::FontData::from_static(PLEX_MONO)));
    let mono_defaults = defaults(&FontFamily::Monospace, &fonts);
    let mut mono = vec!["plex-mono".to_owned(), CJK.to_owned()];
    mono.extend(mono_defaults);
    fonts.families.insert(FontFamily::Monospace, mono);

    // 中文回退:优先微软雅黑(.ttc 集合取第 0 个 face),退到黑体/等线/宋体。
    let cjk = [
        (r"C:\Windows\Fonts\msyh.ttc", 0u32),
        (r"C:\Windows\Fonts\simhei.ttf", 0),
        (r"C:\Windows\Fonts\Deng.ttf", 0),
        (r"C:\Windows\Fonts\simsun.ttc", 0),
    ]
    .into_iter()
    .find_map(|(path, index)| {
        let bytes = std::fs::read(path).ok()?;
        let mut data = egui::FontData::from_owned(bytes);
        data.index = index;
        Some(data)
    });
    match cjk {
        Some(data) => {
            fonts.font_data.insert(CJK.to_owned(), Arc::new(data));
            log::info!("CJK 回退字体已挂载");
        }
        None => {
            // 没有系统中文:从所有 family 摘掉占位,避免 egui 报缺字体。
            for list in fonts.families.values_mut() {
                list.retain(|n| n != CJK);
            }
            log::warn!("未找到系统中文字体,界面中文可能显示为方框");
        }
    }

    ctx.set_fonts(fonts);
    ctx.global_style_mut(|style| {
        if let Some(heading) = style.text_styles.get_mut(&egui::TextStyle::Heading) {
            heading.family = semibold(heading.size).family;
        }
    });
}
