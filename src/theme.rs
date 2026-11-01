//! 界面令牌与主题:颜色/字号/圆角全部走 `Tokens`(暗/亮两套),egui visuals 由
//! `apply` 注入。数值照抄 SolidCraft 的 `assets/styles/ui/palettes.rs` 与
//! `crates/ui-egui/src/theme.rs`,保持两边外观一致。

use std::cell::Cell;

use egui::Color32;

/// 密度:间距与圆角(照 SolidCraft,两主题相同)。
pub const SPACE_3: f32 = 3.0;
pub const SPACE_4: f32 = 4.0;
pub const SPACE_6: f32 = 6.0;
pub const SPACE_8: f32 = 8.0;
pub const SPACE_10: f32 = 10.0;
pub const SPACE_12: f32 = 12.0;
pub const SPACE_16: f32 = 16.0;
pub const RADIUS_SM: u8 = 4;
pub const RADIUS_MD: u8 = 6;
pub const RADIUS_LG: u8 = 8;

/// 字号(两主题相同)。
pub const FONT_PANEL: f32 = 10.5;
pub const FONT_SMALL: f32 = 11.0;
pub const FONT_BODY: f32 = 12.0;
pub const FONT_LABEL: f32 = 12.5;
pub const FONT_BUTTON: f32 = 13.0;
pub const FONT_HEADING: f32 = 14.0;
/// 导航罗盘轴端字母。
pub const FONT_COMPASS: f32 = 13.5;

/// 浮动卡片与视口/工具栏之间的间隙(照 SolidCraft SHELL_GAP)。
pub const SHELL_GAP: f32 = 7.0;

/// 一套主题的全部用色(查看器用到的子集)。
#[derive(Clone, Copy)]
pub struct Tokens {
    /// 顶部应用栏(自绘标题栏)背景。
    pub app_bar: Color32,
    /// 应用栏文字/图标墨色。
    pub app_bar_text: Color32,
    /// 应用栏按钮悬停底色(半透明白)。
    pub app_bar_hover_button: Color32,
    /// 应用栏文字按钮悬停底色。
    pub app_bar_hover: Color32,
    /// 应用栏按钮按下底色。
    pub app_bar_hover_subtle: Color32,
    /// 应用栏上的分隔线。
    pub app_bar_separator: Color32,
    /// ribbon 背景。
    pub toolbar: Color32,
    /// 面板/窗口/卡片背景。
    pub panel: Color32,
    /// 控件静默底色(按钮/输入框)。
    pub panel_header: Color32,
    /// 描边/分隔线。
    pub border: Color32,
    pub text: Color32,
    pub text_dim: Color32,
    /// 主色(选中描边/强调)。
    pub accent: Color32,
    /// 选中底色(soft)。
    pub accent_soft: Color32,
    /// soft 底上的文字色。
    pub accent_ink: Color32,
    /// 悬停底色。
    pub hover: Color32,
    /// 输入框/视图内最暗底。
    pub field: Color32,
    /// 错误红(toast 圆点等)。
    pub error: Color32,
    /// 视口背景渐变(上,下)。
    pub viewport_top: Color32,
    pub viewport_bottom: Color32,
    /// 地面网格线(普通/主轴线)。
    pub grid: Color32,
    pub grid_major: Color32,
    /// 视口内叠加件(罗盘底)的底色。
    pub overlay: Color32,
    /// 坐标三轴色(X,Y,Z)。
    pub triad: [Color32; 3],
    /// accent 上的文字(白)。
    pub on_accent: Color32,
    /// 品牌 logo 三色(亮/中/暗)。
    pub logo: [Color32; 3],
}

impl Tokens {
    pub fn light() -> Self {
        Self {
            app_bar: Color32::from_rgb(52, 56, 64),
            app_bar_text: Color32::from_rgb(230, 234, 242),
            app_bar_hover_button: Color32::from_rgba_unmultiplied(255, 255, 255, 30),
            app_bar_hover: Color32::from_rgba_unmultiplied(255, 255, 255, 28),
            app_bar_hover_subtle: Color32::from_rgba_unmultiplied(255, 255, 255, 18),
            app_bar_separator: Color32::from_rgb(136, 145, 160),
            toolbar: Color32::from_rgb(246, 247, 249),
            panel: Color32::from_rgb(250, 251, 252),
            panel_header: Color32::from_rgb(236, 238, 242),
            border: Color32::from_rgb(208, 212, 220),
            text: Color32::from_rgb(38, 42, 50),
            text_dim: Color32::from_rgb(110, 116, 128),
            accent: Color32::from_rgb(38, 120, 218),
            accent_soft: Color32::from_rgb(214, 230, 250),
            accent_ink: Color32::from_rgb(26, 80, 127),
            hover: Color32::from_rgb(226, 234, 246),
            field: Color32::WHITE,
            error: Color32::from_rgb(214, 54, 54),
            viewport_top: Color32::from_rgb(240, 243, 247),
            viewport_bottom: Color32::from_rgb(203, 210, 221),
            grid: Color32::from_rgb(196, 202, 212),
            grid_major: Color32::from_rgb(170, 178, 190),
            overlay: Color32::from_rgba_unmultiplied(255, 255, 255, 235),
            triad: [
                Color32::from_rgb(220, 60, 60),
                Color32::from_rgb(60, 170, 60),
                Color32::from_rgb(60, 100, 230),
            ],
            on_accent: Color32::WHITE,
            logo: [
                Color32::from_rgb(96, 158, 236),
                Color32::from_rgb(58, 136, 238),
                Color32::from_rgb(55, 108, 197),
            ],
        }
    }

