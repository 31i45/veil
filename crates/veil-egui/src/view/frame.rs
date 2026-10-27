//! 纯像素合成器：块位图、光标、preedit 合成进窗口帧。零 GUI 依赖，headless 可测。
//! 帧直接持有 softbuffer 所需的 opaque XRGB（`0x00RRGGBB`）u32 像素，
//! 不再保留 RGBA 中转缓冲，绘制结束即 `copy_from_slice` 上屏。
//! 输入位图（文本光栅、公式图、图片）仍为 straight-alpha RGBA8。

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
}
