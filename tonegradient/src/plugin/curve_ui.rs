//! Curve = Custom のときにエフェクトコントロールに出すカーブのグラフ。
//!
//! - クリック：近くの点をつかむ（無ければその場所に点を追加）
//! - ドラッグ：つかんだ点を動かす
//! - グラフの外までドラッグして離す：その点を消す
//! 両端 (0,0)・(1,1) は固定。点の間は単調な3次スプライン（engine::curve）でつなぐ。

use super::*;
use crate::engine::curve::{Spline, MAX_POINTS};

/// グラフの大きさ（エフェクトコントロールの中の領域、px）
pub const UI_WIDTH: u16 = 200;
pub const UI_HEIGHT: u16 = 200;
/// グラフの周りの余白
const PAD: f32 = 12.0;
/// 点をつかめる距離（px）
const HIT: f32 = 8.0;
/// グラフの外へこれだけ出して離したら点を消す（px）
const REMOVE: f32 = 16.0;

/// カーブの点（両端を除く）。arbitrary data として保存し、キーフレームで補間する
#[derive(serde::Serialize, serde::Deserialize, PartialEq, PartialOrd, Clone, Copy, Debug)]
#[repr(C)]
pub struct CurveData {
    pub n: u32,
    pub x: [f32; MAX_POINTS],
    pub y: [f32; MAX_POINTS],
}

impl Default for CurveData {
    fn default() -> Self {
        Self {
            n: 0,
            x: [0.0; MAX_POINTS],
            y: [0.0; MAX_POINTS],
        }
    }
}

impl CurveData {
    pub fn points(&self) -> Vec<[f32; 2]> {
        (0..(self.n as usize).min(MAX_POINTS))
            .map(|i| [self.x[i], self.y[i]])
            .collect()
    }

    fn remove(&mut self, i: usize) {
        let n = self.n as usize;
        for k in i..n.saturating_sub(1) {
            self.x[k] = self.x[k + 1];
            self.y[k] = self.y[k + 1];
        }
        self.n = (n - 1) as u32;
    }

    /// x の昇順に並べ直す（つかんでいる点の番号を返す）
    fn sort(&mut self, keep: usize) -> usize {
        let n = self.n as usize;
        let mut idx: Vec<usize> = (0..n).collect();
        idx.sort_by(|&a, &b| self.x[a].partial_cmp(&self.x[b]).unwrap());
        let (x, y) = (self.x, self.y);
        let mut new_keep = keep;
        for (k, &i) in idx.iter().enumerate() {
            self.x[k] = x[i];
            self.y[k] = y[i];
            if i == keep {
                new_keep = k;
            }
        }
        new_keep
    }
}

impl ae::ArbitraryData<CurveData> for CurveData {
    /// 点の数が同じなら形がなめらかに変わる。違うときは途中で切り替わる
    fn interpolate(&self, other: &Self, v: f64) -> Self {
        if self.n != other.n {
            return if v < 0.5 { *self } else { *other };
        }
        let t = v as f32;
        let mut out = *self;
        for i in 0..MAX_POINTS {
            out.x[i] = self.x[i] + (other.x[i] - self.x[i]) * t;
            out.y[i] = self.y[i] + (other.y[i] - self.y[i]) * t;
        }
        out
    }
}

/// グラフの領域（エフェクトコントロールの座標）
struct Area {
    x0: f32,
    y0: f32,
    w: f32,
    h: f32,
}

impl Area {
    fn of(event: &ae::EventExtra) -> Self {
        // AE は行の幅いっぱいを渡してくるので、高さに合わせた正方形にして左に寄せる
        let f = event.current_frame();
        let fw = (f.right - f.left) as f32;
        let fh = (f.bottom - f.top) as f32;
        let side = (fh.min(UI_HEIGHT as f32) - 2.0 * PAD)
            .min(fw - 2.0 * PAD)
            .max(10.0);
        Self {
            x0: f.left as f32 + PAD,
            y0: f.top as f32 + PAD,
            w: side,
            h: side,
        }
    }
    fn to_screen(&self, p: [f32; 2]) -> (f32, f32) {
        (self.x0 + p[0] * self.w, self.y0 + (1.0 - p[1]) * self.h)
    }
    fn to_curve(&self, sx: f32, sy: f32) -> [f32; 2] {
        [(sx - self.x0) / self.w, 1.0 - (sy - self.y0) / self.h]
    }
    /// グラフからどれだけ外に出ているか（px、中なら 0）
    fn outside(&self, sx: f32, sy: f32) -> f32 {
        let dx = (self.x0 - sx).max(sx - (self.x0 + self.w)).max(0.0);
        let dy = (self.y0 - sy).max(sy - (self.y0 + self.h)).max(0.0);
        dx.max(dy)
    }
}

