//! ゲームの画面を読み続けて、買ったアイテムを知らせる（DESIGN.md 18.2）。
//! 読むのは、設定がオンで、見せる相手（OBS の中のオーバーレイか、読み取りの状態を出している操作パネル）がいる間だけ。

use std::sync::Arc;
use std::time::{Duration, Instant};

use super::capture::Capture;
use super::purchases::Tracker;
use super::samples::{self, Sample, Samples};
use super::{BASE_HEIGHT, Frame, Read, Template, clock, heroes, items};
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

/// 読み出した画像（上の端、左下のアイテム欄、時計）
type Images = [image::RgbImage; 3];
/// 時計が読めなくなってから（ショップを開いている間など）、これより経ったら、買った時刻は分からないとする
const CLOCK_KEPT: Duration = Duration::from_secs(120);

/// 画像から読んだもの
struct Reads {
    heroes: heroes::Bar,
    items: items::Slots,
    /// 試合の時計（試合が始まってからの秒）
    clock: Option<u32>,
}

/// 取り込みと、その画面の大きさに合わせた見本
struct Source {
    capture: Capture,
    scale: f32,
    heroes: Vec<Template>,
    items: Vec<Template>,
    digits: Vec<Template>,
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
            digits: clock::templates(scale),
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
            clock::area(size, self.scale),
        ];
        let Some(images) = self.capture.read(&areas)? else {
            return Ok(None);
        };
        Ok(images.try_into().ok())
    }

    /// 画像を読む。`before` は、前に読んだ画像とその答え。同じ画像のところは、照合を飛ばして前の答えを使う
    /// （上の端は後ろの景色が透けて毎回変わるが、アイテム欄は、買ったとき以外ほとんど変わらない）
    fn matched(&self, images: &Images, before: Option<(Images, Reads)>) -> Reads {
        let [top, slots, time] = images;
        let size = self.capture.size();
        let (mut heroes, mut items, mut clock) = (None, None, None);
        if let Some(([top_before, slots_before, time_before], reads)) = before {
            heroes = (top_before == *top).then_some(reads.heroes);
            items = (slots_before == *slots).then_some(reads.items);
            clock = (time_before == *time).then_some(reads.clock);
        }
        let heroes = heroes.unwrap_or_else(|| {
            let frame = Frame {
                image: top,
                origin: (0, 0),
            };
            heroes::read(&frame, &self.heroes, size.0, self.scale)
        });
        let items = items.unwrap_or_else(|| {
            let [left, upper, _, _] = items::area(size, self.scale);
            let frame = Frame {
                image: slots,
                origin: (left as i32, upper as i32),
            };
            items::read(&frame, &self.items, self.scale)
        });
        // 時計の位置は、観戦中かどうかで変わる。時計の画像が同じなら、観戦中かどうかも前と同じ
        let clock = clock.unwrap_or_else(|| {
            let [left, upper, _, _] = clock::area(size, self.scale);
            let frame = Frame {
                image: time,
                origin: (left as i32, upper as i32),
            };
            clock::read(&frame, &self.digits, size.0, self.scale, heroes.spectating)
        });
        Reads {
            heroes,
            items,
            clock,
        }
    }
}

/// 見せる相手がいる間、画面を読み続ける。新しいフレームを待つので、専用のスレッドで呼ぶ
fn read(store: &Store, samples: &Samples) {
    let started = Instant::now();
    let mut tracker = Tracker::default();
    let mut source: Option<Source> = None;
    // 前に読んだ画像と、その答え
    let mut last: Option<(Images, Reads)> = None;
    // 最後に読めた時計と、そのとき。ショップを開いている間は時計が隠れるので、そこから数える
    let mut clock: Option<(u32, Instant)> = None;
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
                let reads = open.matched(&images, last.take());
                let (width, height) = open.capture.size();
                store.set_screen_status(ScreenStatus::Reading {
                    width,
                    height,
                    heroes: answers(&reads.heroes.reads, heroes::THRESHOLD).collect(),
                    items: answers(&reads.items.reads, items::THRESHOLD)
                        .flatten()
                        .collect(),
                    shop: reads.items.shop,
                });
                if let Some(seconds) = reads.clock {
                    clock = Some((seconds, Instant::now()));
                }
                let bought = tracker.observe(started.elapsed(), &reads.heroes, &reads.items);
                if !bought.is_empty() {
                    // 自分のヒーローは、まだ読めない（DESIGN.md 18.6）
                    store.purchased(Purchase {
                        hero: None,
                        clock_seconds: clock
                            .filter(|(_, at)| at.elapsed() < CLOCK_KEPT)
                            .map(|(seconds, at)| seconds as f32 + at.elapsed().as_secs_f32()),
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
