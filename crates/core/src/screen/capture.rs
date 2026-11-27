//! Deadlock のウィンドウを取り込んで、画面の要るところだけを読み出す（DESIGN.md 18.2）。
//! Windows だけ（Windows.Graphics.Capture）。ほかの OS では、`open` がいつも `Ok(None)` を返す。
//!
//! 取り込みは動かしたままにして、呼ぶ側が要るときに `read` で新しい 1 枚を読む。
//! 同じスレッドから順に呼ぶ（`read` は新しいフレームを少し待つので、専用のスレッドで呼ぶ）。

use image::RgbImage;

/// 動いている取り込み。落とすと止まる
pub struct Capture {
    #[cfg(windows)]
    inner: windows::Inner,
}

impl Capture {
    /// Deadlock のウィンドウを探して、取り込みを始める。ウィンドウがなければ `Ok(None)`
    pub fn open() -> anyhow::Result<Option<Self>> {
        #[cfg(windows)]
        return Ok(windows::Inner::open()?.map(|inner| Self { inner }));
        #[cfg(not(windows))]
        Ok(None)
    }

    /// 取り込んでいるゲームの画面の大きさ（幅、高さ）。ウィンドウの表示では、タイトルバーと枠を除いた中身
    pub fn size(&self) -> (u32, u32) {
        #[cfg(windows)]
        return self.inner.size;
        #[cfg(not(windows))]
        (0, 0)
    }

    /// プールに残っている古い画面を捨て、新しい 1 枚を待って、`areas`（左、上、幅、高さ）を順に読み出す。
    /// どれも同じ 1 枚から読む。新しいフレームが来なければ `Ok(None)`（画面が変わっていないか、最小化している。
    /// そのまま次も呼べる）。ウィンドウが閉じた・大きさが変わった・GPU が使えなくなったときは `Err` で、
    /// この取り込みはもう使えない（落として、`open` し直す）
    pub fn read(&mut self, areas: &[[u32; 4]]) -> anyhow::Result<Option<Vec<RgbImage>>> {
        #[cfg(windows)]
        return self.inner.read(areas);
        #[cfg(not(windows))]
        {
            let _ = areas;
            Ok(None)
        }
    }
}

#[cfg(windows)]
mod windows {
    use std::time::{Duration, Instant};

