//! ゲームの画面を読み続けて、買ったアイテムを知らせる（DESIGN.md 18.2）。
//! 読むのは、設定がオンで、見せる相手（OBS の中のオーバーレイか、読み取りの状態を出している操作パネル）がいる間だけ。

use std::sync::Arc;
use std::time::{Duration, Instant};

use super::capture::Capture;
use super::purchases::Tracker;
use super::samples::{self, Sample, Samples};
use super::{BASE_HEIGHT, Frame, Read, Template, heroes, items};
use crate::store::Store;
use crate::types::{Purchase, PurchasedItem, ScreenStatus};

/// 見本の画像を置く場所（データの場所の中）
const SAMPLES: &str = "screen_samples";
/// 読む間隔。1 秒に 3 回ほど
const INTERVAL: Duration = Duration::from_millis(330);
/// Deadlock のウィンドウがないときに、探し直す間隔
const SEARCH: Duration = Duration::from_secs(3);
/// 見本を取れなかったときに、取り直すまでの間
const RETRY: Duration = Duration::from_secs(30);

pub async fn run(store: Arc<Store>) {
    let mut settings = store.watch_settings();
    let mut watchers = store.screen_watchers();
    loop {
        if !wanted(&store) {
            store.set_screen_status(ScreenStatus::Off);
            tokio::select! {
                _ = settings.changed() => {}
                _ = watchers.changed() => {}
            }
            continue;
        }
        store.set_screen_status(ScreenStatus::Preparing);
        match samples::load(&store.data_dir().join(SAMPLES), None).await {
            Ok(samples) => {
                let store = store.clone();
                let reading = tokio::task::spawn_blocking(move || read(&store, &samples));
                if let Err(e) = reading.await {
                    tracing::error!("screen reading stopped: {e}");
                }
            }
            Err(e) => {
                tracing::warn!("cannot load screen samples: {e:#}");
                store.set_screen_status(ScreenStatus::Failed {
                    detail: format!("{e:#}"),
                });
                tokio::time::sleep(RETRY).await;
            }
        }
    }
}

fn wanted(store: &Store) -> bool {
    *store.screen_watchers().borrow() > 0 && store.settings().purchase_cards.enabled
}

/// 読み出した画像（上の端と、左下のアイテム欄）
type Images = [image::RgbImage; 2];

/// 画像から読んだもの
struct Reads {
    heroes: Vec<Read>,
    items: items::Slots,
}

/// 取り込みと、その画面の大きさに合わせた見本
struct Source {
    capture: Capture,
    scale: f32,
    heroes: Vec<Template>,
    items: Vec<Template>,
}

impl Source {
    fn open(samples: &Samples) -> anyhow::Result<Option<Self>> {
        let Some(capture) = Capture::open()? else {
            return Ok(None);
        };
        let scale = capture.size().1 as f32 / BASE_HEIGHT;
        // 読んだものは `class_name` で知らせる
        fn named(samples: &[Sample]) -> impl Iterator<Item = (&str, &image::RgbaImage)> {
            samples
                .iter()
                .map(|sample| (sample.class_name.as_str(), &sample.image))
        }
        Ok(Some(Self {
            heroes: heroes::templates(named(&samples.heroes), scale),
            items: items::templates(named(&samples.items), scale),
            capture,
            scale,
        }))
    }

    /// 新しい画面を読む。画面が変わっていなければ None
    fn read(&mut self) -> anyhow::Result<Option<Images>> {
        let size = self.capture.size();
        let areas = [
            heroes::area(size, self.scale),
            items::area(size, self.scale),
        ];
        let Some(images) = self.capture.read(&areas)? else {
            return Ok(None);
        };
        Ok(images.try_into().ok())
    }

    fn matched(&self, [top, slots]: &Images) -> Reads {
        let size = self.capture.size();
        let [left, upper, _, _] = items::area(size, self.scale);
        let heroes = heroes::read(
            &Frame {
                image: top,
                origin: (0, 0),
            },
            &self.heroes,
            size.0,
            self.scale,
        );
        let items = items::read(
            &Frame {
                image: slots,
                origin: (left as i32, upper as i32),
            },
            &self.items,
            self.scale,
        );
        Reads { heroes, items }
    }
}

/// 見せる相手がいる間、画面を読み続ける。新しいフレームを待つので、専用のスレッドで呼ぶ
fn read(store: &Store, samples: &Samples) {
    let started = Instant::now();
    let mut tracker = Tracker::default();
    let mut source: Option<Source> = None;
    // 前に読んだ画像と、その答え。画面が同じなら、照合を飛ばす（アイテム欄は、買ったとき以外ほとんど変わらない）
    let mut last: Option<(Images, Reads)> = None;
    while wanted(store) {
        let Some(open) = &mut source else {
            match Source::open(samples) {
                Ok(Some(opened)) => source = Some(opened),
                Ok(None) => {
                    store.set_screen_status(ScreenStatus::NoWindow);
                    std::thread::sleep(SEARCH);
                }
                Err(e) => {
                    tracing::warn!("cannot capture the game window: {e:#}");
                    store.set_screen_status(ScreenStatus::Failed {
                        detail: format!("{e:#}"),
                    });
                    std::thread::sleep(SEARCH);
                }
            }
            continue;
        };
        match open.read() {
            Ok(Some(images)) => {
                let reads = match last.take() {
                    Some((before, reads)) if before == images => reads,
                    _ => open.matched(&images),
                };
                let (width, height) = open.capture.size();
                store.set_screen_status(ScreenStatus::Reading {
                    width,
                    height,
                    heroes: answers(&reads.heroes, heroes::THRESHOLD).collect(),
                    items: answers(&reads.items.reads, items::THRESHOLD)
                        .flatten()
                        .collect(),
                    shop: reads.items.shop,
                });
                let bought = tracker.observe(started.elapsed(), &reads.heroes, &reads.items);
                if !bought.is_empty() {
                    // 自分のヒーローと試合の時計は、まだ読めない（DESIGN.md 18.6）
                    store.purchased(Purchase {
                        hero: None,
                        clock_seconds: None,
                        items: bought
                            .into_iter()
                            .map(|item| PurchasedItem { item, rate: None })
                            .collect(),
                    });
                }
                last = Some((images, reads));
            }
            // 画面が変わっていないか、最小化している
            Ok(None) => {}
            // ウィンドウが閉じたか、大きさが変わった。開き直す
            Err(e) => {
                tracing::info!("reopening the capture: {e:#}");
                source = None;
            }
        }
        std::thread::sleep(INTERVAL);
    }
}

fn answers(reads: &[Read], threshold: f32) -> impl Iterator<Item = Option<String>> {
    reads
        .iter()
        .map(move |read| read.answer(threshold).map(str::to_owned))
}
