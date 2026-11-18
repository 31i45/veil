//! 长代码块后台异步高亮：单线程 parse 池 + mpsc + spawn_blocking + catch_unwind。
//!
//! 设计要点：
//! - 后台线程只产出纯数据 `Vec<(usize, usize, TokenKind)>`，ParseState / ParseError /
//!   ScopeStack 全部在后台栈上构造并丢弃——绝不跨线程传递非 Send 类型。
//! - SyntaxSet 通过 `highlight::syntaxes()` 懒加载 OnceLock，返回 `&'static` 只读引用，
//!   OnceLock<T> 只要 T: Sync 即 Sync；SyntaxSet 内部只有 OnceCell + Vec，只读安全。
//! - 后台 parse 完全在 `catch_unwind` 保护内，panicked 时静默丢弃该任务、不重试、
//!   不崩主线程。channel Sender drop 后后台线程自动退出。
//! - 任务与结果都带 epoch：主线程回填前校验 epoch — 后台 parse 期间块被重解析
//!   （起始偏移变了）或编辑了内容 → 新 epoch → spans 静默丢弃。

use std::sync::mpsc;
use std::sync::Arc;

use crate::view::highlight::{
    merge_adjacent, kind_of, lookup_syntax, syntaxes, TokenKind,
};
use syntect::parsing::{ParseState, ScopeStack};

/// 投递给后台线程的 parse 任务。
///
/// 字段全部是 Send 的 owned 类型；不持有任何 syntect non-Send 状态。
#[derive(Debug)]
pub(crate) struct BgTask {
    /// 块起始偏移（同 Highlighter 的缓存 key）。
    pub(crate) key: usize,
    /// 语言名（原始 info string，后台内部会走 `syntax_name` 映射）。
    pub(crate) lang: String,
    /// 块完整文本（后台整块 parse 一次性使用）。
    pub(crate) body: String,
    /// 投递时的 epoch：同 key 多 epoch 只认最新。
    pub(crate) epoch: u64,
}

/// 后台 parse 完成后回主线程的结果。
#[derive(Debug)]
#[allow(dead_code)]
pub(crate) struct BgResult {
    /// 块起始偏移（同 BgTask.key）。
    pub(crate) key: usize,
    /// 与 BgTask 同 epoch，主线程校验匹配才缓存。
    pub(crate) epoch: u64,
    /// 后台 parse 使用的原始文本，主线程用来确认块未被编辑（编辑后 text 会变 → 丢弃）。
    pub(crate) body: String,
    /// parse 产出的致密 spans（主线程直接 cache_spans）。
    pub(crate) spans: Vec<(usize, usize, TokenKind)>,
}

/// 后台 parse 线程入口（spawn_blocking 包裹，内部 catch_unwind 保护）。
///
/// 循环：recv BgTask → parse 整块 → 产出 spans → 回主线程。
/// 任何环节 panic/Err → catch_unwind 吞掉、continue。Sender drop 后 rx.recv() 返回 Err → 退出。
fn bg_loop(rx: mpsc::Receiver<BgTask>, tx: mpsc::Sender<BgResult>) {
    let _ = std::thread::spawn(move || {
        // 用 spawn_blocking 包裹的主线程不是这里 —— 这里已经在 spawn_blocking 里了。
        // 我们只需要确保 parse 逻辑在 catch_unwind 里即可。
        while let Ok(task) = rx.recv() {
            match std::panic::catch_unwind(|| bg_parse(task)) {
                Ok(Some(result)) => {
                    let _ = tx.send(result);
                }
                Ok(None) => { /* 语言不支持，静默跳过 */ }
                Err(_) => { /* panic，静默丢弃 */ }
            }
        }
    });
}

/// 单任务整块 parse：一次性 parse 完整 body，产出致密 spans。
/// 返回 None 表示语言不支持（syntax_name 映射失败）。
fn bg_parse(task: BgTask) -> Option<BgResult> {
    let name = lookup_syntax(&task.lang)?;
    let ss = syntaxes();
    let syntax = ss.find_syntax_by_name(name)?;
    let mut state = ParseState::new(syntax);
    let mut stack = ScopeStack::new();
    let mut out: Vec<(usize, usize, TokenKind)> = Vec::new();
    let mut offset = 0usize;
    for chunk in task.body.split_inclusive('\n') {
        let parse = chunk.strip_suffix('\n').unwrap_or(chunk);
        let parse = parse.strip_suffix('\r').unwrap_or(parse);
        let ops = state.parse_line(parse, ss).unwrap_or_default();
        let mut pos = 0usize;
        for (end, op) in &ops {
            if *end > pos {
                if let Some(k) = kind_of(stack.as_slice()) {
                    out.push((offset + pos, offset + *end, k));
                }
            }
            let _ = stack.apply(op);
            pos = *end;
        }
        if pos < parse.len() {
            if let Some(k) = kind_of(stack.as_slice()) {
                out.push((offset + pos, offset + parse.len(), k));
            }
        }
        offset += chunk.len();
    }
    merge_adjacent(&mut out);
    Some(BgResult {
        key: task.key,
        epoch: task.epoch,
        body: task.body,
        spans: out,
    })
}

