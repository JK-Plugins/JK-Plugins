//! 色の合成まで済ませた結果のキャッシュ（新しい順）。
//!
//! 2コマ・3コマ打ちのアニメでは同じ絵のフレームが続くので、入力の画素と設定が前と同じなら
//! 合成（`shade`、GPU も）と float への変換を飛ばし、覚えておいた「変わった画素の並び」を
//! そのまま出力に書き込む。GPU と CPU のどちらで作った結果も同じ形（`Changes::List` の並び）で持つ。
//! AE に依存しないのでテストできる。

use std::sync::{Arc, Mutex};

/// 変わった画素の並び（入力の番号 y*w+x、色）。番号の昇順
pub type ChangedList = Vec<(u32, [f32; 4])>;

pub struct ResultCache {
    entries: Mutex<Vec<(u64, Arc<ChangedList>)>>,
    max_entries: usize,
    max_bytes: usize,
}

/// 1つの結果のおおよそのメモリ使用量
fn bytes_of(v: &ChangedList) -> usize {
    v.len() * std::mem::size_of::<(u32, [f32; 4])>()
}

impl ResultCache {
    /// `max_entries` 個まで、合計 `max_bytes` まで（いちばん新しい1つはそれより大きくても残す）
    pub const fn new(max_entries: usize, max_bytes: usize) -> Self {
        Self {
            entries: Mutex::new(Vec::new()),
            max_entries,
            max_bytes,
        }
    }

    /// 見つかれば、それをいちばん新しいものにして返す
    pub fn get(&self, key: u64) -> Option<Arc<ChangedList>> {
        let mut e = self.entries.lock().ok()?;
        let pos = e.iter().position(|(k, _)| *k == key)?;
        let hit = e.remove(pos);
        let v = hit.1.clone();
        e.insert(0, hit);
        Some(v)
    }

    pub fn put(&self, key: u64, v: Arc<ChangedList>) {
        let Ok(mut e) = self.entries.lock() else {
            return;
        };
        e.retain(|(k, _)| *k != key);
        e.insert(0, (key, v));
        let mut bytes = 0;
        let mut keep = 0;
        for (_, v) in e.iter() {
            bytes += bytes_of(v);
            if keep >= self.max_entries || (keep > 0 && bytes > self.max_bytes) {
                break;
            }
            keep += 1;
        }
        e.truncate(keep);
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.entries.lock().unwrap().len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::{
        prepare, shade, GroupColor, Settings, Texture, TextureKind, TextureParams,
    };
    use crate::world_io::{
        compose, dense_to_list, encode, read_world, Changes, Layout, Rows, Source,
    };

    fn list(n: usize) -> Arc<ChangedList> {
        Arc::new(vec![(0, [0.0; 4]); n])
    }

    #[test]
    fn lru_order_and_limits() {
        let c = ResultCache::new(2, usize::MAX);
        c.put(1, list(1));
        c.put(2, list(1));
        // 1 を使うと新しくなり、次に入れたとき 2 が消える
        assert!(c.get(1).is_some());
        c.put(3, list(1));
        assert!(c.get(2).is_none());
        assert!(c.get(1).is_some() && c.get(3).is_some());
        assert_eq!(c.len(), 2);

        // メモリの上限：新しいものは大きくても残し、古いものを捨てる
        let per = std::mem::size_of::<(u32, [f32; 4])>();
        let c = ResultCache::new(8, 10 * per);
        c.put(1, list(6));
        c.put(2, list(6));
        assert!(c.get(1).is_none());
        assert!(c.get(2).is_some());
        c.put(3, list(100));
        assert_eq!(c.len(), 1);
        assert!(c.get(3).is_some());
    }

    /// キャッシュから書いた出力が、合成し直して書いた出力と同じになる
    /// （CPU の並び・GPU の全画素から作った並びのどちらでも）
    #[test]
    fn cached_result_writes_same_output() {
        let (w, h) = (40usize, 24usize);
        let layout = Layout::Argb8;
        let src: Vec<[f32; 4]> = (0..w * h)
            .map(|i| {
                let (x, y) = (i % w, i / w);
                let a = if (x + y) % 11 == 0 { 0.5 } else { 1.0 };
                if x * 2 + y < w {
                    [1.0, 1.0, 1.0, a]
                } else {
                    [0.0, 0.0, 0.0, a]
                }
            })
            .collect();
        let mut in_buf = vec![0u8; w * h * 4];
        for (i, p) in src.iter().enumerate() {
            encode(layout, *p, &mut in_buf[i * 4..i * 4 + 4]);
        }
        let rows = Rows {
            stride: w * 4,
            flip: false,
            h,
        };
        let input = Source {
            buf: &in_buf,
            rows,
            w,
            layout,
        };
        // 8bit に丸めた値で計算する（プラグインと同じ）
        let src = read_world(&in_buf, rows, w, layout);
        let s = Settings {
            colors: vec![GroupColor::new([1.0; 3]), GroupColor::new([0.0; 3])],
            width: 10.0,
            alpha_threshold: 0.0,
            ..Default::default()
        };
        let tex = Texture::new(
            TextureParams {
                kind: TextureKind::Dot,
                ..Default::default()
            },
            None,
        );
        let p = prepare(w, h, &src, &s);
        let out = shade(&p, &src, &s, &tex);
        assert!(!out.changed.is_empty());

        let write = |c: Changes, dx: i32, dy: i32| {
            let (ow, oh) = (30, 20);
            let rows = Rows {
                stride: ow * 4,
                flip: false,
                h: oh,
            };
            let mut buf = vec![0x5au8; ow * 4 * oh];
            compose(&input, &mut buf, rows, ow, layout, dx, dy, c);
            buf
        };

        let cache = ResultCache::new(4, usize::MAX);
        cache.put(7, Arc::new(out.changed.clone()));
        let hit = cache.get(7).unwrap();
        for (dx, dy) in [(0, 0), (5, 3), (-4, -2)] {
            assert_eq!(
                write(Changes::List(&out.changed), dx, dy),
                write(Changes::List(&hit), dx, dy)
            );
        }

        // GPU の形（全画素、w：0 変更なし / 1 アルファは入力 / 2 アルファ 1）から作った並びも同じ
        let mut dense = vec![[0f32; 4]; w * h];
        for (i, (idx, c)) in out.changed.iter().enumerate() {
            let flag = if i % 2 == 0 { 2.0 } else { 1.0 };
            dense[*idx as usize] = [c[0], c[1], c[2], flag];
        }
        let from_dense = dense_to_list(&dense, &input);
        for (dx, dy) in [(0, 0), (5, 3), (-4, -2)] {
            assert_eq!(
                write(Changes::Dense(&dense), dx, dy),
                write(Changes::List(&from_dense), dx, dy)
            );
        }
    }
}
