use super::*;

impl LayoutEngine {
    pub fn new() -> Self {
        let StartupFonts {
            font_system,
            mono_family,
            minimal,
        } = new_font_system();
        let (bg_tx, bg_rx) = crate::view::highlight_bg::spawn_background();
        // 首帧前预热：syntect 的上下文正则在首次 parse_line 才编译（Rust 语法约
        // 40 ms）。产品层已在进程入口调用同一个幂等入口（更早启动、窗口更充裕）；
        // 这里再调一次只是兜底（直接使用 LayoutEngine 的其他宿主仍能受益）。
        crate::view::highlight::prewarm_syntax_async();
        LayoutEngine {
            font_system,
            swash: SwashCache::new(),
            scale: 1.0,
            doc_font_size: BASE_FONT_SIZE,
            doc_line_height: BASE_LINE_HEIGHT,
            doc_font: DocFontFamily::System,
            doc_face: None,
            doc_margin_px: crate::geometry::MARGIN,
            doc_readable_cols: crate::geometry::readable_cols(DocMargin::Standard),
            plain_lines: false,
            mono_family,
            fonts_minimal: minimal,
            syntax: SyntaxColors::default_light(),
            highlighter: Highlighter::default(),
            link_color: Theme::light().link,
            muted_color: Theme::light().muted,
            highlight_epoch: 0,
            pending_background_ticks: std::collections::HashMap::new(),
            bg_tx: Some(bg_tx),
            bg_rx: Some(bg_rx),
        }
    }

    /// 最小闭集启动后按需补全系统字族：只在真正需要「全部已安装字族」时发生一次
    /// （打开设置面板列字体、应用配置里的自定义字族），不进入首帧路径。
    fn ensure_full_font_set(&mut self) {
        if !self.fonts_minimal {
            return;
        }
        self.fonts_minimal = false;
        self.font_system.db_mut().load_system_fonts();
    }

    /// 该字族是否已在字体库中（最小闭集只含少数几个族）。
    fn has_family(&self, name: &str) -> bool {
        self.font_system
            .db()
            .faces()
            .any(|f| f.families.iter().any(|(fam, _)| fam == name))
    }

    /// 设置正文排版参数（字号/行高为物理像素基准，字体族与显式字族名）。
    ///
    /// 启动读配置与设置面板确认时调用；排版输入进入 `BaseKey`，随后的主题代次
    /// 递增会按新输入整体重排（详见 view::cache 的键说明）。
    /// `face` 为 `Some` 时优先于 `font`；族名驻留为 `&'static str` 供 `Attrs` 借用
    /// （仅在名字实际变化时驻留一次，人工切换频率下开销可忽略）。
    pub fn set_doc_typography(
        &mut self,
        font_size: f32,
        line_height: f32,
        font: DocFontFamily,
        face: Option<&str>,
    ) {
        // 配置里可能保存着最小闭集之外的族（用户此前从系统字体里选的）：
        // 此时补一次全量扫描，否则该族会静默退回默认族（可见的字体回退）。
        if let Some(name) = face {
            if !self.has_family(name) {
                self.ensure_full_font_set();
            }
        }
        self.doc_font_size = font_size.max(1.0);
        self.doc_line_height = line_height.max(1.0);
        self.doc_font = font;
        if self.doc_face != face {
            self.doc_face = face.map(|name| &*Box::leak(name.to_string().into_boxed_str()));
        }
    }

    /// 设置屏幕版心边距档位（逻辑像素解析值 + 是否启用可读列宽上限）。
    ///
    /// 版心参数进入 `BaseKey` 与 `App::content_width`，设置变更后随主题/会话
    /// 代次一起触发重排（缓存键失配，见 view::cache）。
    pub fn set_doc_margin(&mut self, mode: DocMargin, custom_px: f32) {
        self.doc_margin_px = crate::geometry::content_margin(mode, custom_px);
        self.doc_readable_cols = crate::geometry::readable_cols(mode);
    }

    /// 屏幕版心边距（逻辑像素，未乘 scale）。
    pub(crate) fn doc_margin_px(&self) -> f32 {
        self.doc_margin_px
    }

    /// 可读列宽上限（全角当量）：`None` = 全宽档，不封顶。
    pub(crate) fn doc_readable_cols(&self) -> Option<f32> {
        self.doc_readable_cols
    }

    /// 正文字号基准（物理像素，未乘 scale）。
    pub(crate) fn doc_font_size(&self) -> f32 {
        self.doc_font_size
    }

    /// 正文行高基准（物理像素，未乘 scale）。
    pub(crate) fn doc_line_height(&self) -> f32 {
        self.doc_line_height
    }

    /// 源码模式纯文本行视图开关：按行排版时强制等宽族、不折行、统一行高。
    pub(crate) fn set_plain_lines(&mut self, on: bool) {
        self.plain_lines = on;
    }

    /// 当前正文字体族（表格单元格排版复用）。
    pub(crate) fn doc_font(&self) -> DocFontFamily {
        self.doc_font
    }

