//! 左下のアイテム欄から、持っているアイテムを読む。
//! 枠は、deadlock-api のショップの画像（`shop_image`）を縮めて、決まった位置に置いたもの。

use image::RgbaImage;

use super::{Frame, Read, Template};

/// これより近くなければ「不明」にする（空の枠、鍵の掛かった枠、クールダウンで暗い枠）。
/// 空の枠は 0.73 まで上がり、暗くない枠は 0.83 まで下がる（Disarming Hex）ので、その間に置く
pub const THRESHOLD: f32 = 0.8;
/// 枠の大きさ（1080p）
const SLOT: f32 = 54.0;
/// 隣の枠までの幅（横も縦も）
const SPACING: f32 = 62.0;
/// 左上の枠の、画面での位置
const FIRST: (f32, f32) = (26.0, 946.0);
/// ショップを開いている間は、欄がこれだけ上にずれる
const SHOP_RISE: f32 = 40.0;
/// 枠の並び（横、縦）。鍵の掛かった枠も数える
const GRID: (usize, usize) = (6, 2);

/// 枠のうち、比べる範囲。右上は段の印（III など）が重なり、縁は枠の線が掛かる
fn compared(x: f32, y: f32) -> bool {
    let inside = |v: f32| (0.06..0.94).contains(&v);
    inside(x) && inside(y) && x - y < 0.35
}

/// ショップの画像から、画面の大きさ（`scale` は画面の高さ ÷ 1080）に合わせた見本を作る
pub fn templates<'a>(
    images: impl IntoIterator<Item = (&'a str, &'a RgbaImage)>,
    scale: f32,
) -> Vec<Template> {
    let size = (SLOT * scale).round() as u32;
    images
        .into_iter()
        .map(|(name, image)| Template::new(name, image, (size, size), compared))
        .collect()
}

/// 読んだアイテム欄
pub struct Slots {
    /// 上の段の左から順に
    pub reads: Vec<Read>,
    /// ショップを開いている位置で読めた
    pub shop: bool,
}

/// アイテム欄を読む。ショップを開いているかは分からないので、両方の位置を読んで、読めた枠の多いほうを採る
/// （同じなら、開いていないほう）
pub fn read(frame: &Frame, templates: &[Template], scale: f32) -> Slots {
    let at = |rise: f32| -> Vec<Read> {
        (0..GRID.1)
            .flat_map(|row| (0..GRID.0).map(move |column| (row, column)))
            .map(|(row, column)| {
                let left = (FIRST.0 + SPACING * column as f32) * scale;
                let top = (FIRST.1 - rise + SPACING * row as f32) * scale;
                super::read(frame, templates, left.round() as i32, top.round() as i32)
            })
            .collect()
    };
    // 近さの合計で比べると、アイテムが少ないときに、空の枠の点の揺れで入れ替わる。読めた枠の数で比べる
    let known = |reads: &[Read]| {
        reads
            .iter()
            .filter(|read| read.answer(THRESHOLD).is_some())
            .count()
    };
    let (usual, raised) = (at(0.0), at(SHOP_RISE));
    let shop = known(&raised) > known(&usual);
    Slots {
        reads: if shop { raised } else { usual },
        shop,
    }
}
