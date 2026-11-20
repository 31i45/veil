use super::*;
use crate::theme::Theme;
use crate::view::Frame;
use cosmic_text::fontdb::Source;
use std::fmt::Write as _;
use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use veil_core::Session;

fn write_ppm(path: &Path, frame: &Frame) {
    let mut file = fs::File::create(path).unwrap();
    write!(file, "P6\n{} {}\n255\n", frame.width, frame.height).unwrap();
    let mut rgb = Vec::with_capacity(frame.pixels.len() * 3);
    for &pixel in &frame.pixels {
        rgb.extend_from_slice(&[
            ((pixel >> 16) & 0xff) as u8,
            ((pixel >> 8) & 0xff) as u8,
            (pixel & 0xff) as u8,
        ]);
    }
    file.write_all(&rgb).unwrap();
}

fn source_path(source: &Source) -> String {
    match source {
        Source::Binary(_) => "<memory>".to_string(),
        Source::File(path) => path.display().to_string(),
        Source::SharedFile(path, _) => path.display().to_string(),
    }
}

fn append_face_evidence(
    eng: &mut LayoutEngine,
    label: &str,
    layout: &BlockLayout,
    out: &mut String,
) {
    writeln!(out, "layout={label}\tscripts={:?}", layout.script_ranges).unwrap();
    for (line_i, run) in layout.buffer.layout_runs().enumerate() {
        for glyph in run.glyphs {
            let cluster = &run.text[glyph.start..glyph.end];
            let face = eng
                .font_system()
                .db_mut()
                .face(glyph.font_id)
                .unwrap_or_else(|| panic!("missing face for glyph {}", glyph.glyph_id));
            let families = face
                .families
                .iter()
                .map(|(name, _)| name.as_str())
                .collect::<Vec<_>>()
                .join("|");
            writeln!(
                out,
                "line={line_i}\tline_y={:.6}\ttext={cluster:?}\tglyph_id={}\tfont_size={:.6}\tglyph_weight={:?}\tfamilies={families}\tpostscript={}\tface_index={}\tface_weight={:?}\tstyle={:?}\tsource={}",
                run.line_y,
                glyph.glyph_id,
                glyph.font_size,
                glyph.font_weight,
                face.post_script_name,
                face.index,
                face.weight,
                face.style,
                source_path(&face.source),
            )
            .unwrap();
        }
    }
}

/// 受控排版矩阵：与真机走查共用 corpus，通过 LayoutEngine::set_scale 模拟物理像素缩放。
/// 仅设置 VEIL_TYPOGRAPHY_CAPTURE 与 VEIL_TYPOGRAPHY_CORPUS 时产出证据，普通测试不落地文件。
#[test]
fn capture_typography_matrix() {
    let Some(out) = std::env::var_os("VEIL_TYPOGRAPHY_CAPTURE") else {
        return;
    };
    let corpus_path = PathBuf::from(
        std::env::var_os("VEIL_TYPOGRAPHY_CORPUS").expect("VEIL_TYPOGRAPHY_CORPUS is required"),
    );
    let out = PathBuf::from(out);
    fs::create_dir_all(&out).unwrap();
    let corpus = fs::read_to_string(&corpus_path).unwrap();
    let session = Session::open(&corpus);
    let blocks = session.blocks().to_vec();
    let mut manifest = format!(
        "capture=LayoutEngine::set_scale controlled physical-pixel matrix\n corpus={}\n",
        corpus_path.display()
    );
    let mut faces = String::from(
        "source=cosmic-text LayoutGlyph.font_id + glyph_id resolved through FontSystem.db().face()\n",
    );

    for scale in [1.0_f32, 1.25, 1.5, 2.0] {
        for dark in [false, true] {
            let theme = if dark { Theme::dark() } else { Theme::light() };
            let mut eng = LayoutEngine::new();
            eng.set_scale(scale);
            let margin = (16.0 * scale).round() as i32;
            let gap = (10.0 * scale).round() as i32;
            let width = (760.0 * scale).ceil() as u32;
            let height = (320.0 * scale).ceil() as u32;
            let text_width = (width as i32 - margin * 2) as f32;
            let mut frame = Frame::new(width, height, theme.bg);
            let mut layouts = vec![(
                "UI 13px: 文件 编辑 视图 / File Edit View".to_string(),
                eng.layout_ui("UI 13px · 文件 编辑 视图 / File Edit View"),
            )];
            for (index, (id, kind, range)) in blocks.iter().enumerate() {
                let inlines = session.block_inlines(*id).unwrap_or_default();
                let layout = eng.layout_block(
                    kind.clone(),
                    &corpus[range.start..range.end],
                    *range,
                    &inlines,
                    text_width,
                    usize::MAX,
                    &[],
                    false,
                );
                layouts.push((format!("block {index}: {kind:?}"), layout));
            }

            let mut y = margin;
            for (label, layout) in &layouts {
                let (pixels, raster_w, raster_h) = eng.rasterize(layout, theme.text);
                assert!(
                    pixels.chunks_exact(4).any(|px| px[3] != 0),
                    "empty raster: scale={scale} dark={dark} label={label}"
                );
                assert!(
                    margin >= 0
                        && y >= 0
                        && margin as u32 + raster_w <= width
                        && y as u32 + raster_h <= height,
                    "clipped raster: scale={scale} dark={dark} label={label} raster={raster_w}x{raster_h} at {margin},{y} frame={width}x{height}"
                );
                frame.blit(&pixels, raster_w, raster_h, margin, y);
                if scale == 1.0 && !dark {
                    append_face_evidence(&mut eng, label, layout, &mut faces);
                }
                let sizes = layout
                    .buffer
                    .layout_runs()
                    .flat_map(|run| run.glyphs)
                    .map(|glyph| glyph.font_size)
                    .collect::<Vec<_>>();
                writeln!(
                    manifest,
                    "scale={scale:.2}\tdark={dark}\tlabel={label}\traster={raster_w}x{raster_h}\torigin={margin},{y}\tfont_sizes={sizes:?}"
                )
                .unwrap();
                y += raster_h as i32 + gap;
            }
            assert!(y <= height as i32, "capture content exceeds frame");
            let stem = format!(
                "engine-{}-{:03}",
                if dark { "dark" } else { "light" },
                (scale * 100.0).round() as u32
            );
            write_ppm(&out.join(format!("{stem}.ppm")), &frame);
            writeln!(manifest, "image={stem}.png\tframe={width}x{height}").unwrap();
        }
    }
    fs::write(out.join("capture.txt"), manifest).unwrap();
    fs::write(out.join("font-faces.txt"), faces).unwrap();
}

