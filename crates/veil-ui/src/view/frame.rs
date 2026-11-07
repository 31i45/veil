//! 纯像素合成器：块位图、光标、preedit 合成进窗口帧。零 GUI 依赖，headless 可测。
//! 帧直接持有 softbuffer 所需的 opaque XRGB（`0x00RRGGBB`）u32 像素，
//! 不再保留 RGBA 中转缓冲，绘制结束即 `copy_from_slice` 上屏。
//! 输入位图（文本光栅、公式图、图片）仍为 straight-alpha RGBA8。

use crate::theme::surface;

pub struct Frame {
    pub pixels: Vec<u32>,
    pub width: u32,
    pub height: u32,
}

/// RGBA8 → XRGB（窗口不透明，alpha 丢弃）。
fn pack(c: [u8; 4]) -> u32 {
    ((c[0] as u32) << 16) | ((c[1] as u32) << 8) | c[2] as u32
}

impl Frame {
    pub fn new(width: u32, height: u32, bg: [u8; 4]) -> Self {
        let count = (width as usize) * (height as usize);
        Self {
            pixels: vec![pack(bg); count],
            width,
            height,
        }
    }

    /// 复用已有分配重建尺寸/底色（每帧零分配；尺寸变化才重新分配）。
    pub fn reset(&mut self, width: u32, height: u32, bg: [u8; 4]) {
        let count = (width as usize) * (height as usize);
        self.width = width;
        self.height = height;
        self.pixels.resize(count, 0);
        self.pixels.fill(pack(bg));
    }

    /// 读取像素为 RGBA8（alpha 恒 255），测试与调试用。
    pub fn get(&self, x: u32, y: u32) -> [u8; 4] {
        let p = self.pixels[(y as usize) * (self.width as usize) + x as usize];
        [
            ((p >> 16) & 0xFF) as u8,
            ((p >> 8) & 0xFF) as u8,
            (p & 0xFF) as u8,
            255,
        ]
    }

    /// src-over 混合位图到 (dx, dy)，越界部分裁剪。src 为 straight alpha RGBA8。
    pub fn blit(&mut self, src: &[u8], sw: u32, sh: u32, dx: i32, dy: i32) {
        for sy in 0..sh as i32 {
            let dy_ = dy + sy;
            if dy_ < 0 || dy_ >= self.height as i32 {
                continue;
            }
            for sx in 0..sw as i32 {
                let dx_ = dx + sx;
                if dx_ < 0 || dx_ >= self.width as i32 {
                    continue;
                }
                let si = ((sy as u32 * sw + sx as u32) * 4) as usize;
                let di = (dy_ as usize) * (self.width as usize) + dx_ as usize;
                over(&mut self.pixels[di], &src[si..si + 4]);
            }
        }
    }

    /// src-over 混合位图到 (dx, dy)，按目标尺寸 (dw, dh) 最近邻缩放，越界裁剪。
    /// 图片按版心宽等比缩放后绘制（不缓存缩放副本），src 为 straight alpha RGBA8。
    #[allow(clippy::too_many_arguments)] // 内部位图操作参数天然多
    pub fn blit_scaled(
        &mut self,
        src: &[u8],
        sw: u32,
        sh: u32,
        dw: u32,
        dh: u32,
        dx: i32,
        dy: i32,
    ) {
        if sw == 0 || sh == 0 || dw == 0 || dh == 0 {
            return;
        }
        for y in 0..dh as i32 {
            let dy_ = dy + y;
            if dy_ < 0 || dy_ >= self.height as i32 {
                continue;
            }
            let sy = (y as u64 * sh as u64 / dh as u64) as u32;
            for x in 0..dw as i32 {
                let dx_ = dx + x;
                if dx_ < 0 || dx_ >= self.width as i32 {
                    continue;
                }
                let sx = (x as u64 * sw as u64 / dw as u64) as u32;
                let si = ((sy * sw + sx) * 4) as usize;
                let di = (dy_ as usize) * (self.width as usize) + dx_ as usize;
                over(&mut self.pixels[di], &src[si..si + 4]);
            }
        }
    }

