//! 画面の上の中央の、試合の時計を読む（DESIGN.md 18.4）。買った時刻が、平均より早いか遅いかを出すのに使う。
//! 数字は、決まった位置の枠に 1 つずつ入るので、枠ごとに数字の型と比べる。
//! 数字の絵は deadlock-api にないので、試合の画面から作った型（白黒の小さい形）を持っておく。

use image::{Rgba, RgbaImage};

use super::{Frame, Read, Template};

/// これより近くなければ、時計は読めていない（ショップなどに隠れている）
pub const THRESHOLD: f32 = 0.8;
/// 型の大きさ（1080p での画素）
const GLYPH: (u32, u32) = (9, 11);
/// 枠の上の端（1080p）
const TOP: f32 = 18.0;
/// 枠の左の端（1080p）。分が 2 桁のときと 1 桁のときで、全体が中央に寄るので位置が違う。None はコロン
const TWO_DIGITS: [Option<f32>; 5] = [Some(940.0), Some(949.0), None, Some(962.0), Some(971.0)];
const ONE_DIGIT: [Option<f32>; 4] = [Some(945.0), None, Some(958.0), Some(967.0)];
/// 1080p の画面の幅。枠の位置は、画面の中央からの距離で決まる
const BASE_WIDTH: f32 = 1920.0;

/// 数字の型。`#` が文字
const GLYPHS: [(char, [&str; 11]); 10] = [
    (
        '0',
        [
            ".........",
            "...###...",
            ".######..",
            ".###.###.",
            "###...##.",
            "###...###",
            "###...###",
            ".##...##.",
            ".#######.",
            "..#####..",
            ".........",
        ],
    ),
    (
        '1',
        [
            ".........",
            ".....##..",
            "...####..",
            "..#####..",
            "..#####..",
            "....###..",
            "....###..",
            "....###..",
            "..#######",
            "..#######",
            ".........",
        ],
    ),
    (
        '2',
        [
            ".........",
            "...####..",
            ".#######.",
            ".##..###.",
            ".#...###.",
            "....####.",
            "...####..",
            ".####....",
            "########.",
            "########.",
            ".........",
        ],
    ),
    (
        '3',
        [
            ".........",
            "...####..",
            "..######.",
            ".##..###.",
            ".....###.",
            "...####..",
            "...#####.",
            "###...###",
            ".#######.",
            "..######.",
            ".........",
        ],
    ),
    (
        '4',
        [
            ".........",
            "....###..",
            "....####.",
            "...#####.",
            "..##.###.",
            ".###.###.",
            "#########",
            "#########",
            ".....###.",
            ".....###.",
            ".........",
        ],
    ),
    (
        '5',
        [
            ".........",
            ".#######.",
            ".#######.",
            ".######..",
            ".##.##...",
            ".#######.",
            ".##..###.",
            ".##...##.",
            ".#######.",
            "..#####..",
            ".........",
        ],
    ),
    (
        '6',
        [
            ".........",
            "...####..",
            "..######.",
            ".###..##.",
            "###..##..",
            "########.",
            "####..###",
            ".##....##",
            ".########",
            "..######.",
            ".........",
        ],
    ),
    (
        '7',
        [
            ".........",
            ".#######.",
            "########.",
            ".#######.",
            ".....##..",
            "..#####..",
            ".######..",
            "...###...",
            "...##....",
            "..###....",
            ".........",
        ],
    ),
    (
        '8',
        [
            ".........",
            "..#####..",
            ".#######.",
            ".##...##.",
            ".#######.",
            ".######..",
            ".#######.",
            "###...###",
            "########.",
            ".#######.",
            ".........",
        ],
    ),
    (
        '9',
        [
            ".........",
            "..#####..",
            ".#######.",
            "###...###",
            "###...###",
            ".########",
            "..#######",
            ".##...###",
            ".#######.",
            "..#####..",
            ".........",
        ],
    ),
];

/// 画面の大きさ（`scale` は画面の高さ ÷ 1080）に合わせた、数字の見本
pub fn templates(scale: f32) -> Vec<Template> {
    let size = (
        (GLYPH.0 as f32 * scale).round() as u32,
        (GLYPH.1 as f32 * scale).round() as u32,
    );
    GLYPHS
        .iter()
        .map(|(digit, rows)| {
            let mut image = RgbaImage::from_pixel(GLYPH.0, GLYPH.1, Rgba([0, 0, 0, 255]));
            for (y, row) in rows.iter().enumerate() {
                for (x, cell) in row.bytes().enumerate() {
                    if cell == b'#' {
                        image.put_pixel(x as u32, y as u32, Rgba([255, 255, 255, 255]));
                    }
                }
            }
            Template::new(&digit.to_string(), &image, size, |_, _| true)
        })
        .collect()
}

