//! veil-mermaid：[`MermaidRenderer`](veil_ui::MermaidRenderer) 的第一个真实现
//! （spec `implement-mermaid-l1-quickjs-resvg` 路线 A）。
//!
//! QuickJS-ng（rquickjs）在常驻大栈 worker 线程内执行 pin 版本的官方 Tiny
//! mermaid bundle，统一 `htmlLabels:false` 产出纯 `<text>` SVG；文本测量经
//! 宿主桥注入（cosmic-text，[`worker`]）；SVG 经 resvg 栅格化为不透明 RGBA
//! 供屏幕 blit 与 PDF 位图嵌入（[`raster`]）。
//!
//! # 线程模型
//!
//! QuickJS Runtime 非 Send 且 pie 深递归需要大栈：引擎整个生命周期钉在 64 MiB
//! 专用线程内，调用方经 channel 请求/响应同步阻塞等待（trait 签名保持同步）。
//! 看门狗：`Arc<AtomicI64>` deadline 时间戳 + interrupt handler；内存上限 64 MB。
//! worker 掉线（send 失败）时重建线程一次并重试本次请求，再失败转 Err——
//! 运行时路径零 panic（[`QuickJSRenderer`] 全部可变状态用 Mutex/原子）。
//!
//! # 缓存
//!
//! 双层（[`cache`]）：`(源码哈希, 主题指纹)` → SVG（命中跳过 JS 只重栅格化），
//! `(SVG 键, scale)` → 位图（命中零拷贝克隆，spec「滚动不重布局」）。任一层
//! 超 128 条整体 clear。
//!
//! # 资产再生
//!
//! mermaid 版本 pin **12.3.0**；官方无 tiny dist，bundle 由官方 ESM chunk 自拼：
//! 1. `cd spikes/quickjs-mermaid-spike/jsbuild && npm ci`
//!    （package.json pin mermaid ^12.3.0 + esbuild ^0.25.12）
//! 2. `node make-entry.mjs`：复制官方 ESM 入口为 `src/entry-tiny.mjs`，未选图型
//!    （KEEP 集合仅 flowchart/sequence/pie）的动态 import 替换为 `Promise.reject`；
//!    布局注册 chunk 只留 dagre（剔除 elk/swimlanes/cose-bilkent）；末尾挂
//!    `globalThis.mermaidTiny`。加图型 = 修改 KEEP 集合后重打。
//! 3. `npx esbuild src/entry-tiny.mjs --bundle --minify --format=esm --outfile=out/mermaid-tiny.esm.min.js`
//! 4. 把 `out/mermaid-tiny.esm.min.js` 与 `shim.js` 复制回本 crate `assets/`
//!    （mermaid-tiny.esm.min.js，1720018 字节 @12.3.0；shim 39273 字节）。

use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use veil_ui::{
    MermaidBitmap, MermaidBitmapRenderer, MermaidDrawing, MermaidError, MermaidRenderer,
    MermaidScene, MermaidTheme,
};

mod cache;
mod metrics;
mod raster;
mod worker;

use cache::Cache;
use worker::{Job, ReadyState, ThemeSpec};

/// ready 阻塞等待上限：worker 冷启（Runtime + bundle 编译 + initialize）实测
/// ≤300ms，留 5s 余量；超时报「启动中」，调用方稍后重试即可。
const READY_WAIT: Duration = Duration::from_secs(5);

/// 渲染器：创建即返回（worker 后台冷启），首图渲染前阻塞等待 ready（上限 5s）。
/// 全部可变状态用 Mutex/原子；QuickJS Runtime 只在 worker 线程内触碰。
///
/// 实现 [`MermaidRenderer`]（矢量结果 + 自带 SVG，导出路径）与
/// [`MermaidBitmapRenderer`]（屏幕 blit 路径），两者共享同一双层缓存管线。
pub struct QuickJSRenderer {
    tx: Mutex<Option<mpsc::SyncSender<Job>>>,
    deadline_flag: Arc<AtomicI64>,
    ready: ReadyState,
    timeout: Duration,
    cache: Mutex<Cache>,
    /// JS 布局次数（缓存命中不计）：测试/诊断观测「同键不触发第二次 worker 调用」。
    js_renders: AtomicU64,
    /// 栅格化次数（位图缓存命中不计）：测试/诊断观测「同 scale 命中位图缓存」。
    rasterizes: AtomicU64,
}