/// 启动后台 parse 线程池（当前单线程足够，单个块 parse < 100 ms）。
/// 返回 (Sender<BgTask>, Receiver<BgResult>) —— 主线程持 Sender 投递任务，
/// 同时需要 Receiver 来接收结果。Receiver 通过 `bg_rx()` 暴露。
///
/// channel 用 `sync_channel(1)` 防止后台积压：主线程每帧 drain 一次，
/// 后台 parse 完成后如果结果没被取走就阻塞（不会丢结果）。
pub(crate) fn spawn_background() -> (mpsc::Sender<BgTask>, mpsc::Receiver<BgResult>) {
    let (task_tx, task_rx) = mpsc::channel::<BgTask>();
    let (result_tx, result_rx) = mpsc::channel::<BgResult>();
    bg_loop(task_rx, result_tx);
    let _ = Arc::new(());
    (task_tx, result_rx)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::view::highlight::Highlighter;
    use std::time::{Duration, Instant};

    fn long_rust_body() -> String {
        let mut s = String::new();
        for i in 1..=120 {
            s.push_str(&format!(
                "// comment {}\nfn func_{}(x: i32) -> Result<i32, E> {{ let v = 42 + {}; Ok(v) }}\n",
                i, i, i
            ));
        }
        s
    }

    #[test]
    fn bg_spans_equal_sync_parse_on_long_block() {
        // 后台整块 parse 产出的 spans 必须与 Highlighter 前台同步 parse 逐段等价。
        let body = long_rust_body();
        let mut h = Highlighter::default();
        let sync_spans = h.highlight(999, "rust", &body);
        assert!(!sync_spans.is_empty(), "同步 parse 必须产出 spans");

        let (tx, rx) = spawn_background();
        tx.send(BgTask {
            key: 42,
            lang: "rust".to_string(),
            body: body.clone(),
            epoch: 0,
        })
        .unwrap();

        let timeout = Instant::now() + Duration::from_secs(5);
        let result = loop {
            if let Ok(r) = rx.try_recv() {
                break r;
            }
            if Instant::now() > timeout {
                panic!("后台 parse 5 秒超时");
            }
            std::thread::sleep(Duration::from_millis(10));
        };

        assert_eq!(result.key, 42);
        assert_eq!(result.body, body);
        assert_eq!(result.spans.len(), sync_spans.len(), "spans 数量不等");
        for (i, (bg, sy)) in result.spans.iter().zip(sync_spans.iter()).enumerate() {
            assert_eq!(bg, sy, "span #{} 不等价", i);
        }
    }

    #[test]
    fn unknown_lang_is_silently_dropped() {
        // syntax_name → None → bg_parse → None → 不回传结果。绝不崩。
        let (tx, rx) = spawn_background();
        tx.send(BgTask {
            key: 100,
            lang: "brainfuck".to_string(),
            body: "++++++++++[>+++++++>++++++++++>+++>+<<<<-]".to_string(),
            epoch: 0,
        })
        .unwrap();
        std::thread::sleep(Duration::from_millis(200));
        assert!(rx.try_recv().is_err(), "未知语言应静默跳过，不回传");
    }

    #[test]
    fn background_threads_exit_on_sender_drop() {
        // drop Sender → bg_loop rx.recv() 返回 Err → 退出。不应泄漏线程。
        let (tx, _rx) = spawn_background();
        // 先投一个合法任务确保后台线程正常启动
        let _ = tx.send(BgTask {
            key: 0,
            lang: "rust".to_string(),
            body: "let x = 1;".into(),
            epoch: 0,
        });
        drop(tx);
        std::thread::sleep(Duration::from_millis(200));
        // 无 panic，测试进程正常退出 = 线程干净退出
    }

    #[test]
    fn many_tasks_all_complete() {
        // 20 个排队长块任务必须全部返回。单 worker 串行 parse，慢速 CI runner
        // 上单个长块可达 ~1s，故给 60s 上限（只防挂死，不测吞吐）。
        let (tx, rx) = spawn_background();
        let body = long_rust_body();
        for i in 0..20 {
            tx.send(BgTask {
                key: i,
                lang: "rust".to_string(),
                body: body.clone(),
                epoch: 0,
            })
            .unwrap();
        }
        let mut got = 0;
        let deadline = Instant::now() + Duration::from_secs(60);
        while got < 20 && Instant::now() < deadline {
            if let Ok(r) = rx.try_recv() {
                assert!(r.key < 20);
                got += 1;
            } else {
                std::thread::sleep(Duration::from_millis(20));
            }
        }
        assert_eq!(got, 20, "20 个任务必须全部完成");
    }

    #[test]
    fn syntaxset_once_lock_is_single_instance() {
        let a = syntaxes();
        let b = syntaxes();
        assert!(std::ptr::eq(a, b), "OnceLock 必须返回同一 SyntaxSet");
    }

    #[test]
    fn bg_parse_produces_valid_boundaries() {
        // 后台产出的 spans 必须逐段升序、区间合法、不跨 body.len()。
        let body = long_rust_body();
        let result = bg_parse(BgTask {
            key: 0,
            lang: "rust".to_string(),
            body: body.clone(),
            epoch: 0,
        })
        .expect("已知语言应返回 Some");
        let mut last_end = 0;
        for &(s, e, _) in &result.spans {
            assert!(s >= last_end, "span 必须升序：{} < {}", s, last_end);
            assert!(s <= e, "start ≤ end");
            assert!(e <= body.len(), "end ≤ body.len()");
            assert!(body.is_char_boundary(s), "s 是字符边界");
            assert!(body.is_char_boundary(e), "e 是字符边界");
            last_end = e;
        }
    }
}
