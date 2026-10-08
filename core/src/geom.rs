//! 画面の座標計算。
//!
//! 座標はすべて「仮想デスクトップ」の物理ピクセル (Windows の DPI 仮想化を外した値)。
//! 原点は主モニタの左上。副モニタが左や上にあると、座標は負になる。

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Rect {
    pub fn new(x: i32, y: i32, w: i32, h: i32) -> Self {
        Rect { x, y, w, h }
    }
    pub fn right(&self) -> i32 {
        self.x + self.w
    }
    pub fn bottom(&self) -> i32 {
        self.y + self.h
    }
    pub fn is_empty(&self) -> bool {
        self.w <= 0 || self.h <= 0
    }
    /// 2 つの矩形の重なり。重ならなければ None。
    pub fn intersect(&self, o: &Rect) -> Option<Rect> {
        let x = self.x.max(o.x);
        let y = self.y.max(o.y);
        let r = self.right().min(o.right());
        let b = self.bottom().min(o.bottom());
        if r > x && b > y { Some(Rect::new(x, y, r - x, b - y)) } else { None }
    }
    /// 2 点 (ドラッグの始点と終点) から、向きに関わらず正規化した矩形を作る。
    pub fn from_points(ax: i32, ay: i32, bx: i32, by: i32) -> Rect {
        Rect::new(ax.min(bx), ay.min(by), (ax - bx).abs(), (ay - by).abs())
    }
}

/// すべてのモニタを含む最小の矩形 (仮想デスクトップ全体)。モニタが無ければ None。
pub fn bounding(monitors: &[Rect]) -> Option<Rect> {
    let first = monitors.first()?;
    let (mut l, mut t, mut r, mut b) = (first.x, first.y, first.right(), first.bottom());
    for m in &monitors[1..] {
        l = l.min(m.x);
        t = t.min(m.y);
        r = r.max(m.right());
        b = b.max(m.bottom());
    }
    Some(Rect::new(l, t, r - l, b - t))
}

/// 点 (x, y) があるモニタの番号。どれにも無ければ None。
pub fn monitor_at(monitors: &[Rect], x: i32, y: i32) -> Option<usize> {
    monitors.iter().position(|m| x >= m.x && x < m.right() && y >= m.y && y < m.bottom())
}

/// 選んだ範囲を、モニタごとの断片に分ける。
/// 範囲が複数のモニタにまたがっても、各モニタの画像から切り出して貼り合わせられるようにする。
/// 戻り値: (モニタの番号, そのモニタの画像の中の切り出し範囲, 出力画像の中での貼り付け位置)
pub fn split_by_monitor(region: &Rect, monitors: &[Rect]) -> Vec<(usize, Rect, (i32, i32))> {
    let mut out = Vec::new();
    for (i, m) in monitors.iter().enumerate() {
        if let Some(hit) = region.intersect(m) {
            let src = Rect::new(hit.x - m.x, hit.y - m.y, hit.w, hit.h);
            out.push((i, src, (hit.x - region.x, hit.y - region.y)));
        }
    }
    out
}

/// 範囲の全体を、モニタの合計の内側に収める。外にはみ出した分は切り捨てる。
/// 完全に外なら None。
pub fn clamp_to_desktop(region: &Rect, monitors: &[Rect]) -> Option<Rect> {
    let b = bounding(monitors)?;
    region.intersect(&b)
}

#[cfg(test)]
mod tests {
    use super::*;

    // 左に副モニタ (座標が負)、右に主モニタ
    fn two() -> Vec<Rect> {
        vec![Rect::new(-1920, 0, 1920, 1080), Rect::new(0, 0, 2560, 1440)]
    }

    #[test]
    fn bounding_with_negative_origin() {
        assert_eq!(bounding(&two()), Some(Rect::new(-1920, 0, 4480, 1440)));
        assert_eq!(bounding(&[]), None);
    }

    #[test]
    fn from_points_any_direction() {
        assert_eq!(Rect::from_points(10, 20, 5, 8), Rect::new(5, 8, 5, 12));
        assert_eq!(Rect::from_points(5, 8, 10, 20), Rect::new(5, 8, 5, 12));
    }

    #[test]
    fn monitor_lookup_edges() {
        let m = two();
        assert_eq!(monitor_at(&m, -1, 10), Some(0));
        assert_eq!(monitor_at(&m, 0, 10), Some(1));
        assert_eq!(monitor_at(&m, -1920, 0), Some(0));
        assert_eq!(monitor_at(&m, -1921, 0), None);
        // 左モニタの方が低い (1080) ので、その下の空き地はどこにも属さない
        assert_eq!(monitor_at(&m, -10, 1200), None);
    }

    #[test]
    fn region_across_two_monitors() {
        let m = two();
        let region = Rect::new(-100, 50, 300, 200); // 境目をまたぐ
        let parts = split_by_monitor(&region, &m);
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[0], (0, Rect::new(1820, 50, 100, 200), (0, 0)));
        assert_eq!(parts[1], (1, Rect::new(0, 50, 200, 200), (100, 0)));
        // 断片の面積の合計 = 範囲の面積
        let area: i32 = parts.iter().map(|p| p.1.w * p.1.h).sum();
        assert_eq!(area, region.w * region.h);
    }

    #[test]
    fn region_in_a_gap_is_partial() {
        let m = two();
        // 左モニタの下端 (1080) をまたぐ範囲: 空き地の分は断片にならない
        let region = Rect::new(-200, 1000, 100, 200);
        let parts = split_by_monitor(&region, &m);
        assert_eq!(parts.len(), 1);
        assert_eq!(parts[0].1, Rect::new(1720, 1000, 100, 80));
    }

    #[test]
    fn clamp() {
        let m = two();
        // 左と上にはみ出す範囲は、デスクトップの内側だけ残る
        assert_eq!(clamp_to_desktop(&Rect::new(-2000, -10, 200, 100), &m), Some(Rect::new(-1920, 0, 120, 90)));
        // 完全に外 (左端 -1920 より左) なら None
        assert_eq!(clamp_to_desktop(&Rect::new(-3000, -10, 500, 100), &m), None);
        assert_eq!(clamp_to_desktop(&Rect::new(5000, 0, 10, 10), &m), None);
    }
}