impl QuickJSRenderer {
    /// 默认构造：3s 看门狗（spec 失败路径口径）。
    pub fn new() -> Arc<Self> {
        Self::with_timeout(Duration::from_secs(3))
    }

    /// 自定义看门狗时长（每次 JS 布局的上限）。
    pub fn with_timeout(timeout: Duration) -> Arc<Self> {
        let deadline_flag = Arc::new(AtomicI64::new(0));
        let ready: ReadyState = Arc::new((Mutex::new(None), std::sync::Condvar::new()));
        let tx = worker::spawn_thread(Arc::clone(&deadline_flag), Arc::clone(&ready));
        Arc::new(Self {
            tx: Mutex::new(Some(tx)),
            deadline_flag,
            ready,
            timeout,
            cache: Mutex::new(Cache::new()),
            js_renders: AtomicU64::new(0),
            rasterizes: AtomicU64::new(0),
        })
    }

    /// (JS 布局次数, 栅格化次数)。
    pub fn stats(&self) -> (u64, u64) {
        (
            self.js_renders.load(Ordering::Relaxed),
            self.rasterizes.load(Ordering::Relaxed),
        )
    }

    /// 渲染管线共用入口：位图缓存命中 → 零拷贝；SVG 缓存命中 → 只重栅格化；
    /// 全未命中 → worker 布局 + 校验 + 栅格化。返回位图与（走 JS/栅格路径时的）
    /// 已校验 SVG。
    fn pipeline(
        &self,
        source: &str,
        theme: &MermaidTheme,
        scale: f32,
    ) -> Result<(MermaidBitmap, Option<Arc<str>>), MermaidError> {
        if veil_ui::is_empty_source(source) {
            return Err(MermaidError::new("Mermaid 源码为空"));
        }
        let scale = if scale.is_finite() && scale > 0.0 { scale } else { 1.0 };
        let spec = ThemeSpec::of(theme);
        let (svg_key, bitmap_key) = cache_keys(source, theme, scale);

        // 1) 位图缓存命中：零拷贝克隆 Arc，无重布局/栅格化（spec「滚动不重布局」）。
        if let Some(bm) = self.lock_cache().get_bitmap(bitmap_key) {
            return Ok((bm, None));
        }
        // 2) SVG：缓存命中跳过 JS；未命中走 worker 布局 + 输出校验。
        let (svg, _w, _h) = self.get_or_render_svg(source, &spec, svg_key)?;
        // 3) 栅格化（调用方线程，fontdb 全局缓存）并写入位图层。
        let bm = raster::rasterize(&svg, scale, spec.bg)?;
        self.rasterizes.fetch_add(1, Ordering::Relaxed);
        self.lock_cache().put_bitmap(bitmap_key, bm.clone());
        Ok((bm, Some(svg)))
    }

    fn get_or_render_svg(
        &self,
        source: &str,
        spec: &ThemeSpec,
        svg_key: u64,
    ) -> Result<(Arc<str>, u32, u32), MermaidError> {
        if let Some(hit) = self.lock_cache().get_svg(svg_key) {
            return Ok(hit);
        }
        self.wait_ready()?;
        let timeout = self.timeout;
        let raw = self
            .request(source, spec.clone(), timeout)?
            .map_err(MermaidError::new)?;
        self.js_renders.fetch_add(1, Ordering::Relaxed);
        let (svg, w, h) = validate_svg(&raw)?;
        let svg: Arc<str> = Arc::from(svg);
        self.lock_cache().put_svg(svg_key, (Arc::clone(&svg), w, h));
        Ok((svg, w, h))
    }

