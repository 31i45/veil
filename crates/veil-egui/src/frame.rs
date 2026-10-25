//! 纯像素合成器：块位图、光标、preedit 合成进窗口帧。零 GUI 依赖，headless 可测。
//! 像素格式 RGBA8 straight alpha；`into_u32` 转 softbuffer 所需 opaque XRGB。

pub struct Frame {
    pub pixels: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

impl Frame {
    pub fn new(width: u32, height: u32, bg: [u8; 4]) -> Self {
        let mut pixels = vec![0u8; (width * height * 4) as usize];
        for p in pixels.chunks_exact_mut(4) {
            p.copy_from_slice(&bg);
        }
        Frame {
            pixels,
            width,
            height,
        }
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
                let di = ((dy_ as u32 * self.width + dx_ as u32) * 4) as usize;
                over(&mut self.pixels[di..di + 4], &src[si..si + 4]);
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
                let di = ((dy_ as u32 * self.width + dx_ as u32) * 4) as usize;
                over(&mut self.pixels[di..di + 4], &src[si..si + 4]);
            }
        }
    }

    /// 不透明矩形（光标、下划线、底色块）。越界裁剪。
    pub fn fill_rect(&mut self, x: i32, y: i32, w: u32, h: u32, color: [u8; 4]) {
        let (x0, y0) = (x.max(0), y.max(0));
        let x1 = (x + w as i32).min(self.width as i32);
        let y1 = (y + h as i32).min(self.height as i32);
        for py in y0..y1 {
            for px_ in x0..x1 {
                let i = ((py as u32 * self.width + px_ as u32) * 4) as usize;
                over(&mut self.pixels[i..i + 4], &color);
            }
        }
    }

    /// 转 softbuffer 的 u32 像素（XRGB：窗口不透明，alpha 丢弃）。
    pub fn into_u32(&self) -> Vec<u32> {
        self.pixels
            .chunks_exact(4)
            .map(|p| ((p[0] as u32) << 16) | ((p[1] as u32) << 8) | p[2] as u32)
            .collect()
    }
}

fn over(dst: &mut [u8], src: &[u8]) {
    let a = src[3] as u32;
    let ia = 255 - a;
    dst[0] = ((src[0] as u32 * a + dst[0] as u32 * ia) / 255) as u8;
    dst[1] = ((src[1] as u32 * a + dst[1] as u32 * ia) / 255) as u8;
    dst[2] = ((src[2] as u32 * a + dst[2] as u32 * ia) / 255) as u8;
    dst[3] = (a + dst[3] as u32 * ia / 255) as u8;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_fills_background() {
        let f = Frame::new(2, 2, [10, 20, 30, 255]);
        assert_eq!(
            f.pixels,
            vec![10, 20, 30, 255, 10, 20, 30, 255, 10, 20, 30, 255, 10, 20, 30, 255]
        );
    }

    #[test]
    fn blit_opaque_overwrites() {
        let mut f = Frame::new(4, 1, [0, 0, 0, 255]);
        f.blit(&[200, 100, 50, 255], 1, 1, 2, 0);
        assert_eq!(&f.pixels[8..12], &[200, 100, 50, 255]);
    }

    #[test]
    fn blit_half_alpha_blends() {
        let mut f = Frame::new(1, 1, [0, 0, 0, 255]);
        f.blit(&[255, 0, 0, 128], 1, 1, 0, 0);
        assert_eq!(f.pixels[0], 128); // 255*128/255
        assert_eq!(f.pixels[3], 255);
    }

    #[test]
    fn blit_clips_negative_origin() {
        let mut f = Frame::new(2, 2, [0, 0, 0, 255]);
        let src = vec![
            255u8, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 9, 9, 9, 255,
        ]; // 2x2
        f.blit(&src, 2, 2, -1, -1); // src(1,1)=[9,9,9] 落到帧内 (0,0)，其余裁剪
        assert_eq!(&f.pixels[0..4], &[9, 9, 9, 255]);
        assert_eq!(&f.pixels[12..16], &[0, 0, 0, 255]); // (1,1) 不染
    }

    #[test]
    fn fill_rect_clips_to_bounds() {
        let mut f = Frame::new(4, 4, [0, 0, 0, 255]);
        f.fill_rect(-2, -2, 4, 4, [1, 2, 3, 255]); // 覆盖 [0,2)x[0,2)
        assert_eq!(&f.pixels[0..4], &[1, 2, 3, 255]);
        assert_eq!(&f.pixels[10 * 4..10 * 4 + 4], &[0, 0, 0, 255]); // (2,2) 不染
    }

    #[test]
    fn into_u32_packs_xrgb() {
        let f = Frame::new(1, 1, [0x12, 0x34, 0x56, 255]);
        assert_eq!(f.into_u32(), vec![0x123456]);
    }

    #[test]
    fn blit_scaled_nearest_neighbor_expands_pixels() {
        // 2x1 → 4x1：每个源像素复制两份
        let mut f = Frame::new(4, 1, [0, 0, 0, 255]);
        let src = [255u8, 0, 0, 255, 0, 255, 0, 255];
        f.blit_scaled(&src, 2, 1, 4, 1, 0, 0);
        assert_eq!(&f.pixels[0..4], &[255, 0, 0, 255]);
        assert_eq!(&f.pixels[4..8], &[255, 0, 0, 255]);
        assert_eq!(&f.pixels[8..12], &[0, 255, 0, 255]);
        assert_eq!(&f.pixels[12..16], &[0, 255, 0, 255]);
    }

    #[test]
    fn blit_scaled_clips_and_zero_size_is_noop() {
        let mut f = Frame::new(2, 2, [0, 0, 0, 255]);
        let src = [9u8, 9, 9, 255];
        f.blit_scaled(&src, 1, 1, 2, 2, 1, 1); // 仅 (1,1) 可见
        assert_eq!(&f.pixels[12..16], &[9, 9, 9, 255]);
        assert_eq!(&f.pixels[0..4], &[0, 0, 0, 255]);
        f.blit_scaled(&src, 1, 1, 0, 0, 0, 0); // 零尺寸：无副作用
        assert_eq!(&f.pixels[0..4], &[0, 0, 0, 255]);
    }
}