    use ::windows::Graphics::Capture::{
        Direct3D11CaptureFramePool, GraphicsCaptureItem, GraphicsCaptureSession,
    };
    use ::windows::Graphics::DirectX::Direct3D11::IDirect3DDevice;
    use ::windows::Graphics::DirectX::DirectXPixelFormat;
    use ::windows::Graphics::SizeInt32;
    use ::windows::Win32::Foundation::{HMODULE, HWND, POINT, RECT};
    use ::windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_HARDWARE;
    use ::windows::Win32::Graphics::Direct3D11::{
        D3D11_BOX, D3D11_CPU_ACCESS_READ, D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_MAP_READ,
        D3D11_MAPPED_SUBRESOURCE, D3D11_SDK_VERSION, D3D11_TEXTURE2D_DESC, D3D11_USAGE_STAGING,
        D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext, ID3D11Texture2D,
    };
    use ::windows::Win32::Graphics::Dwm::{DWMWA_EXTENDED_FRAME_BOUNDS, DwmGetWindowAttribute};
    use ::windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC};
    use ::windows::Win32::Graphics::Dxgi::IDXGIDevice;
    use ::windows::Win32::Graphics::Gdi::ClientToScreen;
    use ::windows::Win32::System::WinRT::Direct3D11::{
        CreateDirect3D11DeviceFromDXGIDevice, IDirect3DDxgiInterfaceAccess,
    };
    use ::windows::Win32::System::WinRT::Graphics::Capture::IGraphicsCaptureItemInterop;
    use ::windows::Win32::System::WinRT::{RO_INIT_MULTITHREADED, RoInitialize};
    use ::windows::Win32::UI::HiDpi::{
        DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetThreadDpiAwarenessContext,
    };
    use ::windows::Win32::UI::WindowsAndMessaging::{FindWindowW, GetClientRect, IsWindow};
    use ::windows::core::{Interface, w};
    use anyhow::Context;
    use image::RgbImage;

    /// 新しいフレームを待つ長さ。フレームは画面の書き換え（60 Hz なら 17 ミリ秒ごと）に合わせて届き、
    /// 測ったときは平均 10 ミリ秒、最大 20 ミリ秒弱だった（DESIGN.md 15 章）。
    /// ゲームが裏にあるときは 46 FPS ほどに落ちる
    const WAIT: Duration = Duration::from_millis(100);

    pub struct Inner {
        /// ウィンドウのハンドル。まだあるかを確かめるのに使う
        window: isize,
        device: ID3D11Device,
        context: ID3D11DeviceContext,
        pool: Direct3D11CaptureFramePool,
        session: GraphicsCaptureSession,
        /// 取り込んだフレームの大きさ。ウィンドウの表示では、タイトルバーと枠を含む
        content: (u32, u32),
        /// ゲームの画面（ウィンドウの中身）の、フレームの中での左上
        origin: (u32, u32),
        /// ゲームの画面の大きさ
        pub size: (u32, u32),
        /// 読み出し先。大きさごとに 1 つ作って、使い回す
        staging: Vec<((u32, u32), ID3D11Texture2D)>,
    }

    // SAFETY: Direct3D 11 のデバイスはどのスレッドからでも呼べる。コンテキストは同時には呼べないが、
    // 触るのは `&mut self` の `read` だけ。プールは `CreateFreeThreaded` で作っている
    unsafe impl Send for Inner {}

    impl Inner {
        pub fn open() -> anyhow::Result<Option<Self>> {
            unsafe {
                let Ok(window) = FindWindowW(None, w!("Deadlock")) else {
                    return Ok(None);
                };
                // すでに別の形で初期化されたスレッドでは失敗するが、そのまま使える
                let _ = RoInitialize(RO_INIT_MULTITHREADED);

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
                    ::windows::core::factory::<GraphicsCaptureItem, IGraphicsCaptureItemInterop>()?;
                // 探してから取り込むまでの間に、ウィンドウが閉じることがある
                let Ok(item) = interop.CreateForWindow::<GraphicsCaptureItem>(window) else {
                    return Ok(None);
                };
                let size = item.Size()?;
                let content = (size.Width as u32, size.Height as u32);
                let (origin, inside) = client(window, content);
                // プールは 1 枚。手元のフレームを閉じるまで、次は入らない
                let pool = Direct3D11CaptureFramePool::CreateFreeThreaded(
                    &direct3d,
                    DirectXPixelFormat::B8G8R8A8UIntNormalized,
                    1,
                    size,
                )?;
                let session = pool.CreateCaptureSession(&item)?;
                // 取り込んでいる印の黄色い枠と、マウスのカーソルは要らない。古い Windows では設定できない
                let _ = session.SetIsBorderRequired(false);
                let _ = session.SetIsCursorCaptureEnabled(false);
                session.StartCapture()?;
                Ok(Some(Self {
                    window: window.0 as isize,
                    device,
                    context,
                    pool,
                    session,
                    content,
                    origin,
                    size: inside,
                    staging: Vec::new(),
                }))
            }
        }

        pub fn read(&mut self, areas: &[[u32; 4]]) -> anyhow::Result<Option<Vec<RgbImage>>> {
            unsafe {
                anyhow::ensure!(
                    IsWindow(Some(HWND(self.window as _))).as_bool(),
                    "ウィンドウが閉じた"
                );
                // プールに残っているのは、前に閉じた直後の古い画面
                if let Ok(stale) = self.pool.TryGetNextFrame() {
                    stale.Close()?;
                }
                let waiting = Instant::now();
                let frame = loop {
                    match self.pool.TryGetNextFrame() {
                        Ok(frame) => break frame,
                        Err(_) if waiting.elapsed() < WAIT => {
                            std::thread::sleep(Duration::from_millis(1))
                        }
                        Err(_) => return Ok(None),
                    }
                };
                let SizeInt32 { Width, Height } = frame.ContentSize()?;
                let texture: ID3D11Texture2D = frame
                    .Surface()?
                    .cast::<IDirect3DDxgiInterfaceAccess>()?
                    .GetInterface()?;
                let images = if (Width as u32, Height as u32) == self.content {
                    areas
                        .iter()
                        .map(|&area| self.copy(&texture, area))
                        .collect::<anyhow::Result<Vec<_>>>()
                } else {
                    Err(anyhow::anyhow!(
                        "ウィンドウの大きさが変わった（{Width}x{Height}）"
                    ))
                };
                frame.Close()?;
                images.map(Some)
            }
        }

        /// `texture` の `area`（左、上、幅、高さ）を読み出す
        unsafe fn copy(
            &mut self,
            texture: &ID3D11Texture2D,
            [left, top, width, height]: [u32; 4],
        ) -> anyhow::Result<RgbImage> {
            anyhow::ensure!(
                width > 0
                    && height > 0
                    && left + width <= self.size.0
                    && top + height <= self.size.1,
                "読み出す範囲が画面からはみ出している"
            );
            unsafe {
                let staging = match self
                    .staging
                    .iter()
                    .find(|(size, _)| *size == (width, height))
                {
                    Some((_, staging)) => staging.clone(),
                    None => {
                        let desc = D3D11_TEXTURE2D_DESC {
                            Width: width,
                            Height: height,
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
                        let staging = staging.context("staging")?;
                        self.staging.push(((width, height), staging.clone()));
                        staging
                    }
                };
                self.context.CopySubresourceRegion(
                    &staging,
                    0,
                    0,
                    0,
                    0,
                    texture,
                    0,
                    Some(&D3D11_BOX {
                        left: self.origin.0 + left,
                        top: self.origin.1 + top,
                        front: 0,
                        right: self.origin.0 + left + width,
                        bottom: self.origin.1 + top + height,
                        back: 1,
                    }),
                );
                let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
                self.context
                    .Map(&staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))?;
                let mut rgb = Vec::with_capacity((width * height * 3) as usize);
                for y in 0..height as usize {
                    let row = std::slice::from_raw_parts(
                        (mapped.pData as *const u8).add(y * mapped.RowPitch as usize),
                        width as usize * 4,
                    );
                    for &[blue, green, red, _] in row.as_chunks::<4>().0 {
                        rgb.extend([red, green, blue]);
                    }
                }
                self.context.Unmap(&staging, 0);
                RgbImage::from_raw(width, height, rgb).context("image")
            }
        }
    }

    /// ゲームの画面（ウィンドウの中身）が、取り込んだフレーム（大きさ `content`）のどこにあるか（左上、大きさ）。
    /// ウィンドウの表示では、フレームにタイトルバーと枠が入る。フレームの左上は、影を除いたウィンドウの左上
    /// （`DWMWA_EXTENDED_FRAME_BOUNDS`。OBS のウィンドウキャプチャと同じ求め方）。
    /// 全画面とボーダーレスでは、フレームの全体が中身。求められなければ、フレームの全体にする
    unsafe fn client(window: HWND, content: (u32, u32)) -> ((u32, u32), (u32, u32)) {
        unsafe {
            // 画面の拡大（125% など）に関わらず、画素で測る
            let before = SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
            let (mut frame, mut inside, mut corner) =
                (RECT::default(), RECT::default(), POINT::default());
            let measured = DwmGetWindowAttribute(
                window,
                DWMWA_EXTENDED_FRAME_BOUNDS,
                (&raw mut frame).cast(),
                size_of::<RECT>() as u32,
            )
            .is_ok()
                && GetClientRect(window, &mut inside).is_ok()
                && ClientToScreen(window, &mut corner).as_bool();
            SetThreadDpiAwarenessContext(before);
            let origin = (
                (corner.x - frame.left).max(0) as u32,
                (corner.y - frame.top).max(0) as u32,
            );
            let size = (inside.right.max(0) as u32, inside.bottom.max(0) as u32);
            let fits = size.0 > 0
                && size.1 > 0
                && origin.0 + size.0 <= content.0
                && origin.1 + size.1 <= content.1;
            if measured && fits {
                (origin, size)
            } else {
                ((0, 0), content)
            }
        }
    }

    impl Drop for Inner {
        fn drop(&mut self) {
            let _ = self.session.Close();
            let _ = self.pool.Close();
        }
    }
}
