//! グラフで編集するカーブ（Curve = Custom）。
//!
//! 両端 (0,0) と (1,1) は固定し、その間の点を、行き過ぎない単調な3次スプライン
//! （Fritsch–Carlson）でつなぐ。重み 0 / 1 のところは変わらないので、グラデーションの端で段差が出ない。

/// 両端を除いた点の最大数
pub const MAX_POINTS: usize = 8;

/// 表の分割数
const LUT_SIZE: usize = 256;

/// 点（両端を除く）からカーブの点列（両端を含み x の昇順、x が近すぎる点は1つにまとめる）を作る
pub fn knots(points: &[[f32; 2]]) -> Vec<[f32; 2]> {
    let mut inner: Vec<[f32; 2]> = points
        .iter()
        .take(MAX_POINTS)
        .filter(|p| p[0].is_finite() && p[1].is_finite())
        .map(|p| [p[0].clamp(0.0, 1.0), p[1].clamp(0.0, 1.0)])
        .filter(|p| p[0] > 1e-4 && p[0] < 1.0 - 1e-4)
        .collect();
    inner.sort_by(|a, b| a[0].partial_cmp(&b[0]).unwrap());
    let mut k = vec![[0.0, 0.0]];
    for p in inner {
        if p[0] - k.last().unwrap()[0] > 1e-4 {
            k.push(p);
        }
    }
    k.push([1.0, 1.0]);
    k
}

/// 単調な3次スプライン（Fritsch–Carlson）。`knots` の点を通り、点と点の間で行き過ぎない
pub struct Spline {
    x: Vec<f32>,
    y: Vec<f32>,
    m: Vec<f32>,
}

impl Spline {
    pub fn new(points: &[[f32; 2]]) -> Self {
        let k = knots(points);
        let n = k.len();
        let x: Vec<f32> = k.iter().map(|p| p[0]).collect();
        let y: Vec<f32> = k.iter().map(|p| p[1]).collect();
        let d: Vec<f32> = (0..n - 1)
            .map(|i| (y[i + 1] - y[i]) / (x[i + 1] - x[i]))
            .collect();
        let mut m = vec![0f32; n];
        m[0] = d[0];
        m[n - 1] = d[n - 2];
        for i in 1..n - 1 {
            m[i] = if d[i - 1] * d[i] <= 0.0 {
                0.0
            } else {
                (d[i - 1] + d[i]) * 0.5
            };
        }
        for i in 0..n - 1 {
            if d[i] == 0.0 {
                m[i] = 0.0;
                m[i + 1] = 0.0;
                continue;
            }
            let a = m[i] / d[i];
            let b = m[i + 1] / d[i];
            let s = a * a + b * b;
            if s > 9.0 {
                let t = 3.0 / s.sqrt();
                m[i] = t * a * d[i];
                m[i + 1] = t * b * d[i];
            }
        }
        Self { x, y, m }
    }

    pub fn eval(&self, t: f32) -> f32 {
        let t = t.clamp(0.0, 1.0);
        let n = self.x.len();
        let mut i = self.x.partition_point(|&v| v <= t).saturating_sub(1);
        if i >= n - 1 {
            i = n - 2;
        }
        let h = self.x[i + 1] - self.x[i];
        let s = (t - self.x[i]) / h;
        let (s2, s3) = (s * s, s * s * s);
        let h00 = 2.0 * s3 - 3.0 * s2 + 1.0;
        let h10 = s3 - 2.0 * s2 + s;
        let h01 = -2.0 * s3 + 3.0 * s2;
        let h11 = s3 - s2;
        (h00 * self.y[i] + h10 * h * self.m[i] + h01 * self.y[i + 1] + h11 * h * self.m[i + 1])
            .clamp(0.0, 1.0)
    }
}

/// 描画で使う表（0..1 を 256 分割して線形補間）
pub struct CurveLut {
    v: Vec<f32>,
}

impl CurveLut {
    pub fn new(points: &[[f32; 2]]) -> Self {
        let sp = Spline::new(points);
        let v = (0..=LUT_SIZE)
            .map(|i| sp.eval(i as f32 / LUT_SIZE as f32))
            .collect();
        Self { v }
    }

    #[inline]
    pub fn eval(&self, t: f32) -> f32 {
        let f = t.clamp(0.0, 1.0) * LUT_SIZE as f32;
        let i = (f as usize).min(LUT_SIZE - 1);
        let r = f - i as f32;
        self.v[i] + (self.v[i + 1] - self.v[i]) * r
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_points_is_linear() {
        let l = CurveLut::new(&[]);
        for i in 0..=10 {
            let t = i as f32 / 10.0;
            assert!((l.eval(t) - t).abs() < 1e-4);
        }
    }

    #[test]
    fn ends_are_fixed_and_passes_through_points() {
        let pts = [[0.25, 0.6], [0.7, 0.8]];
        let sp = Spline::new(&pts);
        assert_eq!(sp.eval(0.0), 0.0);
        assert_eq!(sp.eval(1.0), 1.0);
        for p in pts {
            assert!((sp.eval(p[0]) - p[1]).abs() < 1e-5);
        }
    }

    /// 点が単調に増えるなら、カーブも単調に増える（行き過ぎない）
    #[test]
    fn monotone_points_give_monotone_curve() {
        let l = CurveLut::new(&[[0.1, 0.5], [0.15, 0.55], [0.8, 0.6]]);
        let mut prev = 0.0;
        for i in 0..=1000 {
            let v = l.eval(i as f32 / 1000.0);
            assert!(v + 1e-6 >= prev, "{i}");
            assert!((0.0..=1.0).contains(&v));
            prev = v;
        }
    }

    /// 順番がばらばらでも、範囲外でも、同じ x が重なっていても落ちない
    #[test]
    fn messy_points_are_handled() {
        let l = CurveLut::new(&[
            [0.9, 0.2],
            [0.3, 1.5],
            [0.3, 0.1],
            [-1.0, 0.5],
            [f32::NAN, 0.1],
        ]);
        for i in 0..=100 {
            let v = l.eval(i as f32 / 100.0);
            assert!(v.is_finite() && (0.0..=1.0).contains(&v));
        }
        assert_eq!(knots(&[[0.9, 0.2], [0.3, 1.5], [0.3, 0.1]]).len(), 4);
    }
}