    /// 投递请求并阻塞等回复。worker 掉线（send Err / 回复通道断开）时重建
    /// 线程一次并重试本次请求；再失败转 Err。
    fn request(
        &self,
        source: &str,
        spec: ThemeSpec,
        timeout: Duration,
    ) -> Result<Result<String, String>, MermaidError> {
        for _attempt in 0..2 {
            let (reply_tx, reply_rx) = mpsc::sync_channel(1);
            let job = Job {
                source: Arc::from(source),
                theme: spec.clone(),
                deadline: timeout,
                reply: reply_tx,
            };
            let sent = {
                let guard = self.lock_tx();
                match guard.as_ref() {
                    Some(tx) => tx.send(job).is_ok(),
                    None => false,
                }
            };
            if sent {
                // worker 内部超时会回 Err；此处余量 1s 只兜 worker 彻底卡死。
                // Disconnected = worker 中途掉线；Timeout = 真超时。
                return match reply_rx.recv_timeout(timeout + Duration::from_secs(1)) {
                    Ok(res) => Ok(res),
                    Err(mpsc::RecvTimeoutError::Timeout) => Err(MermaidError::new("渲染超时")),
                    Err(mpsc::RecvTimeoutError::Disconnected) => {
                        self.respawn();
                        continue;
                    }
                };
            }
            self.respawn();
        }
        Err(MermaidError::new("渲染引擎不可用"))
    }

    fn respawn(&self) {
        let mut guard = self.lock_tx();
        *guard = Some(worker::spawn_thread(
            Arc::clone(&self.deadline_flag),
            Arc::clone(&self.ready),
        ));
    }

    /// 阻塞等待 worker 就绪（上限 5s）；初始化失败/超时分别转用户可读 Err。
    fn wait_ready(&self) -> Result<(), MermaidError> {
        let (lock, cv) = &*self.ready;
        let guard = lock.lock().unwrap_or_else(|e| e.into_inner());
        let describe = |state: &Option<Result<(), String>>| match state {
            Some(Ok(())) => Ok(()),
            Some(Err(msg)) => Err(MermaidError::new(format!("渲染引擎初始化失败：{msg}"))),
            None => Err(MermaidError::new("渲染引擎启动中")),
        };
        match &*guard {
            Some(_) => describe(&guard),
            None => {
                let (guard, _timed_out) = cv
                    .wait_timeout_while(guard, READY_WAIT, |s| s.is_none())
                    .unwrap_or_else(|e| e.into_inner());
                describe(&guard)
            }
        }
    }

    fn lock_cache(&self) -> MutexGuard<'_, Cache> {
        self.cache.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn lock_tx(&self) -> MutexGuard<'_, Option<mpsc::SyncSender<Job>>> {
        self.tx.lock().unwrap_or_else(|e| e.into_inner())
    }
}

impl MermaidRenderer for QuickJSRenderer {
    fn render(&self, source: &str, theme: &MermaidTheme) -> Result<MermaidDrawing, MermaidError> {
        // 场景为 stub（位图/HTML/PDF 不消费 scene），SVG 走 1x 管线并进缓存。
        let (bm, svg) = self.pipeline(source, theme, 1.0)?;
        let scene = MermaidScene::new(bm.width, bm.height);
        let svg = match svg {
            Some(svg) => Some(svg),
            // 位图缓存命中 ⇒ SVG 缓存条目存在（两层同写同清）；极端淘汰竞争下
            // 退化为纯场景（HTML 导出由 svg_of 从场景派生）。
            None => self
                .lock_cache()
                .get_svg(hash_two(hash_one(source), ThemeSpec::of(theme).fingerprint()))
                .map(|(s, _, _)| s),
        };
        Ok(match svg {
            Some(svg) => MermaidDrawing::with_svg(scene, svg.as_ref()),
            None => MermaidDrawing::scene(scene),
        })
    }
}

impl MermaidBitmapRenderer for QuickJSRenderer {
    fn render_bitmap(
        &self,
        source: &str,
        theme: &MermaidTheme,
        scale: f32,
    ) -> Result<MermaidBitmap, MermaidError> {
        self.pipeline(source, theme, scale).map(|(bm, _)| bm)
    }
}

// ---------------------------------------------------------------------------
// 输出校验（spec：无 foreignObject、含 <text>、尺寸可解析；失败转 Err）
// ---------------------------------------------------------------------------