    /// 矩形（光标、下划线、底色块）。越界裁剪；不透明色走直写快路径。
    pub fn fill_rect(&mut self, x: i32, y: i32, w: u32, h: u32, color: [u8; 4]) {
        let (x0, y0) = (x.max(0), y.max(0));
        let x1 = (x + w as i32).min(self.width as i32);
        let y1 = (y + h as i32).min(self.height as i32);
        let packed = pack(color);
        for py in y0..y1 {
            let row = (py as usize) * (self.width as usize);
            for px_ in x0..x1 {
                let dst = &mut self.pixels[row + px_ as usize];
                if color[3] == 255 {
                    *dst = packed;
                } else {
                    over(dst, &color);
                }
            }
        }
    }

    /// 圆角矩形（浮层 surface 原语）：四角为四分之一圆，边缘按几何覆盖率抗锯齿。
    /// `radius` 为物理像素（f32）；≤0.5 时退化为直角 `fill_rect`。
    ///
    /// 覆盖率算法（扫描线近似）：逐行求圆边界相对左右边的内缩量
    /// `inset = r - √(r² - dy²)`，仅角区每行一次开方（半径 6 时全角共 ≤12 次），
    /// 不逐像素开方；边界像素覆盖率按水平线性近似（把边界局部当竖直边），
    /// 相对逐像素二维距离法的覆盖率误差 ≤0.1（r≤12 时肉眼不可辨）。
    pub(crate) fn fill_rounded_rect(
        &mut self,
        x: i32,
        y: i32,
        w: u32,
        h: u32,
        radius: f32,
        color: [u8; 4],
    ) {
        if w == 0 || h == 0 {
            return;
        }
        // 半径钳制在短边一半内（胶囊上限）；负值按 0 走直角
        let r = radius.clamp(0.0, w.min(h) as f32 * 0.5);
        if r <= 0.5 {
            // 0.5px 内覆盖率近乎阶跃，抗锯齿无意义，走直角快路径
            self.fill_rect(x, y, w, h, color);
            return;
        }
        let (x0, y0) = (x as f32, y as f32);
        let (x1, y1) = (x0 + w as f32, y0 + h as f32);
        let r2 = r * r;
        // 角区外的直行带（行中心 cy=py+0.5 落在两角心水平线之间）：整带宽直写快路径
        let band0 = (y0 + r - 0.5).ceil() as i32; // 首个 interior 行
        let band1 = (y1 - r - 0.5).floor() as i32; // 末个 interior 行
        if band1 >= band0 {
            self.fill_rect(x, band0, w, (band1 - band0 + 1) as u32, color);
        }
        // 角区行：上 [y, band0) 与下 (band1, y1)。由带界定义保证 0 < dy < r，开方安全
        for py in (y..band0).chain(band1 + 1..y + h as i32) {
            let cy = py as f32 + 0.5;
            let dy = (y0 + r - cy).max(cy - (y1 - r)); // 到最近角心水平线的竖直距离
            let inset = r - (r2 - dy * dy).sqrt();
            let (lx, rx) = (x0 + inset, x1 - inset); // 行内可见区间 [lx, rx)
            let (pl, pr) = (lx.floor() as i32, rx.floor() as i32);
            if pl == pr {
                // 两侧边界落进同一像素（尖角行）：覆盖率 ≈ 区间宽
                self.blend_px(pl, py, color, rx - lx);
            } else {
                if pr > pl + 1 {
                    self.fill_rect(pl + 1, py, (pr - pl - 1) as u32, 1, color);
                }
                // 左边界像素覆盖率 = 1 - fract(lx)；右边界 = fract(rx)
                self.blend_px(pl, py, color, 1.0 - (lx - pl as f32));
                self.blend_px(pr, py, color, rx - pr as f32);
            }
        }
    }