fn custom_enabled(params: &ae::Parameters<Params>) -> bool {
    params
        .get(Params::CurveType)
        .and_then(|p| p.as_popup().map(|p| p.value()))
        .map(|v| v == 3)
        .unwrap_or(false)
}

fn read(params: &ae::Parameters<Params>) -> Result<CurveData, Error> {
    Ok(*params
        .get(Params::CurveGraph)?
        .as_arbitrary()?
        .value::<CurveData>()?)
}

fn write(params: &mut ae::Parameters<Params>, data: CurveData) -> Result<(), Error> {
    params
        .get_mut(Params::CurveGraph)?
        .as_arbitrary_mut()?
        .set_value::<CurveData>(data)?;
    Ok(())
}

/// クリック・ドラッグの位置（エフェクトコントロールの座標）
fn mouse(event: &ae::EventExtra) -> (f32, f32) {
    let p = event.screen_point();
    (p.h as f32, p.v as f32)
}

pub fn draw(params: &ae::Parameters<Params>, event: &mut ae::EventExtra) -> Result<(), Error> {
    if event.effect_area() != ae::EffectArea::Control {
        return Ok(());
    }
    let enabled = custom_enabled(params);
    let data = read(params).unwrap_or_default();
    let area = Area::of(event);
    let drawbot = event.context_handle().drawing_reference()?;
    let supplier = drawbot.supplier()?;
    let surface = drawbot.surface()?;
    let a = if enabled { 1.0 } else { 0.35 };
    let rgba = |v: f32, alpha: f32| ae::drawbot::ColorRgba {
        red: v,
        green: v,
        blue: v,
        alpha,
    };

    // 背景
    let mut bg = supplier.new_path()?;
    bg.add_rect(&ae::drawbot::RectF32 {
        left: area.x0,
        top: area.y0,
        width: area.w,
        height: area.h,
    })?;
    surface.fill_path(
        &supplier.new_brush(&rgba(0.13, 1.0))?,
        &bg,
        ae::drawbot::FillType::Winding,
    )?;

    // 目盛り（4分割）と対角線（Linear と同じ）
    let grid_pen = supplier.new_pen(&rgba(0.32, a), 1.0)?;
    let mut grid = supplier.new_path()?;
    for i in 1..4 {
        let t = i as f32 / 4.0;
        grid.move_to(area.x0 + t * area.w, area.y0)?;
        grid.line_to(area.x0 + t * area.w, area.y0 + area.h)?;
        grid.move_to(area.x0, area.y0 + t * area.h)?;
        grid.line_to(area.x0 + area.w, area.y0 + t * area.h)?;
    }
    surface.stroke_path(&grid_pen, &grid)?;
    let mut diag = supplier.new_path()?;
    diag.move_to(area.x0, area.y0 + area.h)?;
    diag.line_to(area.x0 + area.w, area.y0)?;
    surface.stroke_path(&supplier.new_pen(&rgba(0.42, a), 1.0)?, &diag)?;
    surface.stroke_path(&supplier.new_pen(&rgba(0.45, a), 1.0)?, &bg)?;

    // カーブ
    let sp = Spline::new(&data.points());
    let mut curve = supplier.new_path()?;
    let steps = 96;
    for i in 0..=steps {
        let t = i as f32 / steps as f32;
        let (sx, sy) = area.to_screen([t, sp.eval(t)]);
        if i == 0 {
            curve.move_to(sx, sy)?;
        } else {
            curve.line_to(sx, sy)?;
        }
    }
    surface.stroke_path(&supplier.new_pen(&rgba(0.95, a), 1.5)?, &curve)?;

    // 点（両端は灰色、動かせる点はオレンジ）
    let dot = |p: [f32; 2], c: ae::drawbot::ColorRgba| -> Result<(), Error> {
        let (sx, sy) = area.to_screen(p);
        let mut r = supplier.new_path()?;
        r.add_rect(&ae::drawbot::RectF32 {
            left: sx - 3.0,
            top: sy - 3.0,
            width: 6.0,
            height: 6.0,
        })?;
        surface.fill_path(&supplier.new_brush(&c)?, &r, ae::drawbot::FillType::Winding)?;
        Ok(())
    };
    dot([0.0, 0.0], rgba(0.6, a))?;
    dot([1.0, 1.0], rgba(0.6, a))?;
    for p in data.points() {
        dot(
            [p[0].clamp(0.0, 1.0), p[1].clamp(0.0, 1.0)],
            ae::drawbot::ColorRgba {
                red: 1.0,
                green: 0.6,
                blue: 0.2,
                alpha: a,
            },
        )?;
    }

    event.set_event_out_flags(ae::EventOutFlags::HANDLED_EVENT);
    Ok(())
}