    pub fn dark() -> Self {
        Self {
            app_bar: Color32::from_rgb(28, 31, 38),
            toolbar: Color32::from_rgb(39, 43, 52),
            panel: Color32::from_rgb(39, 43, 52),
            panel_header: Color32::from_rgb(47, 52, 62),
            border: Color32::from_rgb(63, 69, 82),
            text: Color32::from_rgb(222, 226, 232),
            text_dim: Color32::from_rgb(148, 155, 168),
            accent: Color32::from_rgb(58, 136, 238),
            accent_soft: Color32::from_rgb(42, 70, 110),
            accent_ink: Color32::from_rgb(156, 199, 245),
            hover: Color32::from_rgb(54, 61, 74),
            field: Color32::from_rgb(30, 34, 41),
            error: Color32::from_rgb(240, 100, 96),
            viewport_top: Color32::from_rgb(46, 52, 64),
            viewport_bottom: Color32::from_rgb(46, 52, 64),
            grid: Color32::from_rgb(59, 65, 76),
            grid_major: Color32::from_rgb(73, 79, 90),
            overlay: Color32::from_rgba_unmultiplied(34, 38, 46, 240),
            ..Self::light()
        }
    }

    /// 当前主题的令牌(UI 线程)。
    pub fn get() -> Self {
        if is_dark() { Self::dark() } else { Self::light() }
    }

    /// 工具栏与面板之间的次要分隔线色(border 向 toolbar 折半,照 SolidCraft)。
    pub fn secondary_line(&self) -> Color32 {
        self.toolbar.lerp_to_gamma(self.border, 0.5)
    }
}

thread_local! {
    static DARK: Cell<bool> = const { Cell::new(true) };
}

pub fn is_dark() -> bool {
    DARK.get()
}

pub fn set_dark(dark: bool) {
    DARK.set(dark);
}

/// 把当前主题注入 egui visuals(照 SolidCraft theme::apply)。
/// 主题偏好(set_theme)由调用方设置,"跟随系统"要读 system_theme。
pub fn apply(ctx: &egui::Context) {
    ctx.set_visuals(visuals(is_dark()));
    ctx.global_style_mut(|s| {
        s.spacing.item_spacing = egui::vec2(SPACE_6, SPACE_4);
        s.spacing.button_padding = egui::vec2(SPACE_6, SPACE_3);
    });
}

fn visuals(dark: bool) -> egui::Visuals {
    let t = if dark { Tokens::dark() } else { Tokens::light() };
    let mut v = if dark { egui::Visuals::dark() } else { egui::Visuals::light() };
    v.panel_fill = t.panel;
    v.window_fill = t.panel;
    v.window_stroke = egui::Stroke::new(1.0, t.border);
    v.extreme_bg_color = t.field;
    v.faint_bg_color = t.panel_header;
    v.override_text_color = Some(t.text);
    v.widgets.inactive.weak_bg_fill = t.panel_header;
    v.widgets.inactive.bg_fill = t.panel_header;
    v.widgets.active.weak_bg_fill = t.accent_soft;
    v.selection.bg_fill = t.accent_soft;
    v.selection.stroke = egui::Stroke::new(1.0, t.accent);
    v.widgets.noninteractive.bg_stroke = egui::Stroke::new(1.0, t.border.gamma_multiply(0.7));
    v.widgets.noninteractive.fg_stroke = egui::Stroke::new(1.0, t.text);
    v.widgets.inactive.fg_stroke = egui::Stroke::new(1.0, t.text);
    v.widgets.hovered.weak_bg_fill = t.hover;
    v.widgets.hovered.bg_fill = t.hover;
    // 可拖动窗口的激活标题栏用 open 态填充。
    v.widgets.open.weak_bg_fill = t.panel_header;
    v.window_corner_radius = egui::CornerRadius::same(RADIUS_MD);
    v.menu_corner_radius = egui::CornerRadius::same(RADIUS_MD);
    for widget in [&mut v.widgets.noninteractive, &mut v.widgets.inactive, &mut v.widgets.active, &mut v.widgets.open] {
        widget.corner_radius = egui::CornerRadius::same(RADIUS_SM / 2);
    }
    v.widgets.hovered.corner_radius = egui::CornerRadius::same(RADIUS_SM.saturating_sub(1));
    v
}

/// 浮动卡片框架:panel 底 + border 描边 + 大圆角(照 SolidCraft Surface::Panel 卡片)。
pub fn card_frame(t: &Tokens) -> egui::Frame {
    egui::Frame::default()
        .fill(t.panel)
        .stroke(egui::Stroke::new(1.0, t.border))
        .corner_radius(egui::CornerRadius::same(RADIUS_LG))
        .inner_margin(egui::Margin::same(SPACE_8 as i8))
}