/// 校验渲染输出并解析尺寸：`<svg` 开头、无 foreignObject（大小写不敏感）、
/// 含 `<text>`、viewBox 或 width/height 可解析出正尺寸。
fn validate_svg(svg: &str) -> Result<(String, u32, u32), MermaidError> {
    let invalid = || MermaidError::new("Mermaid 渲染输出无效");
    let trimmed = svg.trim_start();
    let lower = trimmed.to_ascii_lowercase();
    if !lower.starts_with("<svg") {
        return Err(invalid());
    }
    if lower.contains("foreignobject") {
        return Err(invalid());
    }
    if !lower.contains("<text") {
        return Err(invalid());
    }
    let (w, h) = parse_dims(&lower).ok_or_else(invalid)?;
    Ok((trimmed.to_string(), w, h))
}

/// viewBox 优先（mermaid useMaxWidth:true 时 width 常为百分比），其次
/// width/height 属性（剥单位取数值前缀）。
fn parse_dims(lower: &str) -> Option<(u32, u32)> {
    if let Some(vb) = attr_value(lower, "viewbox") {
        let nums: Vec<f64> = vb
            .split(|c: char| c.is_whitespace() || c == ',')
            .filter(|s| !s.is_empty())
            .filter_map(|s| s.parse::<f64>().ok())
            .collect();
        if nums.len() >= 4 && nums[2].is_finite() && nums[3].is_finite() && nums[2] > 0.0 && nums[3] > 0.0
        {
            return Some((to_px(nums[2]), to_px(nums[3])));
        }
    }
    let w = parse_leading_f64(&attr_value(lower, "width")?)?;
    let h = parse_leading_f64(&attr_value(lower, "height")?)?;
    (w.is_finite() && h.is_finite() && w > 0.0 && h > 0.0).then(|| (to_px(w), to_px(h)))
}

fn to_px(v: f64) -> u32 {
    (v.round().max(1.0) as u32).min(1 << 15)
}

/// 属性名边界安全取值（`<` 或空白后的 `name="value"` / `name='value'`）。
fn attr_value(lower: &str, name: &str) -> Option<String> {
    let key = format!("{name}=");
    let mut from = 0usize;
    loop {
        let i = lower[from..].find(&key)?;
        let at = from + i;
        let ok_boundary = at > 0
            && lower[..at]
                .chars()
                .next_back()
                .is_some_and(|c| c.is_ascii_whitespace() || c == '<');
        if ok_boundary {
            let rest = lower[at + key.len()..].trim_start();
            if let Some(q @ ('"' | '\'')) = rest.chars().next() {
                let end = rest[1..].find(q)?;
                return Some(rest[1..1 + end].to_string());
            }
        }
        from = at + key.len();
    }
}

fn parse_leading_f64(s: &str) -> Option<f64> {
    let num: String = s
        .trim_start()
        .chars()
        .take_while(|c| c.is_ascii_digit() || matches!(c, '.' | '-' | '+' | 'e' | 'E'))
        .collect();
    num.parse::<f64>().ok()
}

/// 缓存键：`(源码哈希, 主题指纹)` → SVG 键；`(SVG 键, scale×100 取整)` → 位图键。
fn cache_keys(source: &str, theme: &MermaidTheme, scale: f32) -> (u64, u64) {
    let svg_key = hash_two(hash_one(source), ThemeSpec::of(theme).fingerprint());
    let scale_key = (scale * 100.0).round().max(0.0) as u64;
    (svg_key, hash_two(svg_key, scale_key))
}

fn hash_one(s: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    s.hash(&mut h);
    h.finish()
}