    /// 当前显式正文字族名（表格单元格排版与 HTML 导出复用）。
    pub(crate) fn doc_face(&self) -> Option<&'static str> {
        self.doc_face
    }

    /// 已安装字族（去重排序）+ 是否含中文字形。
    ///
    /// 中文判定见 [`Self::family_has_cjk`]；结果在设置面板打开时计算一次。
    /// 最小闭集启动时这里补一次全量扫描：字体字段要列出全部已安装族，且用户
    /// 选中的族必须真的在库中（否则会静默退回默认族）。
    pub(crate) fn installed_families(&mut self) -> Vec<(String, bool)> {
        self.ensure_full_font_set();
        let mut names: Vec<String> = self
            .font_system
            .db()
            .faces()
            .flat_map(|f| f.families.iter().map(|(name, _)| name.clone()))
            .collect();
        names.sort_unstable();
        names.dedup();
        names
            .into_iter()
            // 超长名无法写入配置（见 DOC_FONT_FACE_MAX_LEN），不作为可选项。
            .filter(|name| name.len() <= crate::theme::DOC_FONT_FACE_MAX_LEN)
            .map(|name| {
                let cjk = self.family_has_cjk(&name);
                (name, cjk)
            })
            .collect()
    }

    /// 该字族是否自带中文字形。
    ///
    /// 直接读字体 cmap 表（格式 4/12），避免对每个族各排一次版——后者会触发
    /// cosmic-text 的全库回退扫描，在系统字体较多的机器上要数分钟。
    fn family_has_cjk(&self, name: &str) -> bool {
        let db = self.font_system.db();
        db.faces()
            .filter(|f| f.families.iter().any(|(fam, _)| fam == name))
            .any(|f| {
                db.with_face_data(f.id, |data, index| cmap_has_char(data, index, '中' as u32))
                    .unwrap_or(false)
            })
    }

    /// 启动时探测到的等宽族名（代码块/行内代码排版复用；N9）。
    pub(crate) fn mono_family(&self) -> Option<&'static str> {
        self.mono_family
    }

    /// 每帧排版前 drain 后台 parse 结果：把后台产出的 spans 缓存进 Highlighter，
    /// 递增 highlight_epoch 让 LayoutKey 自然失配，下次 layout_block 命中缓存即同步
    /// 产出颜色。后台 parse 期间块被编辑/删除时，Highlighter::highlight 开头会通过
    /// text 比对兜底丢弃旧 spans —— 这里无条件缓存即可。
    pub(crate) fn drain_background(&mut self) {
        let Some(rx) = &self.bg_rx else { return };
        while let Ok(result) = rx.try_recv() {
            self.highlighter
                .cache_spans(result.key, result.body, result.spans);
            self.pending_background_ticks.remove(&result.key);
            self.highlight_epoch = self.highlight_epoch.saturating_add(1);
        }
    }

    /// 更新语法高亮色板（主题切换时调用）。
    pub fn set_syntax_colors(&mut self, colors: SyntaxColors) {
        self.syntax = colors;
    }

    /// 更新链接前景色（主题切换时调用）。
    pub fn set_link_color(&mut self, color: [u8; 3]) {
        self.link_color = color;
    }

    /// 更新次要文本色（主题切换时调用）：图片题注与脚注前缀取同一 muted 语义。
    pub fn set_muted_color(&mut self, color: [u8; 3]) {
        self.muted_color = color;
    }

    /// 当前链接前景色（表格富文本排版复用）。
    pub(crate) fn link_color(&self) -> [u8; 3] {
        self.link_color
    }

    pub fn set_scale(&mut self, scale: f32) {
        self.scale = scale;
    }

    /// 当前 DPI 缩放（表格排版等复用，保持布局单位 = 物理像素）。
    pub(crate) fn scale_factor(&self) -> f32 {
        self.scale
    }

    /// 表格单元格排版需要直接驱动字体系统（跨模块 pub(crate) 访问口）。
    pub(crate) fn font_system(&mut self) -> &mut FontSystem {
        &mut self.font_system
    }

    /// 文档内 IME preedit 小串布局（不换行、默认样式、无消隐），正文字阶。
    /// 与正文同套排版规则：kinsoku 胶水 + 中西文间距经 build_layout_text 注入，
    /// OffsetMap 保证组合串内光标偏移（IME 报告的原文档坐标）正确映射。
    pub fn layout_preedit(&mut self, text: &str) -> BlockLayout {
        self.layout_single(
            text,
            self.doc_font_size * self.scale,
            self.doc_line_height * self.scale,
            doc_family(self.doc_font, self.doc_face),
            Weight::NORMAL,
        )
    }

    /// UI 单行排版（菜单/状态栏/浮层/模态及其中 IME 组合串）：UI 13px 字阶。
    pub(crate) fn layout_ui(&mut self, text: &str) -> BlockLayout {
        self.layout_single(
            text,
            typography::UI_SIZE * self.scale,
            typography::UI_LINE_HEIGHT * self.scale,
            None,
            Weight::NORMAL,
        )
    }

    /// 模态标题排版：比 UI 正文大一档并加粗，与正文共用同一 shaping/光栅路径。
    pub(crate) fn layout_ui_title(&mut self, text: &str) -> BlockLayout {
        self.layout_single(
            text,
            typography::DIALOG_TITLE_SIZE * self.scale,
            crate::tokens::surface::dialog_title_line_height(self.scale),
            None,
            Weight::SEMIBOLD,
        )
    }

    /// 单行排版：不做块级替换与消隐，只按给定字号/行高成形。
    fn layout_single(
        &mut self,
        text: &str,
        size: f32,
        line_height: f32,
        family: Option<Family<'static>>,
        weight: Weight,
    ) -> BlockLayout {
        let metrics = Metrics::new(size, line_height);
        let block = Range {
            start: 0,
            end: text.len(),
        };
        let (layout_text, map, line_bases, source_newlines, _) =
            build_layout_text(text, block, &[], usize::MAX, &[], true, false);
        let mut buffer = Buffer::new(&mut self.font_system, metrics);
        let attrs = match family {
            Some(family) => Attrs::new().family(family).weight(weight),
            None => Attrs::new().weight(weight),
        };
        buffer.set_text(&layout_text, &attrs, Shaping::Advanced, None);
        buffer.set_size(None, None);
        buffer.shape_until_scroll(&mut self.font_system, false);
        let height = buffer
            .layout_runs()
            .map(|r| r.line_top + r.line_height)
            .fold(metrics.line_height, f32::max);
        BlockLayout {
            buffer,
            map,
            line_bases,
            layout_text,
            height,
            imgs: Vec::new(),
            strikes: Vec::new(),
            decorations_under: Vec::new(),
            decorations_over: Vec::new(),
            task_boxes: Vec::new(),
            script_ranges: Vec::new(),
            node_ranges: Vec::new(),
            img_ranges: Vec::new(),
            source_newlines,
            full_len: text.len(),
            truncated: false,
            toc_source: false,
            toc_rendered: false,
        }
    }

    /// 单行文本度量（宽, 高）：仅 shape 不 rasterize，供浮层/模态按内容定宽与折行。
    pub(crate) fn measure(&mut self, text: &str) -> (f32, f32) {
        let layout = self.layout_ui(text);
        let w = layout
            .buffer
            .layout_runs()
            .map(|r| r.line_w)
            .fold(0.0, f32::max);
        (w, layout.height)
    }

    /// 模态标题度量：与 `layout_ui_title` 同源，保证卡片宽度与上屏宽度一致。
    pub(crate) fn measure_title(&mut self, text: &str) -> (f32, f32) {
        let layout = self.layout_ui_title(text);
        let w = layout
            .buffer
            .layout_runs()
            .map(|r| r.line_w)
            .fold(0.0, f32::max);
        (w, layout.height)
    }

    /// 排版单块。`text` 为块文本（含内部换行；尾换行可有可无）；`inlines`/`block` 为文档区间；
    /// `caret` 为文档绝对偏移，驱动 Reveal 激活判定（无光光标语义传 `usize::MAX` 全消隐）。
    /// `imgs`：已渲染的非激活行内公式/图片（切片 8/13）——节点整段替换为 NBSP 占位
    /// （宽≈位图宽），排版后定位占位首字形得 PlacedImg（基线对齐）。激活节点（caret
    /// 闭区间内）的 img 忽略。
    /// `reveal_all`：源码模式——块级标记与行内标记全部显原文、不做图片占位。
    /// 已知 v0 简化：高超一行的行内公式/图片可能与同块下一视觉行重叠（块高已扩展
    /// 不溢出版心）；图片顶部钳在行顶（不上侵前一块），公式保留原基线；
    /// 标题内公式/图片不随标题字号放大（尺寸由渲染侧/图片自然尺寸决定）。
    #[allow(clippy::too_many_arguments)] // 内部布局函数参数天然多
    pub fn layout_block(
        &mut self,
        kind: BlockKind,
        text: &str,
        block: Range,
        inlines: &[InlineNode],
        width: f32,
        caret: usize,
        imgs: &[InlineImg],
        reveal_all: bool,
    ) -> BlockLayout {
        // 无编号表的调用方（打印等）降级：定义标记仍消隐，引用保留 `[^id]` 原文。
        self.layout_block_with_footnotes(
            kind,
            text,
            block,
            inlines,
            width,
            caret,
            imgs,
            reveal_all,
            None,
            &[],
            None,
            false,
        )
    }

    /// 带脚注编号表的排版入口：编号在全文范围内派生，由调用方每轮构建一次并传引用。
    /// `ordinal` 为有序列表项在文档块序列中的显示编号（见 [`ordered_list_ordinals`]）；
    /// `None` 表示调用方拿不到文档顺序，此时有序列表保留源码编号。
    #[allow(clippy::too_many_arguments)] // 内部布局函数参数天然多
    pub(crate) fn layout_block_with_footnotes(
        &mut self,
        kind: BlockKind,
        text: &str,
        block: Range,
        inlines: &[InlineNode],
        width: f32,
        caret: usize,
        imgs: &[InlineImg],
        reveal_all: bool,
        footnotes: Option<&FootnoteCtx>,
        folds: &[InlineFold],
        ordinal: Option<u32>,
        toc_list: bool,
    ) -> BlockLayout {
        self.layout_block_full(
            kind,
            text,
            block,
            inlines,
            width,
            caret,
            imgs,
            reveal_all,
            footnotes,
            folds,
            ordinal,
            toc_list,
            &[],
            0,
        )
    }

    /// 带容器归属的排版入口：`quote_depth > 0` 的提升块（容器内 Details 头块及其
    /// 子块等）按 BlockQuote 同一规则消隐行首 `>` 标记（等长空格，保留缩进）。
    #[allow(clippy::too_many_arguments)] // 内部布局函数参数天然多
    pub(crate) fn layout_block_with_captions_in_container(
        &mut self,
        kind: BlockKind,
        text: &str,
        block: Range,
        inlines: &[InlineNode],
        width: f32,
        caret: usize,
        imgs: &[InlineImg],
        reveal_all: bool,
        footnotes: Option<&FootnoteCtx>,
        folds: &[InlineFold],
        ordinal: Option<u32>,
        toc_list: bool,
        captions: &[ImageCaption],
        quote_depth: u8,
    ) -> BlockLayout {
        self.layout_block_full(
            kind,
            text,
            block,
            inlines,
            width,
            caret,
            imgs,
            reveal_all,
            footnotes,
            folds,
            ordinal,
            toc_list,
            captions,
            quote_depth,
        )
    }

    #[allow(clippy::too_many_arguments)] // 内部布局函数参数天然多
    fn layout_block_full(
        &mut self,
        kind: BlockKind,
        text: &str,
        block: Range,
        inlines: &[InlineNode],
        width: f32,
        caret: usize,
        imgs: &[InlineImg],
        reveal_all: bool,
        footnotes: Option<&FootnoteCtx>,
        folds: &[InlineFold],
        ordinal: Option<u32>,
        toc_list: bool,
        captions: &[ImageCaption],
        quote_depth: u8,
    ) -> BlockLayout {
        let plain = self.plain_lines;
        let full = text.strip_suffix('\n').unwrap_or(text);
        // 纯文本行视图：行块区间可能含行尾 `\r\n`，`\r` 不参与成形。
        let full = if plain {
            full.strip_suffix('\r').unwrap_or(full)
        } else {
            full
        };
        // 单块上限（v0）：只排前缀，超出部分不参与成形/光栅，避免内存放大
        let (body, mut truncated) = cap_body(full);
        // 标题/代码块字号按既有比例从正文字号派生。
        let scale = kind_scale(&kind) * block_scale(kind.clone()) * self.scale;
        let metrics = Metrics::new(self.doc_font_size * scale, self.doc_line_height * scale);
        // 纯文本行视图：统一等宽、常规字形（列对齐稳定），其余路径沿用既有属性。
        let (link_color, doc_font, doc_face, mono_family) = (
            self.link_color,
            self.doc_font,
            self.doc_face,
            self.mono_family,
        );
        let make_attrs = move |kind: BlockKind, st: Style, pt: f32| {
            if plain {
                attrs_of_plain(st, link_color, mono_family, pt)
            } else {
                attrs_of(kind, st, link_color, doc_font, doc_face, mono_family, pt)
            }
        };
        let default_attrs = make_attrs(kind.clone(), Style::default(), metrics.font_size);
        // NBSP 占位段：测量字体必须与实际占位 span 一致，尤其 Mermaid 使用等宽族。
        let nbsp_w = if imgs.is_empty() {
            1.0
        } else {
            self.measure_nbsp_with_attrs(metrics, &default_attrs)
        };
        let mut kept: Vec<&InlineImg> = Vec::new();
        // 缺失/远程 `<img>` 的占位文本区间（块相对 doc 坐标），供 paint 画占位框。
        let mut img_placeholders: Vec<(usize, usize)> = Vec::new();
        // 脚注引用 `[^id]` → `[n]` 的替换区间（块相对 doc 坐标），供 style_spans 之后染链接色。
        let mut footnote_links: Vec<(usize, usize)> = Vec::new();
        // 脚注定义块的编号前缀 / 块末 ↩ 装饰（不来自源码区间，需单独补样式）。
        let is_footnote_def = matches!(kind, BlockKind::FootnoteDef);
        let decor = if !reveal_all && is_footnote_def {
            footnote_def_decor(body, block, caret, footnotes)
        } else {
            BlockDecor::default()
        };
        // 块级替换（切片 12）先入表；行内公式占位段随后按起点排序归并。
        // 源码模式：不做任何块级替换，标记原样显示。
        let mut tasks: Vec<TaskMark> = Vec::new();
        let mut subs: Vec<Sub> = if reveal_all {
            Vec::new()
        } else if is_footnote_def {
            decor.subs.clone()
        } else {
            block_subs(
                kind.clone(),
                body,
                block,
                caret,
                ordinal,
                toc_list,
                &mut tasks,
                quote_depth,
            )
        };
        // 容器内嵌套块标记（REN-SWEEP-002）：引用/提示/列表项里再出现的
        // 列表、任务框、围栏代码、details 标签按块类型递归渲染。
        let mut container_code: Vec<(usize, usize)> = Vec::new();
        if !reveal_all {
            let (nested, code) = container_nested_subs(&kind, body, block, caret, &mut tasks);
            subs.extend(nested);
            container_code = code;
        }
        if !reveal_all {
            let (ref_subs, links) = footnote_ref_subs(inlines, block, caret, body, footnotes);
            subs.extend(ref_subs);
            footnote_links = links;
            for im in imgs {
                if node_active(im.range, caret) {
                    continue; // 激活态显源码，不占位
                }
                let rs = im.range.start.saturating_sub(block.start);
                let re = im.range.end.saturating_sub(block.start).min(body.len());
                if rs >= re {
                    continue; // 防御：img 区间须落在本块内（shell 按块收集，不会越界）
                }
                // 向上取整：占位 advance 必须 ≥ 图宽，否则图右缘会压住后续文本
                // （CJK 前后行内公式曾因四舍五入向下而重叠）。再夹到整行可容纳的
                // 字形数：满宽图片时 ceil 可能多出一个 NBSP 宽度，导致占位折行 →
                // 每个视觉行都按图高计高，独立图片块高度翻倍（半屏空白）。
                let cap = ((width / nbsp_w).floor() as usize).max(1);
                let nbsp = ((im.width as f32 / nbsp_w).ceil() as usize).max(1).min(cap);
                subs.push(Sub::fill(rs, re, "\u{A0}".repeat(nbsp)));
                kept.push(im);
            }
            // `<br>` 硬换行：void 自闭合标签，整段（<br> 或 <br/>）替换为单个 \n。
            // 与 img 占位同源：br range 在 block 绝对坐标，转块相对后入 subs；
            // OffsetMap::from_spans 按起点排序归并，若与已有 subs 重叠（罕见）则按包含判定丢弃。
            collect_br_subs(inlines, block, &mut subs);
            // `<q>` 双引号：非激活态在内容两端插入可见引号（激活态显源码，不插）。
            collect_q_subs(inlines, block, caret, &mut subs);
            // `<img>` 无可用位图（远程 src 被解析层置空 / 本地缺失 / 加载失败）时，
            // 整段替换为 `[alt]` 文本占位，并登记占位框装饰（paint 画浅灰底）。
            // 带题注的图片改用通用占位：alt 已由下方题注给出，避免重复显示。
            collect_html_img_subs(
                inlines,
                block,
                caret,
                &kept,
                captions,
                &mut subs,
                &mut img_placeholders,
            );
            // 图片题注（仅独立成行的图片、alt 非空）：在图片所在视觉行之后插入独立一行。
            // 视图派生文本，不进文档字节；光标进入图片（显源码）时让位，避免与 alt 重复。
            for cap in captions {
                if node_active(cap.range, caret) {
                    continue;
                }
                let rs = cap.range.start.saturating_sub(block.start);
                let re = cap.range.end.saturating_sub(block.start).min(body.len());
                if rs >= re || !body.is_char_boundary(rs) || !body.is_char_boundary(re) {
                    continue;
                }
                let loaded = kept.iter().any(|im| im.range == cap.range);
                if !loaded && !img_placeholders.iter().any(|&(s, e)| s == rs && e == re) {
                    // Markdown 图片缺失：整节点降级为通用占位，alt 由题注行给出（不重复）。
                    subs.push(Sub::fill(rs, re, t(Key::ImageAltFallback)));
                    img_placeholders.push((rs, re));
                }
                let line_end = body[re..].find('\n').map_or(body.len(), |i| re + i);
                let (anchor, fill) = if line_end < body.len() {
                    (line_end + 1, format!("{}\n", cap.text))
                } else {
                    (body.len(), format!("\n{}", cap.text))
                };
                subs.push(Sub::caption(anchor, fill));
            }
            // 行内 `<details>` 折叠（B3）：收起态的正文整段消隐；标签本身由
            // `collect_elide` 按行内 HTML 规则消隐，故不出现裸标签。
            for f in folds {
                if !f.expanded && f.body.0 < f.body.1 {
                    subs.push(Sub::elide(f.body.0, f.body.1));
                }
            }
        }
        let apply_cjk = !matches!(
            kind,
            BlockKind::FencedCode(_)
                | BlockKind::IndentedCode
                | BlockKind::Math
                | BlockKind::Table
                | BlockKind::Mermaid
                // Details 内是整块源码文本，标签消隐后不应在 CJK↔ASCII 边界补空格，
                // 否则 summary 首字前会多出一个游标空格。
                | BlockKind::Details { .. }
        );
        // 反斜杠硬换行标记：非活动块消隐行尾 `\`；换行语义由源 `\n` 保留（REN-SWEEP-007）。
        if !reveal_all
            && !matches!(
                kind,
                BlockKind::FencedCode(_)
                    | BlockKind::IndentedCode
                    | BlockKind::Math
                    | BlockKind::Table
                    | BlockKind::Mermaid
            )
        {
            subs.extend(hard_break_subs(
                body,
                block,
                caret,
                inlines,
                &container_code,
            ));
        }
        // 任务列表方框占位（Task 5）：源码标记整段替换为定宽 NBSP（不参与断行），
        // 宽度只留「方框 + 间距」，末尾补一个空格保证与后续文本的间隔；
        // 方框由绘制层按 `task_boxes` 自绘。占位与勾选态无关。
        if !tasks.is_empty() {
            let slot = task_box_slot(metrics.line_height);
            let nbsp_w = self.measure_nbsp_with_attrs(metrics, &default_attrs);
            let count = ((slot / nbsp_w).ceil() as usize).max(1);
            let fill = format!("{} ", "\u{A0}".repeat(count));
            for mk in &tasks {
                subs.push(Sub::fill(mk.start, mk.end, fill.clone()));
            }
        }
        let (layout_text, map, line_bases, source_newlines, caption_layout_spans) =
            build_layout_text(body, block, inlines, caret, &subs, apply_cjk, reveal_all);
        let mut node_ranges = Vec::new();
        collect_node_ranges(inlines, &mut node_ranges);
        // 指纹输入一律存块相对坐标：增量重解析保留 BlockId 而块 range 可能整体平移，
        // 绝对坐标会在命中校验时把新 caret 误判为落在旧区间内（基线 memo 性质测试反例）。
        for r in node_ranges.iter_mut() {
            *r = Range {
                start: r.start.saturating_sub(block.start),
                end: r.end.saturating_sub(block.start),
            };
        }
        // 行内图片同理：激活位决定是否用 NBSP 占位（显源码），也是排版输入。
        let mut img_ranges: Vec<Range> = imgs
            .iter()
            .map(|im| Range {
                start: im.range.start.saturating_sub(block.start),
                end: im.range.end.saturating_sub(block.start),
            })
            .collect();
        img_ranges.sort_by_key(|r| (r.start, r.end));
        let mut buffer = Buffer::new(&mut self.font_system, metrics);
        // style_spans 产出文档坐标，经映射转 layout 坐标；整体落入消隐/占位区的 span 零宽丢弃。
        // 占位 NBSP 区无样式来源，显式补默认样式 span（cosmic 排版文本 = 各 span 顺次拼接，
        // 覆盖必须完备且有序）；指针序即 layout 偏移序。
        //
        // FencedCode 块带语言标记时走语法高亮（inlines 为空，style_spans 仅产出默认段）。
        let style_segments: Vec<(usize, usize, Style)> = match &kind {
            BlockKind::FencedCode(Some(lang)) => {
                // 长代码块（行数/字节数超阈值）首次渲染跳过高亮：先上默认色首帧，
                // 后台 parse 完成后通过 LayoutKey 失效触发重排，此时缓存已命中 → 同步产出颜色。
                let is_long = body.lines().count()
                    >= crate::view::highlight::LONG_CODE_THRESHOLD_LINES
                    || body.len() >= crate::view::highlight::LONG_CODE_THRESHOLD_BYTES;
                let cache_hit = self.highlighter.has(block.start);
                if is_long && !cache_hit {
                    // 投递后台 parse 任务；epoch 仅用于本地去重（pending_background_ticks），回填不校验结果 epoch。
                    if let Some(tx) = &self.bg_tx {
                        // 去重：同一 key 在本 epoch 已投递则不重复。
                        let current_epoch = self.highlight_epoch;
                        let should_send = self
                            .pending_background_ticks
                            .get(&block.start)
                            .copied()
                            .map(|e| e < current_epoch)
                            .unwrap_or(true);
                        if should_send {
                            let _ = tx.send(crate::view::highlight_bg::BgTask {
                                key: block.start,
                                lang: lang.to_string(),
                                body: body.to_string(),
                            });
                            self.pending_background_ticks
                                .insert(block.start, current_epoch);
                        }
                    }
                    // 返回空 spans → to_styles 产出覆盖整个 body 的全默认色段。
                    to_styles(&[], body.len(), &self.syntax, Style::default())
                } else {
                    let tk = self.highlighter.highlight(block.start, lang, body);
                    to_styles(&tk, body.len(), &self.syntax, Style::default())
                }
            }
            _ => style_spans(body.len(), block, inlines),
        };
        // `[n]` 的展示文本并非源码字符，样式表里没有它的颜色来源；
        // 按替换区间切分样式段并置 link（style_spans 会把同式相邻段合并，不能整段改）。
        let style_segments = with_link_spans(style_segments, &footnote_links, body.len());
        // 容器内嵌套围栏代码：整行覆盖为等宽 + 代码底色（覆盖行内标记样式）。
        let style_segments = with_container_code_spans(style_segments, &container_code, body.len());
        let mut spans: Vec<(&str, Attrs)> = Vec::new();
        // 装饰文本（脚注定义编号前缀 / 块末 `↩`）由下方显式 span 提供；基础样式段
        // 必须让出这两段，否则 set_rich_text 会把同一段拼接两次 —— 表现即
        // 定义行在返回箭头前重复出现 `n.`（REN-SWEEP-006）。
        let decor_prefix_end = decor.prefix_len.min(layout_text.len());
        let decor_suffix_start = layout_text.len().saturating_sub(decor.suffix_len);
        for &(s, e, st) in &style_segments {
            let (ls, le) = (map.doc_to_layout(s), map.doc_to_layout(e));
            let ls = ls.max(decor_prefix_end);
            let le = le.min(decor_suffix_start);
            if ls < le {
                spans.push((
                    &layout_text[ls..le],
                    make_attrs(kind.clone(), st, metrics.font_size),
                ));
            }
        }
        // 装饰 span：前缀 `n. `（muted）与块末 `↩`（链接色）由插入段产生，
        // 必须显式补样式；否则会落进默认 attrs（前缀不高亮、后缀无链接色）。
        if decor.prefix_len > 0 {
            let ls = map.doc_to_layout(0);
            let le = (ls + decor.prefix_len).min(layout_text.len());
            if ls < le {
                let st = Style {
                    color: footnotes.map(FootnoteCtx::muted),
                    ..Default::default()
                };
                spans.push((
                    &layout_text[ls..le],
                    make_attrs(kind.clone(), st, metrics.font_size),
                ));
            }
        }
        if decor.suffix_len > 0 {
            let ls = map.doc_to_layout(body.len());
            let le = (ls + decor.suffix_len).min(layout_text.len());
            if ls < le {
                let st = Style {
                    link: true,
                    ..Default::default()
                };
                spans.push((
                    &layout_text[ls..le],
                    make_attrs(kind.clone(), st, metrics.font_size),
                ));
            }
        }
        // B4：图片高度参与行高。图高于正文行高时，把占位区间的 attrs 的 line_height
        // 抬到图高（cosmic-text 逐 span metrics），该视觉行整体变高 → 后续行/光标/
        // 命中随之下移，不会与图片重叠。占位区间可能与既有样式 span 重叠，故先按
        // 图片区间切分基础 span（图片 attrs 优先），避免重复 span 被 cosmic 丢弃。
        let mut img_specs: Vec<(usize, usize, Attrs)> = Vec::new();
        for im in &kept {
            let ls = map.doc_to_layout(im.range.start.saturating_sub(block.start));
            let ll = map.doc_to_layout(im.range.end.saturating_sub(block.start)) - ls;
            let img_metrics = if im.height as f32 > metrics.line_height {
                Metrics::new(metrics.font_size, im.height as f32)
            } else {
                metrics
            };
            img_specs.push((ls, ls + ll, default_attrs.clone().metrics(img_metrics)));
        }
        if !img_specs.is_empty() {
            let base = layout_text.as_ptr() as usize;
            let mut out: Vec<(&str, Attrs)> = Vec::new();
            // 每个图片占位区间已被基础 span 覆盖的字节段：拼接式富文本里同一段
            // 只能出现一次，否则占位文本被重复排版（图片块高度翻倍）。
            let mut covered: Vec<Vec<(usize, usize)>> = vec![Vec::new(); img_specs.len()];
            for (txt, at) in spans.drain(..) {
                let s = txt.as_ptr() as usize - base;
                let mut pieces = vec![(s, s + txt.len(), at)];
                for (idx, (is, ie, iat)) in img_specs.iter().enumerate() {
                    let mut next = Vec::new();
                    for (ps, pe, pat) in pieces {
                        if *ie <= ps || *is >= pe {
                            next.push((ps, pe, pat));
                            continue;
                        }
                        if ps < *is {
                            next.push((ps, *is, pat.clone()));
                        }
                        let (os, oe) = (ps.max(*is), pe.min(*ie));
                        if os < oe {
                            next.push((os, oe, iat.clone()));
                            covered[idx].push((os, oe));
                        }
                        if pe > *ie {
                            next.push((*ie, pe, pat));
                        }
                    }
                    pieces = next;
                }
                for (ps, pe, pat) in pieces {
                    out.push((&layout_text[ps..pe], pat));
                }
            }
            // 基础 span 未覆盖的占位字节段补默认样式（仅补空缺，不重复整段）。
            for (idx, (is, ie, at)) in img_specs.iter().enumerate() {
                let mut pos = *is;
                covered[idx].sort_unstable();
                for (cs, ce) in &covered[idx] {
                    if *cs > pos {
                        out.push((&layout_text[pos..*cs], at.clone()));
                    }
                    pos = pos.max(*ce);
                }
                if pos < *ie {
                    out.push((&layout_text[pos..*ie], at.clone()));
                }
            }
            spans = out;
        }
        // 题注：次要文本色（与脚注前缀同源的 muted），字号/行高沿用本块 metrics。
        // 位于 layout 文本末尾/图片行之后，必须显式补样式；换行本身无字形，可留空缺。
        for &(cs, ce) in &caption_layout_spans {
            if cs >= ce || ce > layout_text.len() {
                continue;
            }
            let st = Style {
                color: Some(self.muted_color),
                ..Default::default()
            };
            spans.push((
                &layout_text[cs..ce],
                make_attrs(kind.clone(), st, metrics.font_size),
            ));
        }
        if spans.is_empty() {
            spans.push(("", default_attrs.clone())); // 空行也需默认样式
        }
        spans.sort_by_key(|(s, _)| s.as_ptr() as usize);
        buffer.set_rich_text(spans, &default_attrs, Shaping::Advanced, None);
        if plain {
            // 纯文本行视图：不折行（超长行水平溢出由帧裁剪），行高恒等于正文行高。
            buffer.set_wrap(Wrap::None);
            buffer.set_size(None, None);
        } else {
            // WordOrGlyph：按 UAX #14 断行机会折行（CJK 逐字可断 + 行首/行尾禁则
            // 由 CL/OP 类天然生效），单个超宽词（长 URL 等）回退逐字形断行。
            buffer.set_wrap(Wrap::WordOrGlyph);
            buffer.set_size(Some(width), None);
        }
        buffer.shape_until_scroll(&mut self.font_system, false);
        let mut height = buffer
            .layout_runs()
            .map(|r| r.line_top + r.line_height)
            .fold(metrics.line_height, f32::max);
        // 面积上限与排版同源：先按「最宽视觉行 × 块高」预估光栅像素，
        // 超限就截短块高并标记降级。几何与光栅必须一致，否则绘制会在块尾留白。
        let notice_h = if truncated {
            self.doc_line_height * self.scale
        } else {
            0.0
        };
        let max_line_w = buffer
            .layout_runs()
            .flat_map(|r| r.glyphs.iter())
            .map(|g| g.x + g.w)
            .fold(width, f32::max)
            .max(1.0);
        let budget_h = MAX_RASTER_PX as f32 / max_line_w;
        if height + notice_h > budget_h {
            height = (budget_h - notice_h).max(metrics.line_height);
            truncated = true;
        }
        height += notice_h;
        // 占位段首字形定位：x = 字形左缘，图顶 = 行基线(line_y) - baseline；块高扩展到图底。
        // 图片与 Mermaid 底边贴基线，高位图会把图顶推到行顶之上：钳到行顶，避免上侵前一块
        // （同块后续视觉行的重叠属 v0 已知简化，块高已扩展故不影响后续块）。
        let mut placed: Vec<PlacedImg> = Vec::new();
        for im in &kept {
            let ls = map.doc_to_layout(im.range.start.saturating_sub(block.start));
            let line_i = line_bases.partition_point(|&b| b <= ls).saturating_sub(1);
            let index = ls - line_bases.get(line_i).copied().unwrap_or(0);
            let mut at = None;
            for run in buffer.layout_runs() {
                if run.line_i != line_i {
                    continue;
                }
                let run_end = run.glyphs.last().map(|g| g.end).unwrap_or(0);
                if index > run_end && !run.glyphs.is_empty() {
                    continue; // 折行：占位在后续视觉行（同 caret_xy 选 run 规则）
                }
                for g in run.glyphs {
                    if index <= g.start {
                        at = Some((g.x, run.line_y, run.line_top));
                        break;
                    }
                }
                if at.is_some() {
                    break;
                }
            }
            let Some((x, line_y, line_top)) = at else {
                continue;
            }; // 防御：占位段必有 NBSP 字形
               // 独占视觉行的图片按 `<img align>` 在版心内水平对齐（占位自身不可见，
               // 移动位图不会与文本重叠；行内图片的 align 已在 shell 侧置空）。
            let x = match im.align {
                Some(veil_core::ImageAlign::Center) => ((width - im.width as f32) * 0.5).max(0.0),
                Some(veil_core::ImageAlign::Right) => (width - im.width as f32).max(0.0),
                _ => x,
            };
            let mut y = line_y - im.baseline as f32; // 可为负（图高出行顶），frame.blit 裁剪
            if matches!(im.key, InlineAsset::Image(_) | InlineAsset::Mermaid(_)) {
                y = y.max(line_top);
            }
            height = height.max(y + im.height as f32);
            placed.push(PlacedImg {
                key: im.key.clone(),
                x,
                y,
                width: im.width,
                height: im.height,
            });
        }
        probe_break_violations(&buffer, false);
        // 任务方框定位：占位段首字形的 x + 该视觉行的行顶/行高（与 PlacedImg 同一查找口径）。
        // 边长只由行高推导，故 4 预置字族/任意自定义族下完全一致；勾选态不参与。
        let mut task_boxes: Vec<TaskBox> = Vec::new();
        for mk in &tasks {
            let ls = map.doc_to_layout(mk.start);
            let line_i = line_bases.partition_point(|&b| b <= ls).saturating_sub(1);
            let index = ls - line_bases.get(line_i).copied().unwrap_or(0);
            let mut at = None;
            for run in buffer.layout_runs() {
                if run.line_i != line_i {
                    continue;
                }
                for g in run.glyphs {
                    if index <= g.start {
                        at = Some((g.x, run.line_top, run.line_height));
                        break;
                    }
                }
                if at.is_some() {
                    break;
                }
            }
            let Some((x, line_top, line_height)) = at else {
                continue; // 防御：占位段必有字形
            };
            let side = task_box_side(line_height);
            task_boxes.push(TaskBox {
                x,
                y: line_top + ((line_height - side) * 0.5).round(),
                side,
                done: mk.done,
            });
        }
        // Sub/Sup 基线偏移：doc 坐标经 OffsetMap 转 layout 坐标（光栅按字形起点命中）。
        let script_ranges: Vec<(usize, usize, f32)> = style_segments
            .iter()
            .filter_map(|&(s, e, st)| {
                let shift = st.baseline_shift?;
                let (ls, le) = (map.doc_to_layout(s), map.doc_to_layout(e));
                (ls < le).then_some((ls, le, script_shift_px(shift, self.scale)))
            })
            .collect();
        BlockLayout {
            buffer,
            map,
            line_bases,
            layout_text,
            height,
            imgs: placed,
            strikes: style_segments
                .iter()
                .filter(|(_, _, st)| st.strikethrough)
                .map(|&(s, e, _)| (s, e))
                .collect(),
            // 前置装饰：行内代码底色 / mark 底色 / kbd 圆角边框（按此顺序叠，后者覆盖前者）；
            // 块相对 doc 坐标，paint 层用 selection_rects 取物理矩形。
            decorations_under: style_segments
                .iter()
                .flat_map(|&(s, e, st)| {
                    let mut out: Vec<(usize, usize, UnderKind)> = Vec::new();
                    if st.code_bg {
                        out.push((s, e, UnderKind::InlineCodeBg));
                    }
                    if st.mark_bg {
                        out.push((s, e, UnderKind::MarkBg));
                    }
                    if st.kbd_frame {
                        out.push((s, e, UnderKind::KbdFrame));
                    }
                    if let Some(bg) = st.bg {
                        out.push((s, e, UnderKind::StyleBg(bg)));
                    }
                    out
                })
                .chain(
                    img_placeholders
                        .iter()
                        .map(|&(s, e)| (s, e, UnderKind::ImagePlaceholder)),
                )
                .collect(),
            // 后置装饰：abbr 虚线下划线 / 实线下划线 / 上划线；画在文本光栅之上。
            decorations_over: style_segments
                .iter()
                .flat_map(|&(s, e, st)| {
                    let mut out: Vec<(usize, usize, OverKind)> = Vec::new();
                    if st.dotted_underline {
                        out.push((s, e, OverKind::DottedUnderline));
                    }
                    if st.underline {
                        out.push((s, e, OverKind::Underline));
                    }
                    if st.overline {
                        out.push((s, e, OverKind::Overline));
                    }
                    out
                })
                .collect(),
            // Sub/Sup 基线偏移区间（layout 坐标），供光栅化施加 y 偏移。
            script_ranges,
            task_boxes,
            node_ranges,
            img_ranges,
            source_newlines,
            full_len: full.len(),
            truncated,
            toc_source: false,
            toc_rendered: false,
        }
    }

    /// 生成目录的视图层排版：`text` 为目录展示文本（非文档字节），按正文段落排版。
    /// 不产生任何可编辑映射（doc 坐标按展示文本恒等），也不写回文档。
    pub(crate) fn layout_toc(&mut self, text: &str, width: f32) -> BlockLayout {
        let scale = block_scale(BlockKind::Paragraph) * self.scale;
        let metrics = Metrics::new(self.doc_font_size * scale, self.doc_line_height * scale);
        let mut buffer = Buffer::new(&mut self.font_system, metrics);
        let attrs = attrs_of(
            BlockKind::Paragraph,
            Style::default(),
            self.link_color,
            self.doc_font,
            self.doc_face,
            self.mono_family,
            metrics.font_size,
        );
        buffer.set_text(text, &attrs, Shaping::Advanced, None);
        buffer.set_wrap(Wrap::WordOrGlyph);
        buffer.set_size(Some(width), None);
        buffer.shape_until_scroll(&mut self.font_system, false);
        let mut height = buffer
            .layout_runs()
            .map(|r| r.line_top + r.line_height)
            .fold(metrics.line_height, f32::max);
        let max_line_w = buffer
            .layout_runs()
            .flat_map(|r| r.glyphs.iter())
            .map(|g| g.x + g.w)
            .fold(width, f32::max)
            .max(1.0);
        let budget_h = MAX_RASTER_PX as f32 / max_line_w;
        let truncated = height > budget_h;
        if truncated {
            height = budget_h.max(metrics.line_height);
        }
        let line_bases: Vec<usize> = buffer
            .layout_runs()
            .map(|r| r.glyphs.first().map_or(0, |g| g.start))
            .collect();
        let (map, _) = OffsetMap::from_spans(text.len(), &[]);
        BlockLayout {
            buffer,
            map,
            line_bases,
            layout_text: text.to_string(),
            height,
            imgs: Vec::new(),
            strikes: Vec::new(),
            decorations_under: Vec::new(),
            decorations_over: Vec::new(),
            task_boxes: Vec::new(),
            script_ranges: Vec::new(),
            node_ranges: Vec::new(),
            img_ranges: Vec::new(),
            source_newlines: Vec::new(),
            full_len: text.len(),
            truncated,
            toc_source: true,
            toc_rendered: true,
        }
    }

    /// 表格公式占位沿用默认字体口径。
    pub(crate) fn measure_nbsp(&mut self, metrics: Metrics) -> f32 {
        self.measure_nbsp_with_attrs(metrics, &Attrs::new())
    }

    /// NBSP 占位宽度：同 metrics/attrs 下排 8 个 NBSP 求均值，缺字形时退化 1px 防除零。
    pub(super) fn measure_nbsp_with_attrs(&mut self, metrics: Metrics, attrs: &Attrs) -> f32 {
        let probe = "\u{A0}\u{A0}\u{A0}\u{A0}\u{A0}\u{A0}\u{A0}\u{A0}";
        let mut b = Buffer::new(&mut self.font_system, metrics);
        b.set_text(probe, attrs, Shaping::Advanced, None);
        b.set_size(None, None);
        b.shape_until_scroll(&mut self.font_system, false);
        let w: f32 = b
            .layout_runs()
            .flat_map(|r| r.glyphs.iter())
            .map(|g| g.w)
            .sum();
        (w / 8.0).max(1.0)
    }

    /// 各视觉行行顶（块内物理像素，升序）：打印分页的行级候选断点（3b）。
    pub fn line_tops(&self, layout: &BlockLayout) -> Vec<f32> {
        layout.buffer.layout_runs().map(|r| r.line_top).collect()
    }

    /// 像素点 → 块内字节偏移（文档坐标）。y 越界钳到首/末行（hit 内建行为）。
    pub fn hit(&self, layout: &BlockLayout, x: f32, y: f32) -> usize {
        match layout.buffer.hit(x, y) {
            Some(c) => layout
                .map
                .layout_to_doc(layout.line_bases.get(c.line).copied().unwrap_or(0) + c.index),
            None => 0,
        }
    }

    /// 块内字节偏移（文档坐标）→ 光标矩形 (x, y_top, height)。
    /// 超限降级的块只渲染前缀高度：落点统一钳进可视区，避免光标画到下一块上。
    pub fn caret_xy(&self, layout: &BlockLayout, offset: usize) -> (f32, f32, f32) {
        let (x, y, h) = self.caret_xy_raw(layout, offset);
        (x, y.min((layout.height - h).max(0.0)), h)
    }

    fn caret_xy_raw(&self, layout: &BlockLayout, offset: usize) -> (f32, f32, f32) {
        let lo = layout.map.doc_to_layout(offset);
        let line_i = layout
            .line_bases
            .partition_point(|&b| b <= lo)
            .saturating_sub(1);
        let index = lo - layout.line_bases.get(line_i).copied().unwrap_or(0);
        let mut tail = (0.0, 0.0, layout.height); // 同 line_i 最后一个 run 的行尾（兜底）
        for run in layout.buffer.layout_runs() {
            if run.line_i != line_i {
                continue;
            }
            let run_end = run.glyphs.last().map(|g| g.end).unwrap_or(0);
            tail = (
                run.glyphs.last().map(|g| g.x + g.w).unwrap_or(0.0),
                run.line_top,
                run.line_height,
            );
            if index > run_end && !run.glyphs.is_empty() {
                continue; // 折行的后续视觉行才覆盖该 index
            }
            for g in run.glyphs {
                if index <= g.start {
                    return (g.x, run.line_top, run.line_height);
                }
                if index <= g.end {
                    let ratio = (index - g.start) as f32 / (g.end - g.start).max(1) as f32;
                    return (g.x + g.w * ratio, run.line_top, run.line_height);
                }
            }
            // 行尾/空行
            return tail;
        }
        tail
    }

    /// 当前偏移所在视觉行的首尾偏移（块内文档坐标，(start, end)，start <= end）。
    /// Home/End 语义：经 hit 换算，折行按视觉行切分，消隐区间不会成为端点。
    pub fn visual_line_edges(&self, layout: &BlockLayout, offset: usize) -> (usize, usize) {
        let (_, y, _) = self.caret_xy(layout, offset);
        let s = self.hit(layout, 0.0, y + 1.0);
        let e = self.hit(layout, f32::MAX, y + 1.0);
        (s.min(e), s.max(e))
    }

    /// 选区高亮矩形集（块内物理像素，(x, y_top, w, h)；0.19 起同一视觉行在
    /// BiDi/样式切换处可产多条，逐段收齐）。
    /// `start`/`end` 为块相对文档偏移，经 OffsetMap 映射后逐 run 求交；
    /// 换行符本身不产生矩形，空行/纯消隐段无字形故无矩形（v0 已知简化）。
    pub fn selection_rects(
        &self,
        layout: &BlockLayout,
        start: usize,
        end: usize,
    ) -> Vec<(f32, f32, f32, f32)> {
        let (lo0, lo1) = (
            layout.map.doc_to_layout(start),
            layout.map.doc_to_layout(end),
        );
        if lo0 >= lo1 {
            return Vec::new();
        }
        let mut rects = Vec::new();
        for run in layout.buffer.layout_runs() {
            if run.line_top >= layout.height {
                continue; // 超出降级可视区（面积上限截短）不画选区
            }
            let base = layout.line_bases.get(run.line_i).copied().unwrap_or(0);
            // 行内字节区间（半开）：Cursor 默认 Affinity::Before，比较天然排除右端点起始字形
            let (s, e) = (lo0.saturating_sub(base), lo1.saturating_sub(base));
            if s >= e {
                continue;
            }
            // cosmic-text 0.19：highlight 返回迭代器，BiDi/样式切换处单 run 可产多段，须收齐
            for (x, w) in run.highlight(Cursor::new(run.line_i, s), Cursor::new(run.line_i, e)) {
                if w > 0.0 {
                    rects.push((x, run.line_top, w, run.line_height));
                }
            }
        }
        rects
    }

    /// 光栅化为 RGBA8（straight alpha，src-over 混合到透明底）。`color` 为默认文字色。
    /// 已知简化：彩色 emoji（swash 预乘输出）按 straight 处理，边缘或有轻微杂边。
    pub fn rasterize(&mut self, layout: &BlockLayout, color: [u8; 3]) -> (Vec<u8>, u32, u32) {
        self.rasterize_buffer_scripts(&layout.buffer, layout.height, color, &layout.script_ranges)
    }

    /// 光栅化任意已排版 buffer（表格单元格复用同一 path）。`height` 为排版高（物理像素）。
    /// 悬挂标点 v0 在此生效：满行末字为闭标点时光栅右移半字宽（排版几何不变，
    /// caret/命中/选区仍按未位移坐标——光标落在标点之后恰与位移后墨缘对齐）。
    /// 带 Sub/Sup 基线偏移的光栅化：`scripts` 为 layout 字节区间 → 物理像素 dy
    /// （正数下移 sub，负数上移 sup）。字形起点落入区间即施加偏移。
    pub(crate) fn rasterize_buffer_scripts(
        &mut self,
        buffer: &Buffer,
        height: f32,
        color: [u8; 3],
        scripts: &[(usize, usize, f32)],
    ) -> (Vec<u8>, u32, u32) {
        let width_opt = buffer.size().0;
        let mut max_w: f32 = 0.0;
        for run in buffer.layout_runs() {
            let (dx, hi) = hang_shift(&run, width_opt);
            for (i, g) in run.glyphs.iter().enumerate() {
                let shift = if Some(i) == hi { dx } else { 0.0 };
                max_w = max_w.max(g.x + shift + g.w);
            }
        }
        let (w, h) = (
            max_w.ceil().max(1.0) as usize,
            height.ceil().max(1.0) as usize,
        );
        // 光栅面积安全网：单块上限之下仍可能因版心极窄/行极密而偏大，
        // 超出部分直接裁掉（像素越界本就有边界检查），绝不为病态输入分配巨缓冲。
        let h = if w * h > MAX_RASTER_PX {
            (MAX_RASTER_PX / w).max(1)
        } else {
            h
        };
        let mut px = vec![0u8; w * h * 4];
        for run in buffer.layout_runs() {
            let (dx, hi) = hang_shift(&run, width_opt);
            for (i, g) in run.glyphs.iter().enumerate() {
                let shift = if Some(i) == hi { dx } else { 0.0 };
                let base = g.color_opt.map(|c| [c.r(), c.g(), c.b()]).unwrap_or(color);
                let script_dy = scripts
                    .iter()
                    .find(|(s, e, _)| *s <= g.start && g.start < *e)
                    .map_or(0.0, |&(_, _, dy)| dy);
                let baseline = (run.line_y + script_dy).round();
                let physical = g.physical((shift, baseline), 1.0);
                let fs = &mut self.font_system;
                let key_color = Color::rgb(base[0], base[1], base[2]);
                self.swash
                    .with_pixels(fs, physical.cache_key, key_color, |gx, gy, c| {
                        let (x, y) = (physical.x + gx, physical.y + gy);
                        if x < 0 || y < 0 || x >= w as i32 || y >= h as i32 {
                            return;
                        }
                        let i = (y as usize * w + x as usize) * 4;
                        let a = c.a() as u32;
                        let ia = 255 - a;
                        px[i] = ((c.r() as u32 * a + px[i] as u32 * ia) / 255) as u8;
                        px[i + 1] = ((c.g() as u32 * a + px[i + 1] as u32 * ia) / 255) as u8;
                        px[i + 2] = ((c.b() as u32 * a + px[i + 2] as u32 * ia) / 255) as u8;
                        px[i + 3] = (a + px[i + 3] as u32 * ia / 255) as u8;
                    });
            }
        }
        (px, w as u32, h as u32)
    }
}