    /// 柔和投影（浮层 surface 原语）：矩形正下方多层外扩圆角矩形叠出柔和阴影，
    /// 观感 ≈ 8px 高斯模糊 + 2px 下移。阴影为半透明黑叠加，深/浅主题均成立
    /// （深色下按 `surface::SHADOW_DARK_GAIN` 略增 alpha）。仅在浮层打开时调用。
    #[allow(clippy::too_many_arguments)] // 内部位图操作参数天然多（同 blit_scaled）
    pub(crate) fn fill_shadow(
        &mut self,
        x: i32,
        y: i32,
        w: u32,
        h: u32,
        radius: f32,
        scale: f32,
        dark: bool,
    ) {
        let off = surface::shadow_offset_y(scale).round() as i32;
        // 自外向内叠：外层大而淡、内层小而浓；浮层背景后画，会盖住矩形本体区的阴影
        for &(spread, base_alpha) in surface::SHADOW_LAYERS {
            let e = (spread * scale).round() as i32;
            let a = surface::shadow_alpha(base_alpha, dark);
            self.fill_rounded_rect(
                x - e,
                y - e + off,
                (w as i32 + 2 * e) as u32,
                (h as i32 + 2 * e) as u32,
                radius + spread * scale, // 外扩层圆角同步外扩，保持同心
                [0, 0, 0, a],
            );
        }
    }

    /// surface 卡片（全部浮层的统一观感）：柔和投影 + 1px 圆角边框 + 圆角底。
    /// 边框用「外框圆角矩形（边框色）+ 内缩 1px 圆角矩形（底色）」叠出，
    /// 四个角与底同曲率，无直边接缝。x/y/w/h 为面板外边界（阴影自此处外扩）。
    #[allow(clippy::too_many_arguments)] // 内部位图操作参数天然多（同 blit_scaled）
    pub(crate) fn fill_surface_card(
        &mut self,
        x: i32,
        y: i32,
        w: u32,
        h: u32,
        scale: f32,
        dark: bool,
        bg: [u8; 4],
        border: [u8; 4],
    ) {
        let r = surface::corner_radius(scale);
        self.fill_shadow(x, y, w, h, r, scale, dark);
        self.fill_rounded_rect(x, y, w, h, r, border);
        if w > 2 && h > 2 {
            self.fill_rounded_rect(x + 1, y + 1, w - 2, h - 2, (r - 1.0).max(0.0), bg);
        }
    }

    /// 悬停/选中项（surface 原语）：相对条目矩形四边内缩 `hover_inset` 的圆角矩形，
    /// 曲率比面板小一级，形成「浮起」的高亮块。
    pub(crate) fn fill_hover(
        &mut self,
        x: i32,
        y: i32,
        w: u32,
        h: u32,
        scale: f32,
        color: [u8; 4],
    ) {
        let inset = surface::hover_inset(scale).round() as i32;
        let r = (surface::corner_radius(scale) - 2.0 * scale).max(0.0);
        if w <= (2 * inset) as u32 || h <= (2 * inset) as u32 {
            return;
        }
        self.fill_rounded_rect(
            x + inset,
            y + inset,
            w - (2 * inset) as u32,
            h - (2 * inset) as u32,
            r,
            color,
        );
    }

    /// 单像素覆盖率混合（圆角边缘抗锯齿用）：有效 alpha = color.a × cov，src-over。
    fn blend_px(&mut self, x: i32, y: i32, color: [u8; 4], cov: f32) {
        if cov <= 0.0 || x < 0 || y < 0 || x >= self.width as i32 || y >= self.height as i32 {
            return;
        }
        let a = (color[3] as f32 * cov.min(1.0)).round() as u8;
        if a == 0 {
            return;
        }
        let i = (y as usize) * (self.width as usize) + x as usize;
        over(&mut self.pixels[i], &[color[0], color[1], color[2], a]);
    }
}