fn paragraph(eng: &mut LayoutEngine, text: &str, width: f32) -> BlockLayout {
    let session = Session::open(text);
    let (id, kind, range) = session.blocks()[0].clone();
    let inlines = session.block_inlines(id).unwrap_or_default();
    eng.layout_block(
        kind,
        &text[range.start..range.end],
        range,
        &inlines,
        width,
        usize::MAX,
        &[],
        false,
    )
}

fn assert_matches_integer_baseline(
    eng: &mut LayoutEngine,
    layout: &BlockLayout,
    scripts: &[(usize, usize, f32)],
    target: Option<usize>,
) {
    let line_y = layout.buffer.layout_runs().next().unwrap().line_y;
    let effective = line_y + target.map_or(0.0, |index| scripts[index].2);
    let delta = effective.round() - effective;
    assert!(delta.abs() > 0.01, "test needs a fractional baseline");
    let actual = eng.rasterize_buffer_scripts(&layout.buffer, layout.height, [30, 30, 30], scripts);

    let mut reference = scripts.to_vec();
    match target {
        Some(index) => reference[index].2 += delta,
        None => reference = vec![(0, layout.layout_text().len(), delta)],
    }
    let expected =
        eng.rasterize_buffer_scripts(&layout.buffer, layout.height, [30, 30, 30], &reference);
    assert_eq!((actual.1, actual.2), (expected.1, expected.2));
    let mismatches = actual
        .0
        .iter()
        .zip(&expected.0)
        .filter(|(a, b)| a != b)
        .count();
    assert!(
        mismatches == 0,
        "fractional baseline must rasterize exactly like its rounded integer physical-pixel baseline; target={target:?} line_y={line_y} effective={effective} mismatching_bytes={mismatches}"
    );
}

#[test]
fn raster_baseline_snap_normal_text_matches_integer_reference() {
    let mut eng = LayoutEngine::new();
    eng.set_scale(1.25);
    let layout = eng.layout_preedit("Veil 中文 baseline 123");
    assert_matches_integer_baseline(&mut eng, &layout, &[], None);
}

#[test]
fn raster_baseline_snap_sub_sup_matches_integer_reference() {
    let mut eng = LayoutEngine::new();
    eng.set_scale(1.25);
    let layout = paragraph(&mut eng, "H<sub>2</sub>O + x<sup>2</sup>", 500.0);
    assert_eq!(layout.script_ranges.len(), 2);
    for target in 0..layout.script_ranges.len() {
        assert_matches_integer_baseline(&mut eng, &layout, &layout.script_ranges, Some(target));
    }
}