/// 読むのに要る画面の範囲（左、上、幅、高さ）。観戦中にバーが下がる分まで入れる
pub fn area(screen: (u32, u32), scale: f32) -> [u32; 4] {
    let center = screen.0 as f32 / 2.0;
    let half = (BASE_WIDTH / 2.0 - TWO_DIGITS[0].unwrap_or(0.0) + super::MARGIN) * scale;
    let left = (center - half).floor().max(0.0) as u32;
    let right = ((center + half).ceil() as u32).min(screen.0);
    let top = ((TOP - super::MARGIN) * scale).floor().max(0.0) as u32;
    let bottom = (TOP + GLYPH.1 as f32 + super::heroes::SPECTATING_DROP + super::MARGIN) * scale;
    let bottom = (bottom.ceil() as u32).min(screen.1);
    [
        left,
        top,
        right.saturating_sub(left),
        bottom.saturating_sub(top),
    ]
}

/// 時計を読んで、試合が始まってからの秒を返す。読めなければ None。
/// `spectating` は、観戦中でバーが下がっている（`heroes::Bar::spectating`）
pub fn read(
    frame: &Frame,
    templates: &[Template],
    screen_width: u32,
    scale: f32,
    spectating: bool,
) -> Option<u32> {
    let drop = if spectating {
        super::heroes::SPECTATING_DROP
    } else {
        0.0
    };
    let top = ((TOP + drop) * scale).round() as i32;
    let center = screen_width as f32 / 2.0;
    let digits = |cells: &[Option<f32>]| -> Vec<Read> {
        cells
            .iter()
            .flatten()
            .map(|left| {
                let left = center + (left - BASE_WIDTH / 2.0) * scale;
                // 数字は 10 個しかなく、形が似ているので、全部の型でずれを探す
                super::read_refining(frame, templates, left.round() as i32, top, templates.len())
            })
            .collect()
    };
    // 分が 2 桁か 1 桁かは分からないので、両方の並びで読んで、近いほうを採る
    let mean =
        |reads: &[Read]| reads.iter().map(|read| read.score).sum::<f32>() / reads.len() as f32;
    let (two, one) = (digits(&TWO_DIGITS), digits(&ONE_DIGIT));
    let reads = if mean(&two) >= mean(&one) { two } else { one };
    let mut values = Vec::new();
    for read in &reads {
        values.push(read.answer(THRESHOLD)?.parse::<u32>().ok()?);
    }
    let split = values.len() - 2;
    let minutes = values[..split]
        .iter()
        .fold(0, |sum, digit| sum * 10 + digit);
    let seconds = values[split] * 10 + values[split + 1];
    (seconds < 60).then_some(minutes * 60 + seconds)
}

#[cfg(test)]
mod tests {
    use image::{Rgb, RgbImage};

    use super::*;

    /// 型を並べて時計の絵を作る。`cells` は枠の位置、`text` は数字だけ
    fn drawn(cells: &[Option<f32>], text: &str, drop: u32) -> RgbImage {
        let mut image = RgbImage::from_pixel(1920, 60, Rgb([70, 50, 40]));
        for (left, digit) in cells.iter().flatten().zip(text.chars()) {
            let (_, rows) = GLYPHS.iter().find(|(glyph, _)| *glyph == digit).unwrap();
            for (y, row) in rows.iter().enumerate() {
                for (x, cell) in row.bytes().enumerate() {
                    if cell == b'#' {
                        let (x, y) = (*left as u32 + x as u32, TOP as u32 + drop + y as u32);
                        image.put_pixel(x, y, Rgb([200, 190, 170]));
                    }
                }
            }
        }
        image
    }

    fn seconds(image: &RgbImage, spectating: bool) -> Option<u32> {
        let frame = Frame {
            image,
            origin: (0, 0),
        };
        read(&frame, &templates(1.0), 1920, 1.0, spectating)
    }

    #[test]
    fn reads_minutes_of_one_and_two_digits() {
        assert_eq!(seconds(&drawn(&ONE_DIGIT, "907", 0), false), Some(547));
        assert_eq!(seconds(&drawn(&TWO_DIGITS, "1234", 0), false), Some(754));
        assert_eq!(seconds(&drawn(&TWO_DIGITS, "5680", 0), false), None);
    }

    #[test]
    fn the_clock_drops_with_the_bar_while_spectating_and_is_unknown_when_hidden() {
        let dropped = drawn(&TWO_DIGITS, "2258", 8);
        assert_eq!(seconds(&dropped, true), Some(1378));
        assert_eq!(seconds(&dropped, false), None);
        assert_eq!(seconds(&drawn(&TWO_DIGITS, "", 0), false), None);
    }
}