/// 字体文件（含 TTC 子面）的 cmap 是否把 `ch` 映射到非 0 字形。
///
/// 仅解析 `cmap` 的格式 4/12（覆盖 BMP 与增补平面），不解析字形轮廓，
/// 因此开销远小于真正排版；格式不支持时保守返回 `false`（只影响「缺中文」提示）。
fn cmap_has_char(data: &[u8], face_index: u32, ch: u32) -> bool {
    let Some(base) = sfnt_face_offset(data, face_index) else {
        return false;
    };
    let Some(num_tables) = read_u16(data, base + 4) else {
        return false;
    };
    for i in 0..num_tables as usize {
        let rec = base + 12 + i * 16;
        let Some(tag) = data.get(rec..rec + 4) else {
            return false;
        };
        if tag != b"cmap" {
            continue;
        }
        let Some(offset) = read_u32(data, rec + 8) else {
            return false;
        };
        return cmap_table_has_char(data, offset as usize, ch);
    }
    false
}

/// TTC 中第 `face_index` 个子面的 SFNT 表目录偏移。
fn sfnt_face_offset(data: &[u8], face_index: u32) -> Option<usize> {
    if data.get(0..4)? == b"ttcf" {
        let count = read_u32(data, 8)?;
        if face_index >= count {
            return None;
        }
        read_u32(data, 12 + face_index as usize * 4).map(|v| v as usize)
    } else if face_index == 0 {
        Some(0)
    } else {
        None
    }
}