fn over(dst: &mut u32, src: &[u8]) {
    let a = src[3] as u32;
    if a == 0 {
        return;
    }
    if a == 255 {
        *dst = pack([src[0], src[1], src[2], 255]);
        return;
    }
    let ia = 255 - a;
    let d = *dst;
    let r = (src[0] as u32 * a + ((d >> 16) & 0xFF) * ia) / 255;
    let g = (src[1] as u32 * a + ((d >> 8) & 0xFF) * ia) / 255;
    let b = (src[2] as u32 * a + (d & 0xFF) * ia) / 255;
    *dst = (r << 16) | (g << 8) | b;
}

#[cfg(test)]
mod tests {
    use super::*;

    const BLACK: [u8; 4] = [0, 0, 0, 255];

    #[test]
    fn new_fills_background() {
        let f = Frame::new(2, 2, [10, 20, 30, 255]);
        assert_eq!(f.pixels, vec![0x0A141E; 4]);
        assert_eq!(f.get(1, 1), [10, 20, 30, 255]);
    }

    #[test]
    fn reset_reuses_buffer_and_refills() {
        let mut f = Frame::new(2, 2, [1, 2, 3, 255]);
        let cap = f.pixels.capacity();
        f.reset(2, 2, [4, 5, 6, 255]);
        assert_eq!(f.pixels.capacity(), cap, "同尺寸复用分配");
        assert_eq!(f.get(0, 0), [4, 5, 6, 255]);
        f.reset(3, 1, [7, 8, 9, 255]);
        assert_eq!((f.width, f.height, f.pixels.len()), (3, 1, 3));
        assert_eq!(f.get(2, 0), [7, 8, 9, 255]);
    }

    #[test]
    fn blit_opaque_overwrites() {
        let mut f = Frame::new(4, 1, BLACK);
        f.blit(&[200, 100, 50, 255], 1, 1, 2, 0);
        assert_eq!(f.get(2, 0), [200, 100, 50, 255]);
    }

    #[test]
    fn blit_half_alpha_blends() {
        let mut f = Frame::new(1, 1, BLACK);
        f.blit(&[255, 0, 0, 128], 1, 1, 0, 0);
        assert_eq!(f.get(0, 0), [128, 0, 0, 255]); // 255*128/255
    }

    #[test]
    fn blit_transparent_keeps_destination() {
        let mut f = Frame::new(1, 1, [9, 8, 7, 255]);
        f.blit(&[255, 0, 0, 0], 1, 1, 0, 0);
        assert_eq!(f.get(0, 0), [9, 8, 7, 255]);
    }

    #[test]
    fn blit_clips_negative_origin() {
        let mut f = Frame::new(2, 2, BLACK);
        let src = vec![
            255u8, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 9, 9, 9, 255,
        ]; // 2x2
        f.blit(&src, 2, 2, -1, -1); // src(1,1)=[9,9,9] 落到帧内 (0,0)，其余裁剪
        assert_eq!(f.get(0, 0), [9, 9, 9, 255]);
        assert_eq!(f.get(1, 1), BLACK); // (1,1) 不染
    }

    #[test]
    fn fill_rect_clips_to_bounds() {
        let mut f = Frame::new(4, 4, BLACK);
        f.fill_rect(-2, -2, 4, 4, [1, 2, 3, 255]); // 覆盖 [0,2)x[0,2)
        assert_eq!(f.get(0, 0), [1, 2, 3, 255]);
        assert_eq!(f.get(2, 2), BLACK); // (2,2) 不染
    }

    #[test]
    fn fill_rect_half_alpha_blends() {
        let mut f = Frame::new(1, 1, BLACK);
        f.fill_rect(0, 0, 1, 1, [255, 255, 255, 128]);
        assert_eq!(f.get(0, 0), [128, 128, 128, 255]);
    }

