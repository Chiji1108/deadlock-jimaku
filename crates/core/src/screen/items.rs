//! 左下のアイテム欄から、持っているアイテムを読む。
//! 枠は、deadlock-api のショップの画像（`shop_image`）を縮めて、決まった位置に置いたもの。

use image::RgbaImage;

use super::{Frame, Read, Template};

/// これより近くなければ「不明」にする（空の枠、鍵の掛かった枠、クールダウンで暗い枠）。
/// 空の枠は 0.73 まで上がり、暗くない枠は 0.83 まで下がる（Disarming Hex）ので、その間に置く
pub const THRESHOLD: f32 = 0.83;
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

/// 段の印（枠の右上の三角）の色。色相（度）と、彩度の幅。アイテムの種類（武器、バイタリティ、スピリット）で決まる。
/// 印は、アイテムがクールダウンで暗い間も同じ色で出ていて、空の枠にはない。空の枠は後ろの景色が透けるが、
/// 色相と彩度の両方がここに入ることは少ない（DESIGN.md 18.3）
const BADGES: [(f32, (f32, f32)); 3] =
    [(35.5, (0.6, 1.0)), (85.0, (0.6, 1.0)), (273.5, (0.3, 0.55))];
/// 印の色相は、この幅まで外れてよい
const BADGE_HUE: f32 = 8.0;
/// 印は、これより暗くならない（明るさは 0〜1）
const BADGE_VALUE: f32 = 0.15;

/// 枠のうち、段の印だけが出ている範囲
fn badge(x: f32, y: f32) -> bool {
    let inside = |v: f32| (0.06..0.94).contains(&v);
    inside(x) && inside(y) && x - y >= 0.58
}