fn cmap_table_has_char(data: &[u8], table: usize, ch: u32) -> bool {
    let Some(subtables) = read_u16(data, table + 2) else {
        return false;
    };
    for i in 0..subtables as usize {
        let rec = table + 4 + i * 8;
        let Some(offset) = read_u32(data, rec + 4) else {
            return false;
        };
        if cmap_subtable_has_char(data, table + offset as usize, ch) {
            return true;
        }
    }
    false
}

fn cmap_subtable_has_char(data: &[u8], sub: usize, ch: u32) -> bool {
    match read_u16(data, sub) {
        Some(4) => cmap_format4_has_char(data, sub, ch),
        Some(12) => cmap_format12_has_char(data, sub, ch),
        _ => false,
    }
}

/// cmap 格式 4（BMP；idRangeOffset 链式查表）。
fn cmap_format4_has_char(data: &[u8], sub: usize, ch: u32) -> bool {
    let c = u16::try_from(ch).ok();
    let Some(c) = c else {
        return false;
    };
    let Some(seg_x2) = read_u16(data, sub + 6) else {
        return false;
    };
    let segs = seg_x2 as usize / 2;
    if segs == 0 {
        return false;
    }
    let end_codes = sub + 14;
    let start_codes = end_codes + seg_x2 as usize + 2;
    let id_deltas = start_codes + seg_x2 as usize;
    let id_range_offsets = id_deltas + seg_x2 as usize;
    for i in 0..segs {
        let (Some(end), Some(start), Some(delta), Some(range_off)) = (
            read_u16(data, end_codes + i * 2),
            read_u16(data, start_codes + i * 2),
            read_i16(data, id_deltas + i * 2),
            read_u16(data, id_range_offsets + i * 2),
        ) else {
            return false;
        };
        if c < start || c > end {
            continue;
        }
        let glyph = if range_off == 0 {
            c.wrapping_add(delta as u16)
        } else {
            let pos = id_range_offsets + i * 2 + range_off as usize + (c - start) as usize * 2;
            let Some(g) = read_u16(data, pos) else {
                return false;
            };
            if g == 0 {
                return false;
            }
            g.wrapping_add(delta as u16)
        };
        return glyph != 0;
    }
    false
}

