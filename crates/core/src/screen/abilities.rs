//! 画面の下の中央に並ぶアビリティの絵から、自分のヒーローを当てる（DESIGN.md 18.4）。
//! 上部バーの自分の枠は、背景の形が状態で変わり、後ろの景色とも見分けにくいので、当てにしない。
//! アビリティの絵は、色が状態で変わる（取れる、取った、使っている最中）ので、明るさの形だけを比べ、
//! 白黒が逆でも同じとみなす。大きさと位置も動くので、探す（粗く全部と比べてから、近いものだけ細かく）。

use image::imageops::FilterType;
use image::{GrayImage, RgbImage, RgbaImage};

use super::Frame;

/// これより近い絵だけを数える
pub const THRESHOLD: f32 = 0.8;
/// 同じヒーローの絵が、いくつ合えば決めるか（使っている最中やクールダウン中の絵は読めないことがある）
const AGREEING: usize = 2;
/// 絵の中心（1080p。横は画面の中央から）。取れる状態の絵は、少し上に浮く
const CENTERS: [f32; 4] = [-134.0, -45.0, 45.0, 134.0];
const CENTER_Y: f32 = 1020.0;
/// 絵の大きさの範囲（1080p の画素）と、探す位置のずれ
const SIZES: std::ops::RangeInclusive<u32> = 40..=61;
const SHIFT_X: i32 = 6;
const SHIFT_UP: i32 = 22;
const SHIFT_DOWN: i32 = 8;
/// 粗く比べるときに、縮める割合
const COARSE: u32 = 3;
/// 細かく比べ直す候補の数
const REFINED: usize = 5;

/// アビリティの絵 1 つ（明るさ × 不透明さ。絵の外は 0）
pub struct Shape {
    hero: String,
    image: GrayImage,
    /// 粗く比べるための、縮めた絵（大きさごと）
    coarse: Vec<Patch>,
}

/// 平均を引いて、長さを 1 にした正方形の絵
struct Patch {
    size: u32,
    values: Vec<f32>,
}

impl Patch {
    fn new(image: &GrayImage, size: u32) -> Self {
        let resized = image::imageops::resize(image, size, size, FilterType::Triangle);
        let mut values: Vec<f32> = resized.as_raw().iter().map(|v| f32::from(*v)).collect();
        super::normalize(&mut values);
        Self { size, values }
    }

    /// 左上を `(left, top)` に置いたときの近さ。白黒が逆でも同じにする。はみ出すなら None
    fn score(&self, screen: &Gray, left: i32, top: i32) -> Option<f32> {
        let size = self.size as i32;
        if left < 0 || top < 0 || left + size > screen.width || top + size > screen.height {
            return None;
        }
        let (mut sum, mut squares, mut both) = (0.0f32, 0.0f32, 0.0f32);
        for (row, ours) in self.values.chunks(self.size as usize).enumerate() {
            let start = ((top + row as i32) * screen.width + left) as usize;
            for (a, b) in ours.iter().zip(&screen.values[start..start + ours.len()]) {
                sum += b;
                squares += b * b;
                both += a * b;
            }
        }
        // 見本は平均が 0 なので、画面の平均を引かなくても積の和は変わらない
        let spread = squares - sum * sum / self.values.len() as f32;
        (spread >= self.values.len() as f32).then(|| (both / spread.sqrt()).abs())
    }
}

/// 画面の明るさ
struct Gray {
    width: i32,
    height: i32,
    values: Vec<f32>,
}

impl Gray {
    fn new(image: &RgbImage) -> Self {
        Self {
            width: image.width() as i32,
            height: image.height() as i32,
            values: image
                .pixels()
                .map(|pixel| pixel.0.iter().map(|v| f32::from(*v)).sum::<f32>() / 3.0)
                .collect(),
        }
    }

    /// `factor` 分の 1 に縮める（平均）
    fn shrunk(&self, factor: i32) -> Self {
        let (width, height) = (self.width / factor, self.height / factor);
        let mut values = Vec::with_capacity((width * height) as usize);
        for y in 0..height {
            for x in 0..width {
                let mut sum = 0.0;
                for dy in 0..factor {
                    let start = ((y * factor + dy) * self.width + x * factor) as usize;
                    sum += self.values[start..start + factor as usize]
                        .iter()
                        .sum::<f32>();
                }
                values.push(sum / (factor * factor) as f32);
            }
        }
        Self {
            width,
            height,
            values,
        }
    }
}

fn sizes(scale: f32) -> impl Iterator<Item = u32> {
    SIZES
        .step_by(COARSE as usize)
        .map(move |size| (size as f32 * scale).round() as u32)
}

/// アビリティの絵（持ち主のヒーローの名前と、絵）から、画面の大きさに合わせた見本を作る
pub fn shapes<'a>(
    images: impl IntoIterator<Item = (&'a str, &'a RgbaImage)>,
    scale: f32,
) -> Vec<Shape> {
    images
        .into_iter()
        .map(|(hero, image)| {
            let image = GrayImage::from_fn(image.width(), image.height(), |x, y| {
                let [r, g, b, a] = image.get_pixel(x, y).0;
                let light = (u32::from(r) + u32::from(g) + u32::from(b)) / 3;
                image::Luma([(light * u32::from(a) / 255) as u8])
            });
            let coarse = sizes(scale)
                .map(|size| Patch::new(&image, (size / COARSE).max(2)))
                .collect();
            Shape {
                hero: hero.to_owned(),
                image,
                coarse,
            }
        })
        .collect()
}