/// 左上が画面の `(left, top)` にある枠に、段の印が出ているか（アイテムが入っているか）。
/// 印の範囲の色の、中央の値の色相と彩度で見る。
/// 空の枠の後ろに印と同じ色の景色（炎など）が来ると、入っていると読み違えるが、入っている枠を空と読むことは、
/// 記録になかった。使う側は、こちらの向きにだけ頼る
fn occupied(frame: &Frame, left: i32, top: i32, size: u32) -> bool {
    let mut pixels: Vec<[u8; 3]> = Vec::new();
    for y in 0..size {
        for x in 0..size {
            let at = (
                (x as f32 + 0.5) / size as f32,
                (y as f32 + 0.5) / size as f32,
            );
            let (x, y) = (
                left + x as i32 - frame.origin.0,
                top + y as i32 - frame.origin.1,
            );
            if badge(at.0, at.1)
                && x >= 0
                && y >= 0
                && let Some(pixel) = frame.image.get_pixel_checked(x as u32, y as u32)
            {
                pixels.push(pixel.0);
            }
        }
    }
    if pixels.is_empty() {
        return false;
    }
    // 段の数字（暗い線）が重なるので、平均ではなく中央の値を採る
    let middle: [u8; 3] = std::array::from_fn(|channel| {
        let mut values: Vec<u8> = pixels.iter().map(|pixel| pixel[channel]).collect();
        *values.select_nth_unstable(pixels.len() / 2).1
    });
    let [r, g, b] = middle.map(|value| f32::from(value) / 255.0);
    let (max, min) = (r.max(g).max(b), r.min(g).min(b));
    let spread = max - min;
    if max < BADGE_VALUE || spread == 0.0 {
        return false;
    }
    let hue = 60.0
        * if max == r {
            ((g - b) / spread).rem_euclid(6.0)
        } else if max == g {
            (b - r) / spread + 2.0
        } else {
            (r - g) / spread + 4.0
        };
    let saturation = spread / max;
    BADGES.iter().any(|(badge, (low, high))| {
        (hue - badge).abs() <= BADGE_HUE && (*low..=*high).contains(&saturation)
    })
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

/// 読むのに要る画面の範囲（左、上、幅、高さ）。ショップを開いているときの位置も入れる
pub fn area(screen: (u32, u32), scale: f32) -> [u32; 4] {
    let left = ((FIRST.0 - super::MARGIN) * scale).floor().max(0.0) as u32;
    let top = ((FIRST.1 - SHOP_RISE - super::MARGIN) * scale)
        .floor()
        .max(0.0) as u32;
    let right = (FIRST.0 + SPACING * (GRID.0 - 1) as f32 + SLOT + super::MARGIN) * scale;
    let bottom = (FIRST.1 + SPACING * (GRID.1 - 1) as f32 + SLOT + super::MARGIN) * scale;
    let (right, bottom) = (
        (right.ceil() as u32).min(screen.0),
        (bottom.ceil() as u32).min(screen.1),
    );
    [
        left.min(right),
        top.min(bottom),
        right.saturating_sub(left),
        bottom.saturating_sub(top),
    ]
}

/// 読んだアイテム欄
pub struct Slots {
    /// 上の段の左から順に
    pub reads: Vec<Read>,
    /// 枠ごとの、アイテムが入っているか（段の印が出ているか）。読めない枠が、空なのか、クールダウンで
    /// 暗いだけなのかを見分ける
    pub occupied: Vec<bool>,
    /// ショップを開いている位置で読めた
    pub shop: bool,
}

/// アイテム欄を読む。ショップを開いているかは分からないので、両方の位置を読んで、読めた枠の多いほうを採る
/// （同じなら、開いていないほう）。`before` は前の回の答え。前と同じ位置で 1 つでも読めたら、もう一方の位置は読まない
/// （欄が動くと、元の位置では 1 つも読めなくなる）
pub fn read(frame: &Frame, templates: &[Template], scale: f32, before: Option<&Slots>) -> Slots {
    // 枠の左上の、画面での位置
    let corners = |shop: bool| {
        let rise = if shop { SHOP_RISE } else { 0.0 };
        (0..GRID.1)
            .flat_map(|row| (0..GRID.0).map(move |column| (row, column)))
            .map(move |(row, column)| {
                let left = (FIRST.0 + SPACING * column as f32) * scale;
                let top = (FIRST.1 - rise + SPACING * row as f32) * scale;
                (left.round() as i32, top.round() as i32)
            })
    };
    let at = |shop: bool| -> Vec<Read> {
        // 前の答えは、同じ位置で読んだものだけを使う
        let before = before.filter(|before| before.shop == shop);
        corners(shop)
            .enumerate()
            .map(|(slot, (left, top))| {
                let before = before.and_then(|before| before.reads.get(slot));
                super::read_again(frame, templates, left, top, before)
            })
            .collect()
    };
    let slots = |reads: Vec<Read>, shop: bool| {
        let size = (SLOT * scale).round() as u32;
        let occupied = corners(shop)
            .map(|(left, top)| occupied(frame, left, top, size))
            .collect();
        Slots {
            reads,
            occupied,
            shop,
        }
    };
    // 近さの合計で比べると、アイテムが少ないときに、空の枠の点の揺れで入れ替わる。読めた枠の数で比べる
    let known = |reads: &[Read]| {
        reads
            .iter()
            .filter(|read| read.answer(THRESHOLD).is_some())
            .count()
    };
    let first = before.is_some_and(|before| before.shop);
    let reads = at(first);
    if known(&reads) > 0 {
        return slots(reads, first);
    }
    let other = at(!first);
    let (usual, raised) = if first {
        (other, reads)
    } else {
        (reads, other)
    };
    let shop = known(&raised) > known(&usual);
    slots(if shop { raised } else { usual }, shop)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 画面から切り出す範囲は、通常の位置とショップの位置の全部の枠を、ずれを探す分まで含む
    #[test]
    fn the_area_covers_every_slot_in_both_positions() {
        for (screen, scale) in [((1920, 1080), 1.0), ((2560, 1440), 1440.0 / 1080.0)] {
            let [left, top, width, height] = area(screen, scale);
            let shift = 2.0;
            assert!(left as f32 <= FIRST.0 * scale - shift);
            assert!(top as f32 <= (FIRST.1 - SHOP_RISE) * scale - shift);
            let last = (
                (FIRST.0 + SPACING * 5.0 + SLOT) * scale,
                (FIRST.1 + SPACING + SLOT) * scale,
            );
            assert!((left + width) as f32 >= last.0 + shift);
            assert!((top + height) as f32 >= last.1 + shift);
            assert!(left + width <= screen.0 && top + height <= screen.1);
        }
    }
}
