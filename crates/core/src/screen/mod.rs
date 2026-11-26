//! ゲームの画面の決まった場所の絵を、見本と比べて当てる（DESIGN.md 15 章「画面からの読み取り」、18 章）。
//! 枠に出る絵は、deadlock-api の画像を縮めて決まった位置に置いたものなので、枠ごとに全部の見本と比べて
//! （色の正規化相互相関）、いちばん近いものを答えにする。画面の取り込みも通信もしない。

pub mod heroes;
pub mod items;
pub mod purchases;

use image::imageops::FilterType;
use image::{RgbImage, RgbaImage};

/// 配置は 1080p で測った。ほかの解像度では、画面の高さに比例させる（16:9 の 1080p でしか確かめていない）
pub const BASE_HEIGHT: f32 = 1080.0;
/// 置かれる位置のずれを、この画素数まで探す
const SHIFT: i32 = 2;

/// 画面の一部。読むのは上の端と左下だけなので、そこだけを切り出して渡せる
pub struct Frame<'a> {
    pub image: &'a RgbImage,
    /// 切り出した左上の、画面での位置
    pub origin: (i32, i32),
}

/// 縮めた見本のうち、比べる画素（見本の左上からの位置）と、その色（平均を引いて長さ 1 にしたもの）
pub struct Template {
    name: String,
    pixels: Vec<(u32, u32)>,
    colors: Vec<f32>,
}

impl Template {
    /// `image` を `size` に縮める。`compared` は、比べる範囲（幅と高さに対する割合で、画素の中心を渡す）。
    /// ほかの表示が重なるところを外す
    pub fn new(
        name: &str,
        image: &RgbaImage,
        size: (u32, u32),
        compared: impl Fn(f32, f32) -> bool,
    ) -> Self {
        let image = image::imageops::resize(image, size.0, size.1, FilterType::Lanczos3);
        let mut pixels = Vec::new();
        let mut colors = Vec::new();
        for (x, y, pixel) in image.enumerate_pixels() {
            let [r, g, b, a] = pixel.0;
            let at = (
                (x as f32 + 0.5) / size.0 as f32,
                (y as f32 + 0.5) / size.1 as f32,
            );
            // 見本の外（透明）は、画面では背景が透ける
            if a > 200 && compared(at.0, at.1) {
                pixels.push((x, y));
                colors.extend([r as f32, g as f32, b as f32]);
            }
        }
        normalize(&mut colors);
        Self {
            name: name.to_owned(),
            pixels,
            colors,
        }
    }

    /// 見本の左上を画面の `(left, top)` に置いたときの近さ（-1〜1）。`frame` からはみ出すなら None
    fn score(&self, frame: &Frame, left: i32, top: i32, buffer: &mut Vec<f32>) -> Option<f32> {
        let (left, top) = (left - frame.origin.0, top - frame.origin.1);
        let (width, height) = (frame.image.width() as i32, frame.image.height() as i32);
        buffer.clear();
        for &(x, y) in &self.pixels {
            let (x, y) = (left + x as i32, top + y as i32);
            if x < 0 || y < 0 || x >= width || y >= height {
                return None;
            }
            buffer.extend(frame.image.get_pixel(x as u32, y as u32).0.map(f32::from));
        }
        normalize(buffer);
        Some(
            self.colors
                .iter()
                .zip(buffer.iter())
                .map(|(a, b)| a * b)
                .sum(),
        )
    }
}

/// 平均を引いて、長さを 1 にする
fn normalize(values: &mut [f32]) {
    let mean = values.iter().sum::<f32>() / values.len().max(1) as f32;
    values.iter_mut().for_each(|v| *v -= mean);
    let norm = values.iter().map(|v| v * v).sum::<f32>().sqrt().max(1e-6);
    values.iter_mut().for_each(|v| *v /= norm);
}

/// 枠にいちばん近い見本
#[derive(Clone, Debug, PartialEq)]
pub struct Read {
    pub name: String,
    pub score: f32,
    /// その次に近い見本の近さ
    pub second: f32,
}

impl Read {
    /// `threshold` より近ければ、その見本の名前。そうでなければ「不明」
    pub fn answer(&self, threshold: f32) -> Option<&str> {
        (self.score >= threshold).then_some(self.name.as_str())
    }
}

/// 見本の左上が画面の `(left, top)` に来る枠を読む
pub fn read(frame: &Frame, templates: &[Template], left: i32, top: i32) -> Read {
    let mut buffer = Vec::new();
    let mut scores: Vec<(f32, &str)> = templates
        .iter()
        .map(|template| {
            let mut best = -1.0f32;
            for dy in -SHIFT..=SHIFT {
                for dx in -SHIFT..=SHIFT {
                    if let Some(score) = template.score(frame, left + dx, top + dy, &mut buffer) {
                        best = best.max(score);
                    }
                }
            }
            (best, template.name.as_str())
        })
        .collect();
    scores.sort_by(|a, b| b.0.total_cmp(&a.0));
    let (score, name) = scores.first().copied().unwrap_or((-1.0, ""));
    Read {
        name: name.to_owned(),
        score,
        second: scores.get(1).map_or(-1.0, |(score, _)| *score),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{Rgb, Rgba};

    /// 見本ごとに違う模様の絵
    fn picture(seed: u32) -> RgbaImage {
        RgbaImage::from_fn(40, 40, |x, y| {
            let v = (x * (3 + seed) + y * (7 + seed * 5) + x * y * seed) % 256;
            Rgba([v as u8, (v * 3 % 256) as u8, (255 - v) as u8, 255])
        })
    }

    fn templates() -> Vec<Template> {
        (1..=3)
            .map(|seed| Template::new(&format!("p{seed}"), &picture(seed), (40, 40), |_, _| true))
            .collect()
    }

    /// 灰色の画面の `at` に、絵を置く
    fn screen(picture: Option<(&RgbaImage, (u32, u32))>) -> RgbImage {
        let mut screen = RgbImage::from_pixel(120, 100, Rgb([90, 90, 90]));
        if let Some((picture, at)) = picture {
            for (x, y, pixel) in picture.enumerate_pixels() {
                let [r, g, b, _] = pixel.0;
                screen.put_pixel(at.0 + x, at.1 + y, Rgb([r, g, b]));
            }
        }
        screen
    }

    #[test]
    fn finds_the_picture_even_when_it_is_a_little_off() {
        let image = screen(Some((&picture(2), (31, 22))));
        let frame = Frame {
            image: &image,
            origin: (0, 0),
        };
        let read = read(&frame, &templates(), 30, 20);
        assert_eq!(read.answer(0.8), Some("p2"));
        assert!(read.score > 0.99, "{read:?}");
        assert!(read.second < 0.8, "{read:?}");
    }

    #[test]
    fn positions_are_on_the_screen_not_on_the_cut_out_part() {
        let whole = screen(Some((&picture(1), (30, 20))));
        let part = image::imageops::crop_imm(&whole, 20, 10, 100, 90).to_image();
        let frame = Frame {
            image: &part,
            origin: (20, 10),
        };
        assert_eq!(read(&frame, &templates(), 30, 20).answer(0.8), Some("p1"));
    }

    #[test]
    fn a_plain_place_and_a_place_off_the_frame_are_unknown() {
        let image = screen(None);
        let frame = Frame {
            image: &image,
            origin: (0, 0),
        };
        assert_eq!(read(&frame, &templates(), 30, 20).answer(0.8), None);
        assert_eq!(read(&frame, &templates(), 110, 90).answer(0.8), None);
        assert_eq!(read(&frame, &[], 30, 20).answer(0.8), None);
    }
}
