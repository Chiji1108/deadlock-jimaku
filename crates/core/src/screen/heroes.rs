//! 上部バーの枠から、試合に出ているヒーローを読む。
//! 枠は、deadlock-api のカード画像（`icon_hero_card`）を縮めて、上の端に付けて置いたもの。

use image::RgbaImage;

use super::{Frame, Read, Template};

/// これより近くなければ「不明」にする（死亡中の暗い枠、誰もいない枠、ふだんと違う絵、見た目を変える MOD）
pub const THRESHOLD: f32 = 0.8;
/// 枠に出るカードの大きさ（1080p）。上の端は画面の上の端に付く
const CARD: (f32, f32) = (70.0, 95.0);
/// 隣の枠の中心までの幅
const SPACING: f32 = 88.0;
/// 画面の中央から、いちばん内側の枠の中心までの幅
const INNER: f32 = 146.0;
/// チームの枠の数。人数の少ないモード（ストリートブロール）は、内側から詰める
const TEAM: usize = 6;
/// 死んでほかの人を観戦している間は、画面の上の端にチームの色の帯が出て、バーがこれだけ下がる
const SPECTATING_DROP: f32 = 8.0;

/// カードのうち、比べる範囲。左右はチームの印（「!」）が、下は丸い縁とソウルの数が重なる
fn compared(x: f32, y: f32) -> bool {
    (0.25..0.75).contains(&x) && y < 0.8
}

/// カード画像から、画面の大きさ（`scale` は画面の高さ ÷ 1080）に合わせた見本を作る
pub fn templates<'a>(
    cards: impl IntoIterator<Item = (&'a str, &'a RgbaImage)>,
    scale: f32,
) -> Vec<Template> {
    let size = (
        (CARD.0 * scale).round() as u32,
        (CARD.1 * scale).round() as u32,
    );
    cards
        .into_iter()
        .map(|(name, card)| Template::new(name, card, size, compared))
        .collect()
}

/// 読むのに要る画面の範囲（左、上、幅、高さ）。上の端の帯
pub fn area(screen: (u32, u32), scale: f32) -> [u32; 4] {
    let height = ((CARD.1 + SPECTATING_DROP + super::MARGIN) * scale).ceil() as u32;
    [0, 0, screen.0, height.min(screen.1)]
}

/// 読んだ上部バー
pub struct Bar {
    /// 左から順に（左のチームの 6 枠、右のチームの 6 枠）
    pub reads: Vec<Read>,
    /// 下がった位置で読めた。死んで、ほかの人を観戦している
    pub spectating: bool,
}

/// 上部バーを読む。観戦しているかは分からないので、両方の位置を読んで、読めた枠の多いほうを採る
/// （同じなら、観戦していないほう）
pub fn read(frame: &Frame, templates: &[Template], screen_width: u32, scale: f32) -> Bar {
    let center = screen_width as f32 / 2.0;
    let at = |drop: f32| -> Vec<Read> {
        let offsets = (0..TEAM).map(|i| (INNER + SPACING * i as f32) * scale);
        let left = offsets.clone().rev().map(|offset| center - offset);
        left.chain(offsets.map(|offset| center + offset))
            .map(|center| {
                let left = (center - CARD.0 * scale / 2.0).round() as i32;
                super::read(frame, templates, left, (drop * scale).round() as i32)
            })
            .collect()
    };
    let known = |reads: &[Read]| {
        reads
            .iter()
            .filter(|read| read.answer(THRESHOLD).is_some())
            .count()
    };
    let (usual, dropped) = (at(0.0), at(SPECTATING_DROP));
    let spectating = known(&dropped) > known(&usual);
    Bar {
        reads: if spectating { dropped } else { usual },
        spectating,
    }
}
