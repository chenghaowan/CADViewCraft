//! CATPart 转换支路:cadmpeg 的 CATIA V5 编解码器(清洁室实现,Apache-2.0)把
//! `.CATPart` 解码成中性的 CadIr,重述为 AP214 STEP 文本后交给 scene.rs 的 STEP
//! 读取链。本模块是全程序唯一感知 CATIA 的位置——solidcraft 只见到 STP
//! (照 SolidCraft io::native_cad 的 convert 流程裁剪,去掉格式探测:扩展名
//! 分派在 scene.rs,进到这里的只会是 CATPart)。

use std::io::Cursor;

use cadmpeg_codec_catia::CatiaCodec;
use cadmpeg_core::decode::{DecodeArena, DecodeContext, DecodeDepth, DecodeMode, DecodePolicy, ResourceLimits};
use cadmpeg_ir::codec::write::{EncodeInput, target::TargetRequest};
use cadmpeg_ir::codec::{Codec, DecodeFailure, DecodeOptions};
use cadmpeg_ir::validate::validate_neutral_for_decode;
use cadmpeg_registry::{Format, build_encoder};

/// 敌意输入的解码上限:cadmpeg 收紧的 `service` 档,输入尺寸沿用 solidcraft 的
/// 512 MB 上限。service 档已放行 800 万实体的详细零件(cadmpeg 0.6.3 起取消了
/// 收集项预算和工作单元预算,#1783)。
fn policy() -> DecodePolicy {
    DecodePolicy {
        mode: DecodeMode::Salvage,
        limits: ResourceLimits { max_input_bytes: solidcraft_io::MAX_STEP_BYTES as u64, ..ResourceLimits::service() },
    }
}

/// CATPart 字节(`file` 是文件名,用于报错)→ AP214 STEP 文本 + 警告。
/// 解码到几何层(体与产品,不含导入链不留存的设计历史),校验中性模型,
/// 再重述为 STEP 交给 STEP 读取链。
pub fn to_step(bytes: &[u8], file: &str) -> Result<(String, Vec<String>), String> {
    let name = std::path::Path::new(file)
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| file.to_string());
    let policy = policy();
    let options = DecodeOptions { depth: DecodeDepth::Geometry, policy };
    let decoded = CatiaCodec
        .decode(&mut Cursor::new(bytes), &options)
        .map_err(|e| match e {
            DecodeFailure::Codec(e) => format!("{name}:读不了这个 CATPart 文件: {e}"),
            other => format!("{name}: {other}"),
        })?;
    let (ir, report, fidelity) = decoded.into_parts();
    if !report.geometry_transferred() {
        return Err(format!("{name}:这个 CATPart 文件里没有可读的几何"));
    }
    let mut warnings: Vec<String> = report.losses.iter().map(|l| format!("{}: {}", l.code, l.message)).collect();

    // 校验中性模型(cadmpeg 自己的转换也这么做),发现的问题并进警告。
    let arena = DecodeArena::new();
    let ctx = DecodeContext::for_loaded_input(&arena, &policy, bytes.len() as u64).map_err(|e| e.to_string())?;
    let validation = validate_neutral_for_decode(&ctx, &ir, Vec::new()).map_err(|e| e.to_string())?;
    ctx.finish_session().map_err(|e| e.to_string())?;
    warnings.extend(validation.findings.iter().map(|f| format!("{}: {}", f.severity, f.message)));
    if !validation.is_ok() {
        return Err(format!("{name}:CATIA 模型无效({} 个错误)", validation.error_count()));
    }

    // 重述为 STEP AP214。没有能导出的实体时明确拒绝,其余损失并进警告。
    let plan = build_encoder(Format::Step)
        .plan(EncodeInput::new(&ir, Some(&fidelity)), TargetRequest::Explicit("ap214"))
        .map_err(|e| format!("{name}:无法重述为 STEP:{e}"))?;
    let losses = &plan.report().losses;
    if losses.iter().any(|l| l.code.to_string() == "step/export.no-exportable-solids") {
        return Err(format!("{name}:这个 CATPart 文件里没有能导入的实体"));
    }
    warnings.extend(losses.iter().map(|l| format!("{}: {}", l.code, l.message)));
    let mut out = Vec::new();
    plan.write_to(&mut out).map_err(|e| e.to_string())?;
    let step = String::from_utf8(out).map_err(|e| e.to_string())?;
    Ok((name_unnamed_products(step, &name), warnings))
}

/// cadmpeg 的 STEP 写手给无名 product 起名 `cadmpeg_model`;换成文件名
/// (纯 STEP 文本:字母、数字、空格、`-`、`_`、`.` 之外一律去掉)。
fn name_unnamed_products(step: String, file: &str) -> String {
    const PLACEHOLDER: &str = "'cadmpeg_model'";
    let stem: String = std::path::Path::new(file)
        .file_stem()
        .map(|s| s.to_string_lossy().chars().filter(|c| c.is_ascii_alphanumeric() || matches!(c, ' ' | '-' | '_' | '.')).collect())
        .unwrap_or_default();
    if stem.trim().is_empty() || !step.contains(PLACEHOLDER) { step } else { step.replace(PLACEHOLDER, &format!("'{stem}'")) }
}

/// 盘上中间文件里的标记注释:识别"这个 .stp 是本程序生成的",决定能否覆盖。
const ARTIFACT_MARKER: &str = "converted by CADViewCraft from CATPart";

/// 把转换出的 STEP 文本写到源 CATPart 旁(`<同名>.stp`),作为可检视、可复用的
/// 中间文件。尽力而为:写出失败或撞上别人的同名文件都只返回警告,不挡导入
/// (导入用的是内存里的同一份文本)。同名文件只有带本程序标记才覆盖。
pub fn write_intermediate(step: &str, source: &std::path::Path) -> Option<String> {
    let stem = source.file_stem()?;
    let target = source.with_file_name(format!("{}.stp", stem.to_string_lossy()));
    if target.is_file() {
        let ours = std::fs::read(&target)
            .map(|b| String::from_utf8_lossy(&b).contains(ARTIFACT_MARKER))
            .unwrap_or(false);
        if !ours {
            return Some(format!("旁边已有同名 {},不是本程序生成的,未覆盖", target.display()));
        }
    }
    // 标记注释只注入盘上的副本(注释是 Part 21 允许的);导入用的内存文本保持写手原样。
    let marked = match step.strip_prefix("ISO-10303-21;") {
        Some(rest) => format!("ISO-10303-21;\n/* {ARTIFACT_MARKER} */{rest}"),
        None => step.to_string(),
    };
    match std::fs::write(&target, marked) {
        Ok(()) => None,
        Err(e) => Some(format!("STEP 中间文件 `{}` 写出失败({e}),仅内存转换", target.display())),
    }
}