/// cmap 格式 12（分组覆盖，支持增补平面）。
fn cmap_format12_has_char(data: &[u8], sub: usize, ch: u32) -> bool {
    let Some(groups) = read_u32(data, sub + 12) else {
        return false;
    };
    let first = sub + 16;
    let available = data.len().saturating_sub(first) / 12;
    for i in 0..(groups as usize).min(available) {
        let g = first + i * 12;
        let (Some(start), Some(end), Some(start_glyph)) = (
            read_u32(data, g),
            read_u32(data, g + 4),
            read_u32(data, g + 8),
        ) else {
            return false;
        };
        if ch < start {
            break;
        }
        if ch <= end {
            return start_glyph.wrapping_add(ch - start) != 0;
        }
    }
    false
}

fn read_u16(data: &[u8], offset: usize) -> Option<u16> {
    Some(u16::from_be_bytes([
        *data.get(offset)?,
        *data.get(offset + 1)?,
    ]))
}

fn read_i16(data: &[u8], offset: usize) -> Option<i16> {
    Some(i16::from_be_bytes([
        *data.get(offset)?,
        *data.get(offset + 1)?,
    ]))
}

fn read_u32(data: &[u8], offset: usize) -> Option<u32> {
    Some(u32::from_be_bytes([
        *data.get(offset)?,
        *data.get(offset + 1)?,
        *data.get(offset + 2)?,
        *data.get(offset + 3)?,
    ]))
}
