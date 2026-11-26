//! Deadlock のウィンドウを Windows.Graphics.Capture で取り込めるかを確かめる（DESIGN.md 15 章「画面からの読み取り」）。
//! 取り込んだ画面を PNG に書き、取り込みに掛かった時間を出す。
//!
//! usage: cargo run -p overlays-core --release --example capture -- <出力.png> [枚数] [間隔ミリ秒]
//!   枚数が 2 以上なら、`出力-1.png` のように番号を付けて書く

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
    use std::time::{Duration, Instant};

    use anyhow::Context;
    use windows::Graphics::Capture::{Direct3D11CaptureFramePool, GraphicsCaptureItem};
    use windows::Graphics::DirectX::Direct3D11::IDirect3DDevice;
    use windows::Graphics::DirectX::DirectXPixelFormat;
    use windows::Win32::Foundation::{HMODULE, HWND, RECT};
    use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_HARDWARE;
    use windows::Win32::Graphics::Direct3D11::{
        D3D11_CPU_ACCESS_READ, D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_MAP_READ,
        D3D11_MAPPED_SUBRESOURCE, D3D11_SDK_VERSION, D3D11_TEXTURE2D_DESC, D3D11_USAGE_STAGING,
        D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext, ID3D11Texture2D,
    };
    use windows::Win32::Graphics::Dxgi::IDXGIDevice;
    use windows::Win32::System::WinRT::Direct3D11::{
        CreateDirect3D11DeviceFromDXGIDevice, IDirect3DDxgiInterfaceAccess,
    };
    use windows::Win32::System::WinRT::Graphics::Capture::IGraphicsCaptureItemInterop;
    use windows::Win32::System::WinRT::{RO_INIT_MULTITHREADED, RoInitialize};
    use windows::Win32::UI::HiDpi::{
        DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        FindWindowW, GWL_EXSTYLE, GWL_STYLE, GetClientRect, GetForegroundWindow, GetWindowLongW,
        GetWindowRect,
    };
    use windows::core::{Interface, w};

    pub fn run() -> anyhow::Result<()> {
        let mut args = std::env::args().skip(1);
        let out = args.next().context("出力の PNG を渡す")?;
        let count: usize = args.next().map(|v| v.parse()).transpose()?.unwrap_or(1);
        let interval =
            Duration::from_millis(args.next().map(|v| v.parse()).transpose()?.unwrap_or(1000));

        unsafe {
            let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
            RoInitialize(RO_INIT_MULTITHREADED)?;

            let window: HWND =
                FindWindowW(None, w!("Deadlock")).context("Deadlock のウィンドウ")?;
            let (mut outer, mut client) = (RECT::default(), RECT::default());
            GetWindowRect(window, &mut outer)?;
            GetClientRect(window, &mut client)?;
            println!(
                "ウィンドウ ({}, {}) {}x{}　中身 {}x{}　style {:#010x}　exstyle {:#010x}　前面 {}",
                outer.left,
                outer.top,
                outer.right - outer.left,
                outer.bottom - outer.top,
                client.right,
                client.bottom,
                GetWindowLongW(window, GWL_STYLE),
                GetWindowLongW(window, GWL_EXSTYLE),
                GetForegroundWindow() == window,
            );

            let started = Instant::now();
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
            println!(
                "枠を消す {:?}　カーソルを外す {:?}",
                session.SetIsBorderRequired(false).map_err(|e| e.message()),
                session
                    .SetIsCursorCaptureEnabled(false)
                    .map_err(|e| e.message()),
            );
            session.StartCapture()?;
            println!(
                "取り込みを始めるまで {:.0?}　大きさ {}x{}",
                started.elapsed(),
                size.Width,
                size.Height
            );

            for index in 0..count {
                let waiting = Instant::now();
                // プールに残っているのは、前に空けた直後の古い画面。プールは 1 枚で、空けるまで次は入らないので、
                // 古いほうは捨てて次を待つ（画面が変わらないとフレームは来ない）
                if index > 0
                    && let Ok(stale) = pool.TryGetNextFrame()
                {
                    stale.Close()?;
                }
                let frame = loop {
                    match pool.TryGetNextFrame() {
                        Ok(frame) => break frame,
                        Err(_) => std::thread::sleep(Duration::from_millis(1)),
                    }
                    anyhow::ensure!(
                        waiting.elapsed() < Duration::from_secs(5),
                        "5 秒待ってもフレームが来ない"
                    );
                };
                let waited = waiting.elapsed();

                let reading = Instant::now();
                let texture: ID3D11Texture2D = frame
                    .Surface()?
                    .cast::<IDirect3DDxgiInterfaceAccess>()?
                    .GetInterface()?;
                let mut desc = D3D11_TEXTURE2D_DESC::default();
                texture.GetDesc(&mut desc);
                desc.Usage = D3D11_USAGE_STAGING;
                desc.BindFlags = 0;
                desc.CPUAccessFlags = D3D11_CPU_ACCESS_READ.0 as u32;
                desc.MiscFlags = 0;
                let mut staging = None;
                device.CreateTexture2D(&desc, None, Some(&mut staging))?;
                let staging = staging.context("staging")?;
                context.CopyResource(&staging, &texture);
                let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
                context.Map(&staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))?;
                let (width, height) = (desc.Width as usize, desc.Height as usize);
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
                context.Unmap(&staging, 0);
                let read = reading.elapsed();
                let content = frame.ContentSize()?;
                frame.Close()?;

                let path = if count == 1 {
                    out.clone()
                } else {
                    let path = std::path::Path::new(&out);
                    let stem = path.file_stem().unwrap_or_default().to_string_lossy();
                    path.with_file_name(format!("{stem}-{}.png", index + 1))
                        .to_string_lossy()
                        .into_owned()
                };
                image::RgbImage::from_raw(width as u32, height as u32, rgb)
                    .context("image")?
                    .save(&path)?;
                println!(
                    "{path}　{width}x{height}（中身 {}x{}）　待ち {waited:.0?}　読み出し {read:.1?}",
                    content.Width, content.Height
                );
                if index + 1 < count {
                    std::thread::sleep(interval.saturating_sub(waiting.elapsed()));
                }
            }
            session.Close()?;
            pool.Close()?;
        }
        Ok(())
    }
}
