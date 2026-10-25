//! Spike（ADR-002 落地依据）：windows-msvc 上 KaTeX 渲染路径计时。
//!
//! 两条候选路径：
//!   A. `katex` crate duktape 后端（quick-js 后端在 windows-msvc 不可编译，
//!      见 katex-0.4.6 src/js_engine.rs cfg 分支）
//!   B. `rquickjs` 直接集成 vendor 的 katex.min.js（ADR-002 备选路径）
//!
//! 用法：cargo run --release

use std::hint::black_box;
use std::time::{Duration, Instant};

/// 生成有代表性的公式：短（变量/上下标）、中（分式/根号）、长（求和/积分/矩阵）混合。
/// 矩阵环境刻意保留：正是它触发 duktape 缺失的 Array.prototype.fill。
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

fn report(name: &str, cold: Duration, inline: Duration, display: Duration) {
    println!("--- {} ---", name);
    println!("cold (engine init + katex.js eval + first render): {:?}", cold);
    println!("100 inline  renders: {:?} total, avg {:?}", inline, inline / 100);
    println!("100 display renders: {:?} total, avg {:?}", display, display / 100);
    let budget = Duration::from_millis(300);
    println!(
        "verdict: 100-inline-sync {} 300ms budget ({:.1}%)",
        if inline < budget { "WITHIN" } else { "EXCEEDS" },
        inline.as_secs_f64() / budget.as_secs_f64() * 100.0
    );
}

/// 路径 A：katex crate（duktape 后端）
fn bench_duktape(formulas: &[String]) -> Result<(), String> {
    use katex::{render_with_opts, Opts, OutputType};
    let opts = |display: bool| {
        Opts::builder()
            .display_mode(display)
            .output_type(OutputType::Html)
            .build()
            .unwrap()
    };

    let t0 = Instant::now();
    let _ = black_box(
        render_with_opts(black_box("E = mc^2"), opts(false)).map_err(|e| e.to_string())?,
    );
    let cold = t0.elapsed();

    let t1 = Instant::now();
    for (i, f) in formulas.iter().enumerate() {
        render_with_opts(black_box(f), opts(false))
            .map_err(|e| format!("inline render failed at formula #{}: {} [{}]", i, e, f))?;
    }
    let inline = t1.elapsed();

    let t2 = Instant::now();
    for (i, f) in formulas.iter().enumerate() {
        render_with_opts(black_box(f), opts(true))
            .map_err(|e| format!("display render failed at formula #{}: {} [{}]", i, e, f))?;
    }
    let display = t2.elapsed();

    report("A: katex crate / duktape", cold, inline, display);
    Ok(())
}

/// 路径 B：rquickjs 直接跑 vendor 的 katex.min.js
fn bench_rquickjs(formulas: &[String]) -> Result<(), String> {
    use rquickjs::{Context, Function, Runtime};

    let katex_src = String::from_utf8_lossy(include_bytes!("../katex.min.js")).into_owned();

    let t0 = Instant::now();
    let runtime = Runtime::new().map_err(|e| e.to_string())?;
    let context = Context::full(&runtime).map_err(|e| e.to_string())?;

    context.with(|ctx| -> Result<(), String> {
        ctx.eval::<rquickjs::Value, _>(katex_src.into_bytes())
            .map_err(|e| format!("eval katex.js failed: {}", e))?;
        // 包一层全局渲染函数，参数最小化（公式串 + display 标志）
        ctx.eval::<(), _>(
            r#"globalThis.__veilRender = function (src, display) {
                return katex.renderToString(src, {
                    displayMode: display, output: "html", throwOnError: true
                });
            };"#
            .as_bytes(),
        )
        .map_err(|e| format!("wrapper eval failed: {}", e))?;

        let render: Function = ctx
            .globals()
            .get("__veilRender")
            .map_err(|e| e.to_string())?;

        let first: String = render
            .call((black_box("E = mc^2".to_string()), black_box(false)))
            .map_err(|e| e.to_string())?;
        let cold = t0.elapsed();
        black_box(first);

        let t1 = Instant::now();
        let mut total_bytes = 0usize;
        for (i, f) in formulas.iter().enumerate() {
            let html: String = render
                .call((black_box(f.clone()), black_box(false)))
                .map_err(|e| format!("inline render failed at #{}: {} [{}]", i, e, f))?;
            total_bytes += black_box(html.len());
        }
        let inline = t1.elapsed();

        let t2 = Instant::now();
        for (i, f) in formulas.iter().enumerate() {
            let html: String = render
                .call((black_box(f.clone()), black_box(true)))
                .map_err(|e| format!("display render failed at #{}: {} [{}]", i, e, f))?;
            total_bytes += black_box(html.len());
        }
        let display = t2.elapsed();

        report("B: rquickjs + katex.min.js", cold, inline, display);
        println!("total output bytes: {}", total_bytes);
        Ok(())
    })
}

fn main() {
    let formulas = make_formulas(100);

    match bench_duktape(&formulas) {
        Ok(()) => {}
        Err(e) => println!("--- A: katex crate / duktape ---\nFAILED: {}", e),
    }
    println!();
    match bench_rquickjs(&formulas) {
        Ok(()) => {}
        Err(e) => println!("--- B: rquickjs + katex.min.js ---\nFAILED: {}", e),
    }
}