    #[test]
    fn frame_is_xrgb_packed() {
        let f = Frame::new(1, 1, [0x12, 0x34, 0x56, 255]);
        assert_eq!(f.pixels, vec![0x123456]);
    }

    #[test]
    fn blit_scaled_nearest_neighbor_expands_pixels() {
        // 2x1 → 4x1：每个源像素复制两份
        let mut f = Frame::new(4, 1, BLACK);
        let src = [255u8, 0, 0, 255, 0, 255, 0, 255];
        f.blit_scaled(&src, 2, 1, 4, 1, 0, 0);
        assert_eq!(f.get(0, 0), [255, 0, 0, 255]);
        assert_eq!(f.get(1, 0), [255, 0, 0, 255]);
        assert_eq!(f.get(2, 0), [0, 255, 0, 255]);
        assert_eq!(f.get(3, 0), [0, 255, 0, 255]);
    }

    #[test]
    fn blit_scaled_clips_and_zero_size_is_noop() {
        let mut f = Frame::new(2, 2, BLACK);
        let src = [9u8, 9, 9, 255];
        f.blit_scaled(&src, 1, 1, 2, 2, 1, 1); // 仅 (1,1) 可见
        assert_eq!(f.get(1, 1), [9, 9, 9, 255]);
        assert_eq!(f.get(0, 0), BLACK);
        f.blit_scaled(&src, 1, 1, 0, 0, 0, 0); // 零尺寸：无副作用
        assert_eq!(f.get(0, 0), BLACK);
    }

    const WHITE: [u8; 4] = [255, 255, 255, 255];

    #[test]
    fn rounded_rect_center_full_corner_outside_untouched_edge_blended() {
        let mut f = Frame::new(16, 16, WHITE);
        f.fill_rounded_rect(0, 0, 16, 16, 4.0, BLACK);
        // 中心像素：全色
        assert_eq!(f.get(8, 8), BLACK);
        // 顶边中点（无角影响）：全色
        assert_eq!(f.get(8, 0), BLACK);
        // 角外像素：角心 (4,4)，(0,0) 中心距 √(3.5²+3.5²)≈4.95 > r=4 → 不变
        assert_eq!(f.get(0, 0), WHITE);
        // 顶行 r=4：inset = 4 - √(16-3.5²) ≈ 2.064，左边界像素 (2,0) 覆盖率 ≈0.936
        let edge = f.get(2, 0);
        assert!(
            edge[0] > 0 && edge[0] < 255,
            "角边像素应为黑白之间的混合中间值: {edge:?}"
        );
        // 边界外像素 (1,0)：覆盖率 <0 → 不变
        assert_eq!(f.get(1, 0), WHITE);
        // 左右角对称
        assert_eq!(f.get(13, 0), edge);
    }

    #[test]
    fn rounded_rect_tiny_radius_is_square() {
        let mut f = Frame::new(8, 8, WHITE);
        f.fill_rounded_rect(0, 0, 8, 8, 0.0, BLACK);
        assert_eq!(f.get(0, 0), BLACK, "半径 ≤0.5 退化为直角");
    }

    #[test]
    fn shadow_darkens_below_and_fades_with_distance() {
        let mut f = Frame::new(64, 64, WHITE);
        f.fill_shadow(16, 16, 16, 8, 6.0, 1.0, false);
        // 矩形底边 y=24（不含），正下方中心列 x=24
        let near = f.get(24, 25); // 底边下 1px：全部 5 层覆盖
        let far = f.get(24, 30); // 底边下 6px：仅外扩 6/8 两层覆盖
        assert!(near[0] < 255, "矩形正下方应变暗: {near:?}");
        assert!(far[0] < 255, "外扩层覆盖处也应变暗: {far:?}");
        assert!(near[0] < far[0], "离矩形越远阴影越浅: {near:?} vs {far:?}");
        // 阴影带（底 + 偏移 2 + 外扩 8 = 33）之外不受影响
        assert_eq!(f.get(24, 34), WHITE);
    }
}
