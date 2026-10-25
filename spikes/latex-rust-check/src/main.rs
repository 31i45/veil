//! Spike（方案 2 落地依据）：latex-rust 纯 Rust 公式渲染验证。
//!
//! 与 spikes/katex-timing 同口径：同一组 4 类代表公式（短/中/长/矩阵），
//! 同样的 cold / 100 次 text / 100 次 display 计时，便于与 ADR-002 表直接对照。
//! 另产出 4 张 PNG + 1 个 SVG 供肉眼质检。
//!
//! 用法：cargo run --release

use std::hint::black_box;
use std::time::{Duration, Instant};

use latex_rust::{latex_to_png, latex_to_svg, MathFont, PngOptions, SvgOptions};

/// 与 katex-timing 完全相同的公式生成器。
fn make_formulas(n: usize) -> Vec<String> {
    let mut v = Vec::with_capacity(n);
    for i in 0..n {
        match i % 4 {
            0 => v.push(format!("x_{{{}}} + y_{{{}}} = z_{{{}}}", i, i + 1, i + 2)),
            1 => v.push(format!(
                "\\frac{{a_{{{}}} + b_{{{}}}}}{{\\sqrt{{c_{{{}}} + 1}}}}",
                i, i, i
            )),
            2 => v.push(format!(
                "\\sum_{{k=0}}^{{{}}} k^2 = \\frac{{n(n+1)(2n+1)}}{{6}} + \\int_0^{{{}}} t_{{{}}}\\,dt",
                i % 50 + 5, i % 20 + 3, i
            )),
            _ => v.push(format!(
                "A_{{{}}} = \\begin{{pmatrix}} a_{{{}}} & b \\\\ c & d_{{{}}} \\end{{pmatrix}}, \\det A_{{{}}} = a_{{{}}}d_{{{}}} - bc",
                i, i, i, i, i, i
            )),
        }
    }
    v
}

fn report(name: &str, cold: Duration, text: Duration, display: Duration) {
    println!("--- {} ---", name);
    println!("cold (font init + first render): {:?}", cold);
    println!("100 text    renders: {:?} total, avg {:?}", text, text / 100);
    println!("100 display renders: {:?} total, avg {:?}", display, display / 100);
    let budget = Duration::from_millis(300);
    println!(
        "verdict: 100-text-sync {} 300ms budget ({:.1}%)",
        if text < budget { "WITHIN" } else { "EXCEEDS" },
        text.as_secs_f64() / budget.as_secs_f64() * 100.0
    );
}

fn main() -> Result<(), String> {
    let formulas = make_formulas(100);

    // ---- SVG 路径 ----
    let t0 = Instant::now();
    let font = MathFont::stix_two_math().map_err(|e| format!("font: {e}"))?;
    let first = latex_to_svg(black_box("E = mc^2"), &font, &SvgOptions::new())
        .map_err(|e| format!("first svg: {e}"))?;
    black_box(&first);
    let cold_svg = t0.elapsed();

    let opt_text = SvgOptions::new();
    let mut opt_disp = SvgOptions::new();
    opt_disp.display = true;

    let t1 = Instant::now();
    let mut svg_bytes = 0usize;
    let mut svg_bytes_total = 0usize;
    for (i, f) in formulas.iter().enumerate() {
        let s = latex_to_svg(black_box(f), &font, &opt_text)
            .map_err(|e| format!("svg text failed at #{}: {} [{}]", i, e, f))?;
        svg_bytes = svg_bytes.max(s.len());
        svg_bytes_total += s.len();
    }
    let svg_text = t1.elapsed();

    let t2 = Instant::now();
    for (i, f) in formulas.iter().enumerate() {
        let s = latex_to_svg(black_box(f), &font, &opt_disp)
            .map_err(|e| format!("svg display failed at #{}: {} [{}]", i, e, f))?;
        svg_bytes = svg_bytes.max(s.len());
        svg_bytes_total += s.len();
    }
    let svg_disp = t2.elapsed();
    report("SVG", cold_svg, svg_text, svg_disp);
    println!("svg max bytes: {}, total bytes: {}", svg_bytes, svg_bytes_total);

    // ---- PNG 路径（144dpi 默认，透明底）----
    let t3 = Instant::now();
    let first_png = latex_to_png(black_box("E = mc^2"), &font, &PngOptions::new())
        .map_err(|e| format!("first png: {e}"))?;
    black_box(&first_png);
    let cold_png = t3.elapsed();

    let popts_text = PngOptions::new();
    let mut popts_disp = PngOptions::new();
    popts_disp.display = true;

    let t4 = Instant::now();
    let mut png_bytes_total = 0usize;
    for (i, f) in formulas.iter().enumerate() {
        let p = latex_to_png(black_box(f), &font, &popts_text)
            .map_err(|e| format!("png text failed at #{}: {} [{}]", i, e, f))?;
        png_bytes_total += black_box(p.len());
    }
    let png_text = t4.elapsed();

    let t5 = Instant::now();
    for (i, f) in formulas.iter().enumerate() {
        let p = latex_to_png(black_box(f), &font, &popts_disp)
            .map_err(|e| format!("png display failed at #{}: {} [{}]", i, e, f))?;
        png_bytes_total += black_box(p.len());
    }
    let png_disp = t5.elapsed();
    report("PNG", cold_png, png_text, png_disp);
    println!("png total bytes: {}", png_bytes_total);

    // ---- 质检样本：4 类公式各一 PNG（display）+ 矩阵 SVG ----
    let samples = [
        ("sample0_short", &formulas[0]),
        ("sample1_frac", &formulas[1]),
        ("sample2_sumint", &formulas[2]),
        ("sample3_matrix", &formulas[3]),
    ];
    for (name, f) in samples {
        let p = latex_to_png(f, &font, &popts_disp).map_err(|e| format!("{name}: {e}"))?;
        std::fs::write(format!("{name}.png"), &p).map_err(|e| e.to_string())?;
    }
    let svg = latex_to_svg(&formulas[3], &font, &opt_disp).map_err(|e| e.to_string())?;
    std::fs::write("sample3_matrix.svg", &svg).map_err(|e| e.to_string())?;
    println!("samples written: sample0..3 .png + sample3_matrix.svg");

    // ---- 参考对照（ADR-002 spike，同机同口径）----
    println!();
    println!("ref (katex-timing, rquickjs 路径): cold 29ms, avg 2.1ms, 100-inline 210ms (70% budget)");
    Ok(())
}