fn hash_two(a: u64, b: u64) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    a.hash(&mut h);
    b.hash(&mut h);
    h.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    const FLOWCHART: &str = "flowchart TD\n    A[获取数据] --> B{是否有效?}\n    B -->|是| C[处理数据]\n    B -->|否| D[丢弃]\n    C --> E((完成))";

    fn light_theme() -> MermaidTheme {
        MermaidTheme {
            fg: [32, 33, 36],
            bg: [255, 255, 255],
            accent: [0, 120, 212],
            muted: [120, 120, 120],
            font_size: 16.0,
            line_height: 22.0,
            dark: false,
            families: &["Microsoft YaHei UI", "Segoe UI"],
        }
    }

    // ---- 校验与键（纯函数，无 worker）----

    #[test]
    fn validate_accepts_text_svg_and_parses_viewbox() {
        let (svg, w, h) = validate_svg(
            r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 120.5 60"><text>hi</text></svg>"#,
        )
        .unwrap();
        assert_eq!((w, h), (121, 60));
        assert!(svg.starts_with("<svg"));
    }

    #[test]
    fn validate_rejects_foreignobject_non_svg_missing_text_or_size() {
        // foreignObject 大小写不敏感一律拒绝。
        assert!(validate_svg(
            r#"<svg viewBox="0 0 10 10"><foreignObject><body/></foreignObject><text>x</text></svg>"#
        )
        .is_err());
        assert!(validate_svg(
            r#"<svg viewBox="0 0 10 10"><FOREIGNOBJECT/><text>x</text></svg>"#
        )
        .is_err());
        assert!(validate_svg("<div>nope</div>").is_err());
        assert!(validate_svg("not svg at all").is_err());
        assert!(validate_svg(r#"<svg viewBox="0 0 10 10"><rect/></svg>"#).is_err());
        assert!(validate_svg(r#"<svg><text>x</text></svg>"#).is_err());
    }

    #[test]
    fn parse_dims_falls_back_to_width_height_attrs() {
        // 生产入口传 lowercase 后的整段 SVG；此处同口径。viewBox 优先：
        let lower = |s: &str| s.to_ascii_lowercase();
        assert_eq!(
            parse_dims(&lower(r#"<svg width="300px" height="150pt" viewBox="0 0 80 40">"#)),
            Some((80, 40))
        );
        // viewBox 无效（零尺寸）时回落 width/height 属性（剥单位）。
        assert_eq!(
            parse_dims(&lower(r#"<svg width="300px" height="150pt" viewBox="0 0 0 0">"#)),
            Some((300, 150))
        );
        assert_eq!(
            parse_dims(&lower(r#"<svg width="100%" height="40" viewBox="0 0 80 40">"#)),
            Some((80, 40))
        );
        assert_eq!(parse_dims(&lower(r#"<svg width="10" height="-3">"#)), None);
    }

    #[test]
    fn cache_keys_differ_across_source_scale_theme_and_are_stable() {
        let t1 = light_theme();
        let mut t2 = t1;
        t2.dark = true;
        let (s1, b1) = cache_keys("A-->B", &t1, 1.0);
        assert_ne!(s1, cache_keys("A-->C", &t1, 1.0).0, "不同源码应不同 SVG 键");
        assert_ne!(b1, cache_keys("A-->B", &t1, 2.0).1, "不同 scale 应不同位图键");
        assert_ne!(b1, cache_keys("A-->B", &t2, 1.0).1, "不同主题应不同位图键");
        assert_eq!(cache_keys("A-->B", &t1, 1.0), (s1, b1), "相同输入键稳定");
        assert_eq!(b1, cache_keys("A-->B", &t1, 1.0).1);
    }

    // ---- 集成（真实 QuickJS worker，冷启 ~300ms/个）----

    #[test]
    fn same_input_hits_caches_without_relayout() {
        let r = QuickJSRenderer::new();
        let t = light_theme();
        let b1 = r.render_bitmap(FLOWCHART, &t, 1.0).expect("首次应成功");
        assert_eq!(r.stats(), (1, 1));
        // 同键重绘：位图缓存命中，无重布局/重栅格化，且 Arc 零拷贝。
        let b2 = r.render_bitmap(FLOWCHART, &t, 1.0).expect("重绘应成功");
        assert_eq!(r.stats(), (1, 1));
        assert!(Arc::ptr_eq(&b1.rgba, &b2.rgba), "命中应零拷贝克隆");
        // 仅 scale 变化：SVG 缓存命中，跳过 JS 只重栅格化；2x 宽度应翻倍。
        let b3 = r.render_bitmap(FLOWCHART, &t, 2.0).expect("2x 应成功");
        assert_eq!(r.stats(), (1, 2));
        assert_eq!(b3.width, b1.width * 2);
        r.render_bitmap(FLOWCHART, &t, 1.5).expect("1.5x 应成功");
        assert_eq!(r.stats(), (1, 3), "同源码新 scale 仍不应触发 JS");
        // 新源码：重新走 JS 布局。
        r.render_bitmap("sequenceDiagram\n    A->>B: 你好", &t, 1.0).expect("新源码应成功");
        assert_eq!(r.stats(), (2, 4));
    }

    #[test]
    fn timeout_watchdog_returns_err_without_panic() {
        // 1ms 看门狗：initialize/布局必被 interrupt 中断；进程必须存活（不 panic、
        // 不挂死），后续请求仍走完整错误路径。
        let r = QuickJSRenderer::with_timeout(Duration::from_millis(1));
        let err = r
            .render_bitmap(FLOWCHART, &light_theme(), 1.0)
            .expect_err("超时必须报错");
        assert!(err.message.contains("超时"), "应为超时错误: {err}");
        // 看门狗之后 worker 仍存活：再来一次仍完整走 Err 路径（不 panic）。
        let again = r.render_bitmap(FLOWCHART, &light_theme(), 1.0);
        assert!(again.is_err());
    }

    #[test]
    fn integration_flowchart_renders_text_svg_and_bitmap() {
        let r = QuickJSRenderer::new();
        let t = light_theme();
        let drawing = r.render(FLOWCHART, &t).expect("渲染应成功");
        let svg = drawing.svg.clone().expect("应携带自带 SVG");
        let lower = svg.to_ascii_lowercase();
        assert!(lower.contains("<text"), "节点文本应可见");
        assert!(!lower.contains("foreignobject"), "不得含 foreignObject");
        assert!(drawing.scene.width > 0 && drawing.scene.height > 0);

        let bm = r.render_bitmap(FLOWCHART, &t, 2.0).expect("位图应成功");
        assert!(bm.width > 0 && bm.height > 0);
        assert_eq!(bm.rgba.len(), bm.width as usize * bm.height as usize * 4);
        // 背景应为主题 bg（白底）而非透明：任取四角像素 alpha=255。
        for px in [0, bm.rgba.len() - 4] {
            assert_eq!(bm.rgba[px + 3], 255, "位图必须不透明");
        }
    }

    #[test]
    fn empty_source_is_error() {
        let r = QuickJSRenderer::new();
        assert!(r.render_bitmap("%% 只有注释", &light_theme(), 1.0).is_err());
    }

    // 可视化目检（人工核对 CJK 布局，checklist「度量注入生效」回归工具）：
    // cargo test -p veil-mermaid --release -- --ignored --nocapture viz_cjk
    // 输出 target/mermaid-viz/{flowchart,sequence,pie}.{svg,png}（2x，白底）。
    #[test]
    #[ignore]
    fn viz_cjk_flowchart_sequence_pie_pngs() {
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../target/mermaid-viz");
        std::fs::create_dir_all(dir).unwrap();
        let r = QuickJSRenderer::new();
        let t = light_theme();
        let cases: &[(&str, &str)] = &[
            ("flowchart", FLOWCHART),
            (
                "sequence",
                "sequenceDiagram\n    participant U as 用户\n    participant S as 服务端\n    U->>S: 提交表单数据\n    S-->>U: 返回处理结果",
            ),
            (
                "pie",
                "pie title 饼图示例\n    \"已完成\" : 42\n    \"进行中\" : 31\n    \"未开始\" : 27",
            ),
        ];
        for (name, src) in cases {
            let d = r.render(src, &t).expect("渲染失败");
            if let Some(svg) = &d.svg {
                std::fs::write(format!("{dir}/{name}.svg"), svg).expect("写 SVG 失败");
            }
            let bm = r.render_bitmap(src, &t, 2.0).expect("渲染失败");
            let mut pm = resvg::tiny_skia::Pixmap::new(bm.width, bm.height).unwrap();
            pm.data_mut().copy_from_slice(&bm.rgba);
            let path = format!("{dir}/{name}.png");
            pm.save_png(&path).expect("写 PNG 失败");
            println!("已写出 {path} ({}x{})", bm.width, bm.height);
        }
    }
}
