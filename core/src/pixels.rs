//! 撮った画面の画素 (BGRA 8bit)。GDI で取れる形のまま持ち、切り出しと RGBA への変換だけをここでやる。

use crate::geom::Rect;

/// 仮想デスクトップ (またはその一部) の画素。origin は、data の左上がデスクトップ座標のどこか。
#[derive(Debug, Clone)]
pub struct Bgra {
    pub origin: (i32, i32),
    pub w: i32,
    pub h: i32,
    pub data: Vec<u8>, // w * h * 4、上の行から
}

impl Bgra {
    pub fn rect(&self) -> Rect {
        Rect::new(self.origin.0, self.origin.1, self.w, self.h)
    }

    /// デスクトップ座標の範囲 r を切り出す。範囲が画素の外にはみ出していたら、内側だけ。全部外なら None。
    pub fn crop(&self, r: &Rect) -> Option<Bgra> {
        let hit = self.rect().intersect(r)?;
        let (sx, sy) = (hit.x - self.origin.0, hit.y - self.origin.1);
        let mut out = Vec::with_capacity((hit.w * hit.h * 4) as usize);
        for row in 0..hit.h {
            let start = (((sy + row) * self.w + sx) * 4) as usize;
            out.extend_from_slice(&self.data[start..start + (hit.w * 4) as usize]);
        }
        Some(Bgra { origin: (hit.x, hit.y), w: hit.w, h: hit.h, data: out })
    }

    /// 青と赤を入れ替えて RGBA にする。アルファは全部不透明にする (GDI のアルファは当てにならない)。
    pub fn to_rgba(&self) -> Vec<u8> {
        let mut out = self.data.clone();
        for px in out.chunks_exact_mut(4) {
            px.swap(0, 2);
            px[3] = 255;
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // 4x3 の画素。値は (x, y) から決めて、どこを切ったか分かるようにする
    fn sample(origin: (i32, i32)) -> Bgra {
        let (w, h) = (4, 3);
        let mut data = Vec::new();
        for y in 0..h {
            for x in 0..w {
                data.extend_from_slice(&[x as u8, y as u8, 99, 0]); // B=x, G=y, R=99
            }
        }
        Bgra { origin, w, h, data }
    }

    #[test]
    fn crop_inside() {
        let c = sample((0, 0)).crop(&Rect::new(1, 1, 2, 2)).unwrap();
        assert_eq!((c.w, c.h, c.origin), (2, 2, (1, 1)));
        // 左上は元の (1,1)
        assert_eq!(&c.data[0..4], &[1, 1, 99, 0]);
        // 右下は元の (2,2)
        assert_eq!(&c.data[12..16], &[2, 2, 99, 0]);
    }

    #[test]
    fn crop_with_negative_origin() {
        // 画素の左上がデスクトップの (-4, -3) にあるとき、(-3,-2) は画素の (1,1)
        let c = sample((-4, -3)).crop(&Rect::new(-3, -2, 1, 1)).unwrap();
        assert_eq!(&c.data[0..4], &[1, 1, 99, 0]);
    }

    #[test]
    fn crop_clamps_and_misses() {
        let s = sample((0, 0));
        let c = s.crop(&Rect::new(-5, -5, 7, 7)).unwrap(); // 左上にはみ出す
        assert_eq!((c.w, c.h), (2, 2));
        assert!(s.crop(&Rect::new(10, 10, 5, 5)).is_none());
    }

    #[test]
    fn rgba_swaps_and_opaque() {
        let c = Bgra { origin: (0, 0), w: 1, h: 1, data: vec![10, 20, 30, 0] };
        assert_eq!(c.to_rgba(), vec![30, 20, 10, 255]);
    }
}