/// 読むのに要る画面の範囲（左、上、幅、高さ）
pub fn area(screen: (u32, u32), scale: f32) -> [u32; 4] {
    let half_size = *SIZES.end() as f32 / 2.0;
    let half = (CENTERS[3] + half_size + SHIFT_X as f32 + super::MARGIN) * scale;
    let center = screen.0 as f32 / 2.0;
    let left = (center - half).floor().max(0.0) as u32;
    let right = ((center + half).ceil() as u32).min(screen.0);
    let top = (CENTER_Y - half_size - SHIFT_UP as f32 - super::MARGIN) * scale;
    let bottom = (CENTER_Y + half_size + SHIFT_DOWN as f32 + super::MARGIN) * scale;
    let (top, bottom) = (
        top.floor().max(0.0) as u32,
        (bottom.ceil() as u32).min(screen.1),
    );
    [
        left,
        top,
        right.saturating_sub(left),
        bottom.saturating_sub(top),
    ]
}

/// 4 つの絵それぞれに、いちばん近いヒーローと、その近さ
pub fn read(frame: &Frame, shapes: &[Shape], screen_width: u32, scale: f32) -> Vec<(String, f32)> {
    let fine = Gray::new(frame.image);
    let coarse = fine.shrunk(COARSE as i32);
    let reach = |pixels: i32| (pixels as f32 * scale).round() as i32;
    CENTERS
        .iter()
        .map(|offset| {
            // 切り出した画像の中での、絵の中心
            let center = (
                screen_width as f32 / 2.0 + offset * scale - frame.origin.0 as f32,
                CENTER_Y * scale - frame.origin.1 as f32,
            );
            // 粗く: 全部の見本、全部の大きさと位置
            let mut candidates: Vec<(f32, &Shape, u32, (i32, i32))> = Vec::new();
            for shape in shapes {
                let mut best = (0.0f32, 0, (0, 0));
                for (patch, size) in shape.coarse.iter().zip(sizes(scale)) {
                    let step = COARSE as i32;
                    for dy in (-reach(SHIFT_UP)..=reach(SHIFT_DOWN)).step_by(COARSE as usize) {
                        for dx in (-reach(SHIFT_X)..=reach(SHIFT_X)).step_by(COARSE as usize) {
                            let left = (center.0 - size as f32 / 2.0).round() as i32 + dx;
                            let top = (center.1 - size as f32 / 2.0).round() as i32 + dy;
                            if let Some(score) = patch.score(&coarse, left / step, top / step)
                                && score > best.0
                            {
                                best = (score, size, (left, top));
                            }
                        }
                    }
                }
                candidates.push((best.0, shape, best.1, best.2));
            }
            candidates.sort_by(|a, b| b.0.total_cmp(&a.0));
            // 細かく: 近かった見本だけ、まわりの大きさと位置
            let mut best = (String::new(), 0.0f32);
            for (_, shape, size, (left, top)) in candidates.into_iter().take(REFINED) {
                if size == 0 {
                    continue;
                }
                for size in size.saturating_sub(2)..=size + 2 {
                    let patch = Patch::new(&shape.image, size);
                    let step = COARSE as i32;
                    for dy in -step..=step {
                        for dx in -step..=step {
                            if let Some(score) = patch.score(&fine, left + dx, top + dy)
                                && score > best.1
                            {
                                best = (shape.hero.clone(), score);
                            }
                        }
                    }
                }
            }
            best
        })
        .collect()
}

/// 4 つの絵の答えから、自分のヒーローを決める。同じヒーローの絵が `AGREEING` 個以上よく合ったら、そのヒーロー
pub fn hero(reads: &[(String, f32)]) -> Option<&str> {
    let agreed = |name: &str| {
        reads
            .iter()
            .filter(|(hero, score)| hero == name && *score >= THRESHOLD)
            .count()
    };
    reads
        .iter()
        .map(|(name, _)| name.as_str())
        .find(|name| agreed(name) >= AGREEING)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(name: &str, score: f32) -> (String, f32) {
        (name.to_owned(), score)
    }

    #[test]
    fn two_pictures_of_one_hero_decide_it() {
        // 使っている最中の絵と、数字が重なった絵は読めない
        let reads = [
            read("hero_solomon", 0.72),
            read("hero_solomon", 0.95),
            read("hero_pocket", 0.52),
            read("hero_solomon", 0.91),
        ];
        assert_eq!(hero(&reads), Some("hero_solomon"));
        // 試合が始まる前の、鍵が掛かった絵
        let locked = [
            read("hero_paradox", 0.52),
            read("hero_apollo", 0.59),
            read("hero_haze", 0.89),
            read("hero_danny", 0.55),
        ];
        assert_eq!(hero(&locked), None);
    }
}