pub fn click(params: &mut ae::Parameters<Params>, event: &mut ae::EventExtra) -> Result<(), Error> {
    if event.effect_area() != ae::EffectArea::Control || !custom_enabled(params) {
        return Ok(());
    }
    let area = Area::of(event);
    let (mx, my) = mouse(event);
    if area.outside(mx, my) > 0.0 {
        return Ok(());
    }
    let mut data = read(params)?;
    // 近くの点をつかむ
    let mut grab = None;
    let mut best = HIT;
    for (i, p) in data.points().iter().enumerate() {
        let (sx, sy) = area.to_screen(*p);
        let d = ((sx - mx).powi(2) + (sy - my).powi(2)).sqrt();
        if d <= best {
            best = d;
            grab = Some(i);
        }
    }
    // 無ければ点を追加（両端の近くと、いっぱいのときは追加しない）
    if grab.is_none() {
        let c = area.to_curve(mx, my);
        if (data.n as usize) < MAX_POINTS && c[0] > 0.02 && c[0] < 0.98 {
            let i = data.n as usize;
            data.x[i] = c[0];
            data.y[i] = c[1].clamp(0.0, 1.0);
            data.n += 1;
            let i = data.sort(i);
            write(params, data)?;
            grab = Some(i);
        }
    }
    if let Some(i) = grab {
        // つかんだ点の番号（+1）をドラッグに引き継ぐ
        event.set_continue_refcon(0, (i + 1) as _);
        event.set_send_drag(true);
    }
    event.set_event_out_flags(ae::EventOutFlags::HANDLED_EVENT);
    Ok(())
}

pub fn drag(params: &mut ae::Parameters<Params>, event: &mut ae::EventExtra) -> Result<(), Error> {
    if event.effect_area() != ae::EffectArea::Control {
        return Ok(());
    }
    let grabbed = event.continue_refcon(0) as usize;
    if grabbed == 0 {
        return Ok(());
    }
    let mut data = read(params)?;
    let i = grabbed - 1;
    if i >= data.n as usize {
        return Ok(());
    }
    let area = Area::of(event);
    let (mx, my) = mouse(event);
    if event.last_time() && area.outside(mx, my) >= REMOVE {
        // グラフの外で離した：点を消す
        data.remove(i);
        write(params, data)?;
        event.set_continue_refcon(0, 0);
    } else {
        let c = area.to_curve(mx, my);
        data.x[i] = c[0].clamp(0.01, 0.99);
        data.y[i] = c[1].clamp(0.0, 1.0);
        let i = data.sort(i);
        write(params, data)?;
        event.set_continue_refcon(0, (i + 1) as _);
        event.set_send_drag(!event.last_time());
    }
    event.set_event_out_flags(ae::EventOutFlags::HANDLED_EVENT);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interpolate_same_count_blends_points() {
        let mut a = CurveData::default();
        a.n = 1;
        a.x[0] = 0.2;
        a.y[0] = 0.2;
        let mut b = a;
        b.x[0] = 0.6;
        b.y[0] = 1.0;
        let m = ae::ArbitraryData::interpolate(&a, &b, 0.5);
        assert!((m.x[0] - 0.4).abs() < 1e-6 && (m.y[0] - 0.6).abs() < 1e-6);
        let mut c = a;
        c.n = 2;
        assert_eq!(ae::ArbitraryData::interpolate(&a, &c, 0.4), a);
        assert_eq!(ae::ArbitraryData::interpolate(&a, &c, 0.6), c);
    }

    #[test]
    fn sort_keeps_grabbed_point() {
        let mut d = CurveData::default();
        d.n = 3;
        d.x[..3].copy_from_slice(&[0.8, 0.2, 0.5]);
        d.y[..3].copy_from_slice(&[0.8, 0.2, 0.5]);
        let k = d.sort(0);
        assert_eq!(&d.x[..3], &[0.2, 0.5, 0.8]);
        assert_eq!(k, 2);
        d.remove(1);
        assert_eq!(d.n, 2);
        assert_eq!(&d.x[..2], &[0.2, 0.8]);
    }
}
