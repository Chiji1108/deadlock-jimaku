//! Deadlock のウィンドウを取り込み続けたときの負荷を測る（DESIGN.md 18.6 節）。
//! 決まった秒数だけ取り込み、読み出しに掛かった時間と、このプロセスが使った CPU の時間を出す。
//! ゲームの FPS は外から測れないので、終わりに画面の右上（ゲームが出す FPS の表示）を PNG に書く。
//! ゲームと OBS の CPU と GPU は、`experiments/capture_load.ps1` がこの例を動かしながら測る。
//!
//! usage: cargo run -p overlays-core --release --example capture_load -- <測り方> [秒数] [出力のフォルダ] [1 秒の回数]
//!   測り方: none（取り込まない）、idle（取り込むが読み出さない）、full（画面全体を読み出す）、
//!           region（上部バーとアイテム欄のあたりだけ読み出す）

#[cfg(not(windows))]
fn main() {
    eprintln!("Windows でだけ動く");
}

#[cfg(windows)]
fn main() -> anyhow::Result<()> {
    windows_capture::run()
}

#[cfg(windows)]
mod windows_capture {
    use std::path::Path;
    use std::time::{Duration, Instant};

    use anyhow::Context;
    use windows::Graphics::Capture::{
        Direct3D11CaptureFramePool, GraphicsCaptureItem, GraphicsCaptureSession,
    };
    use windows::Graphics::DirectX::Direct3D11::IDirect3DDevice;
    use windows::Graphics::DirectX::DirectXPixelFormat;
    use windows::Win32::Foundation::{FILETIME, HMODULE};
    use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_HARDWARE;
    use windows::Win32::Graphics::Direct3D11::{
        D3D11_BOX, D3D11_CPU_ACCESS_READ, D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_MAP_READ,
        D3D11_MAPPED_SUBRESOURCE, D3D11_SDK_VERSION, D3D11_TEXTURE2D_DESC, D3D11_USAGE_STAGING,
        D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext, ID3D11Texture2D,
    };
    use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC};
    use windows::Win32::Graphics::Dxgi::IDXGIDevice;
    use windows::Win32::System::Threading::{GetCurrentProcess, GetProcessTimes};
    use windows::Win32::System::WinRT::Direct3D11::{
        CreateDirect3D11DeviceFromDXGIDevice, IDirect3DDxgiInterfaceAccess,
    };
    use windows::Win32::System::WinRT::Graphics::Capture::IGraphicsCaptureItemInterop;
    use windows::Win32::System::WinRT::{RO_INIT_MULTITHREADED, RoInitialize};
    use windows::Win32::UI::HiDpi::{
        DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext,
    };
    use windows::Win32::UI::WindowsAndMessaging::FindWindowW;
    use windows::core::{Interface, w};

    /// 配置は 1080p で測った。ほかの解像度では、画面の高さに比例させる
    const BASE_HEIGHT: f32 = 1080.0;
    /// 読み出す範囲（1080p。左、上、右、下）。上部バーと、アイテム欄（ショップを開くと上にずれる分も入れる）
    const REGIONS: [[f32; 4]; 2] = [[0.0, 0.0, 1920.0, 120.0], [0.0, 890.0, 420.0, 1080.0]];
    /// ゲームが FPS を出す場所（1080p。右の端からの幅と、上の端からの高さ）
    const FPS: (f32, f32) = (240.0, 48.0);

    /// 取り込みの口。フレームは、プールの 1 枚を空けるたびに次が入る
    struct Capture {
        device: ID3D11Device,
        context: ID3D11DeviceContext,
        pool: Direct3D11CaptureFramePool,
        session: GraphicsCaptureSession,
        size: (u32, u32),
    }

    impl Capture {
        unsafe fn start() -> anyhow::Result<Self> {
            unsafe {
                let window = FindWindowW(None, w!("Deadlock")).context("Deadlock のウィンドウ")?;
                let (mut device, mut context) = (None::<ID3D11Device>, None::<ID3D11DeviceContext>);
                D3D11CreateDevice(
                    None,
                    D3D_DRIVER_TYPE_HARDWARE,
                    HMODULE::default(),
                    D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                    None,
                    D3D11_SDK_VERSION,
                    Some(&mut device),
                    None,
                    Some(&mut context),
                )?;
                let (device, context) = (device.context("device")?, context.context("context")?);
                let direct3d: IDirect3DDevice =
                    CreateDirect3D11DeviceFromDXGIDevice(&device.cast::<IDXGIDevice>()?)?.cast()?;
                let interop =
                    windows::core::factory::<GraphicsCaptureItem, IGraphicsCaptureItemInterop>()?;
                let item: GraphicsCaptureItem = interop.CreateForWindow(window)?;
                let size = item.Size()?;
                let pool = Direct3D11CaptureFramePool::CreateFreeThreaded(
                    &direct3d,
                    DirectXPixelFormat::B8G8R8A8UIntNormalized,
                    1,
                    size,
                )?;
                let session = pool.CreateCaptureSession(&item)?;
                let _ = session.SetIsBorderRequired(false);
                let _ = session.SetIsCursorCaptureEnabled(false);
                session.StartCapture()?;
                Ok(Self {
                    device,
                    context,
                    pool,
                    session,
                    size: (size.Width as u32, size.Height as u32),
                })
            }
        }

        /// いま出ている画面を待って、`read` に渡す。プールに残っているのは前に空けた直後の古い画面なので、
        /// それは捨てて次を待つ（プールは 1 枚なので、空けるまで次は入らない）。
        /// `wait` のうちに画面が変わらなければ None
        unsafe fn fresh<T>(
            &self,
            wait: Duration,
            read: impl FnOnce(&ID3D11Texture2D) -> anyhow::Result<T>,
        ) -> anyhow::Result<Option<T>> {
            unsafe {
                let started = Instant::now();
                if let Ok(stale) = self.pool.TryGetNextFrame() {
                    stale.Close()?;
                }
                let frame = loop {
                    match self.pool.TryGetNextFrame() {
                        Ok(frame) => break frame,
                        Err(_) if started.elapsed() < wait => {
                            std::thread::sleep(Duration::from_millis(1))
                        }
                        Err(_) => return Ok(None),
                    }
                };
                let texture: ID3D11Texture2D = frame
                    .Surface()?
                    .cast::<IDirect3DDxgiInterfaceAccess>()?
                    .GetInterface()?;
                let result = read(&texture);
                frame.Close()?;
                result.map(Some)
            }
        }

        /// `area`（左、上、右、下）を読み出す口
        unsafe fn reader(&self, area: [u32; 4]) -> anyhow::Result<Reader<'_>> {
            unsafe {
                let desc = D3D11_TEXTURE2D_DESC {
                    Width: area[2] - area[0],
                    Height: area[3] - area[1],
                    MipLevels: 1,
                    ArraySize: 1,
                    Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                    SampleDesc: DXGI_SAMPLE_DESC {
                        Count: 1,
                        Quality: 0,
                    },
                    Usage: D3D11_USAGE_STAGING,
                    BindFlags: 0,
                    CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
                    MiscFlags: 0,
                };
                let mut staging = None;
                self.device
                    .CreateTexture2D(&desc, None, Some(&mut staging))?;
                Ok(Reader {
                    capture: self,
                    staging: staging.context("staging")?,
                    area,
                })
            }
        }

        unsafe fn close(self) -> anyhow::Result<()> {
            self.session.Close()?;
            self.pool.Close()?;
            Ok(())
        }
    }

    /// 画面の決まった範囲を、GPU から読み出す。読み出し先は使い回す
    struct Reader<'a> {
        capture: &'a Capture,
        staging: ID3D11Texture2D,
        area: [u32; 4],
    }

    impl Reader<'_> {
        /// RGB の画素を、左上から順に返す
        unsafe fn read(&self, texture: &ID3D11Texture2D) -> anyhow::Result<Vec<u8>> {
            unsafe {
                let [left, top, right, bottom] = self.area;
                let context = &self.capture.context;
                context.CopySubresourceRegion(
                    &self.staging,
                    0,
                    0,
                    0,
                    0,
                    texture,
                    0,
                    Some(&D3D11_BOX {
                        left,
                        top,
                        front: 0,
                        right,
                        bottom,
                        back: 1,
                    }),
                );
                let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
                context.Map(&self.staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))?;
                let (width, height) = ((right - left) as usize, (bottom - top) as usize);
                let mut rgb = Vec::with_capacity(width * height * 3);
                for y in 0..height {
                    let row = std::slice::from_raw_parts(
                        (mapped.pData as *const u8).add(y * mapped.RowPitch as usize),
                        width * 4,
                    );
                    for pixel in row.chunks_exact(4) {
                        rgb.extend([pixel[2], pixel[1], pixel[0]]);
                    }
                }
                context.Unmap(&self.staging, 0);
                Ok(rgb)
            }
        }
    }

    /// このプロセスが使った CPU の時間
    fn cpu_time() -> Duration {
        let mut times = [FILETIME::default(); 4];
        let [created, exited, kernel, user] = &mut times;
        unsafe {
            let _ = GetProcessTimes(GetCurrentProcess(), created, exited, kernel, user);
        }
        let ticks = |time: FILETIME| (time.dwHighDateTime as u64) << 32 | time.dwLowDateTime as u64;
        Duration::from_nanos((ticks(times[2]) + ticks(times[3])) * 100)
    }

    pub fn run() -> anyhow::Result<()> {
        let mut args = std::env::args().skip(1);
        let mode = args
            .next()
            .context("測り方（none、idle、full、region）を渡す")?;
        anyhow::ensure!(
            ["none", "idle", "full", "region"].contains(&mode.as_str()),
            "測り方は none、idle、full、region のどれか"
        );
        let seconds: u64 = args.next().map(|v| v.parse()).transpose()?.unwrap_or(30);
        let folder = args.next().unwrap_or_else(|| ".".to_owned());
        let rate: u32 = args.next().map(|v| v.parse()).transpose()?.unwrap_or(3);
        let period = Duration::from_secs(1) / rate;

        unsafe {
            let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
            RoInitialize(RO_INIT_MULTITHREADED)?;

            let before = cpu_time();
            let started = Instant::now();
            let capture = (mode != "none").then(|| Capture::start()).transpose()?;
            let mut waits = Vec::new();
            let mut reads = Vec::new();
            let mut missed = 0;
            if let Some(capture) = &capture {
                let scale = capture.size.1 as f32 / BASE_HEIGHT;
                let areas: Vec<[u32; 4]> = match mode.as_str() {
                    "full" => vec![[0, 0, capture.size.0, capture.size.1]],
                    "region" => REGIONS
                        .iter()
                        .map(|area| {
                            let [left, top, right, bottom] =
                                area.map(|v| (v * scale).round() as u32);
                            [
                                left,
                                top,
                                right.min(capture.size.0),
                                bottom.min(capture.size.1),
                            ]
                        })
                        .collect(),
                    _ => Vec::new(),
                };
                let readers = areas
                    .into_iter()
                    .map(|area| capture.reader(area))
                    .collect::<anyhow::Result<Vec<_>>>()?;
                let mut tick = 0;
                while started.elapsed() < Duration::from_secs(seconds) {
                    let waiting = Instant::now();
                    let read = capture.fresh(Duration::from_millis(50), |texture| {
                        let reading = Instant::now();
                        for reader in &readers {
                            std::hint::black_box(reader.read(texture)?);
                        }
                        Ok((reading, reading.elapsed()))
                    })?;
                    match read {
                        Some((reading, read)) => {
                            waits.push(reading - waiting);
                            reads.push(read);
                        }
                        None => missed += 1,
                    }
                    tick += 1;
                    std::thread::sleep((period * tick).saturating_sub(started.elapsed()));
                }
            } else {
                std::thread::sleep(Duration::from_secs(seconds));
            }
            let elapsed = started.elapsed();
            let cpu = cpu_time() - before;

            // FPS の表示を書く。取り込まない測り方では、ここで初めて取り込む
            let capture = match capture {
                Some(capture) => capture,
                None => Capture::start()?,
            };
            let scale = capture.size.1 as f32 / BASE_HEIGHT;
            let (width, height) = (
                (FPS.0 * scale).round() as u32,
                (FPS.1 * scale).round() as u32,
            );
            let reader = capture.reader([capture.size.0 - width, 0, capture.size.0, height])?;
            let rgb = capture
                .fresh(Duration::from_secs(2), |texture| reader.read(texture))?
                .context("フレームが来ない")?;
            let shown = image::RgbImage::from_raw(width, height, rgb).context("image")?;
            let shown = image::imageops::resize(
                &shown,
                width * 3,
                height * 3,
                image::imageops::FilterType::Nearest,
            );
            let path = Path::new(&folder).join(format!("{mode}-fps.png"));
            shown.save(&path)?;
            drop(reader);
            capture.close()?;

            let stats = |times: &mut Vec<Duration>| {
                times.sort();
                let mean = times.iter().sum::<Duration>() / times.len().max(1) as u32;
                let at = |part: f32| {
                    times
                        .get(((times.len() as f32 * part) as usize).min(times.len().max(1) - 1))
                        .copied()
                        .unwrap_or_default()
                };
                format!(
                    "平均 {:.2?}　95% {:.2?}　最大 {:.2?}",
                    mean,
                    at(0.95),
                    at(1.0)
                )
            };
            println!(
                "{mode}　{:.1} 秒　{} 回（来なかった {missed} 回）　CPU {:.0?}（1 コアの {:.2}%）",
                elapsed.as_secs_f32(),
                reads.len(),
                cpu,
                cpu.as_secs_f32() / elapsed.as_secs_f32() * 100.0,
            );
            if !reads.is_empty() {
                println!("  新しい画面を待つ {}", stats(&mut waits));
                println!("  読み出し {}", stats(&mut reads));
            }
            println!("  FPS の表示 {}", path.display());
        }
        Ok(())
    }
}
