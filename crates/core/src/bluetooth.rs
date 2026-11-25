//! マイクが Bluetooth でつながっているかを OS に聞く。Bluetooth のイヤホンは、マイクを開くと通話用の
//! 接続に替わり、聞こえる音が悪くなる。cpal の `interface_type` は、macOS では接続の種類を返さず、
//! Windows でも通話用の口（Hands-Free）を Bluetooth と見なさないので、使わない。
//!
//! `id` は cpal の `DeviceId::id()`（macOS は機器の UID、Windows はエンドポイントの ID）。
//! 聞けなければ、Bluetooth ではないとして続ける

#[cfg(target_os = "macos")]
pub fn connected(id: &str) -> bool {
    use std::ffi::c_void;
    use std::ptr::NonNull;

    use objc2_core_audio::{
        AudioObjectGetPropertyData, AudioObjectPropertyAddress, kAudioDevicePropertyTransportType,
        kAudioDeviceTransportTypeBluetooth, kAudioDeviceTransportTypeBluetoothLE,
        kAudioHardwarePropertyTranslateUIDToDevice, kAudioObjectPropertyElementMain,
        kAudioObjectPropertyScopeGlobal, kAudioObjectSystemObject,
    };
    use objc2_core_foundation::CFString;

    /// `object` の `selector` の値（u32）。`qualifier` は、値を選ぶための CFString
    fn read(object: u32, selector: u32, qualifier: Option<&CFString>) -> Option<u32> {
        let address = AudioObjectPropertyAddress {
            mSelector: selector,
            mScope: kAudioObjectPropertyScopeGlobal,
            mElement: kAudioObjectPropertyElementMain,
        };
        // 修飾子は、CFString への参照（ポインタ）が入った場所を渡す
        let pointer = qualifier.map(|q| q as *const CFString);
        let (size, data) = match &pointer {
            Some(p) => (size_of::<*const CFString>(), p as *const _ as *const c_void),
            None => (0, std::ptr::null()),
        };
        let mut value = 0u32;
        let mut value_size = size_of::<u32>() as u32;
        // SAFETY: どちらの値も u32 で、`value_size` はその大きさ。修飾子は呼び出しの間、生きている
        let status = unsafe {
            AudioObjectGetPropertyData(
                object,
                NonNull::from(&address),
                size as u32,
                data,
                NonNull::from(&mut value_size),
                NonNull::from(&mut value).cast(),
            )
        };
        (status == 0).then_some(value)
    }

    let uid = CFString::from_str(id);
    read(
        kAudioObjectSystemObject as u32,
        kAudioHardwarePropertyTranslateUIDToDevice,
        Some(&uid),
    )
    .and_then(|device| read(device, kAudioDevicePropertyTransportType, None))
    .is_some_and(|transport| {
        [
            kAudioDeviceTransportTypeBluetooth,
            kAudioDeviceTransportTypeBluetoothLE,
        ]
        .contains(&transport)
    })
}

#[cfg(windows)]
pub fn connected(id: &str) -> bool {
    use windows::Win32::Devices::Properties::DEVPKEY_Device_EnumeratorName;
    use windows::Win32::Foundation::PROPERTYKEY;
    use windows::Win32::Media::Audio::{IMMDeviceEnumerator, MMDeviceEnumerator};
    use windows::Win32::System::Com::StructuredStorage::{PropVariantClear, PropVariantToBSTR};
    use windows::Win32::System::Com::{CLSCTX_ALL, CoCreateInstance, STGM_READ};
    use windows::core::HSTRING;

    /// 機器を見つけたバスの名前（USB、HDAUDIO、BTHENUM、BTHHFENUM など）
    fn enumerator(id: &str) -> windows::core::Result<String> {
        let key = PROPERTYKEY {
            fmtid: DEVPKEY_Device_EnumeratorName.fmtid,
            pid: DEVPKEY_Device_EnumeratorName.pid,
        };
        // SAFETY: COM は、先に機器を数えた cpal がこのスレッドで初期化している。
        // 取った値は、読んだあとに解放する
        unsafe {
            let devices: IMMDeviceEnumerator =
                CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
            let store = devices
                .GetDevice(&HSTRING::from(id))?
                .OpenPropertyStore(STGM_READ)?;
            let mut value = store.GetValue(&key)?;
            let name = PropVariantToBSTR(&value).map(|name| name.to_string());
            let _ = PropVariantClear(&mut value);
            name
        }
    }

    match enumerator(id) {
        Ok(name) => {
            tracing::debug!("mic {id}: {name}");
            is_bluetooth_bus(&name)
        }
        Err(e) => {
            tracing::debug!("mic {id}: {e}");
            false
        }
    }
}

#[cfg(not(any(target_os = "macos", windows)))]
pub fn connected(_id: &str) -> bool {
    false
}

/// Windows のバスの名前が Bluetooth のものか。音楽用は BTHENUM、通話用は BTHHFENUM、LE Audio は BTHLEDEVICE
#[cfg(any(windows, test))]
fn is_bluetooth_bus(enumerator: &str) -> bool {
    enumerator.to_ascii_uppercase().starts_with("BTH")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bluetooth_buses_are_told_from_the_others() {
        for bus in ["BTHENUM", "BTHHFENUM", "BTHLEDEVICE", "bthenum"] {
            assert!(is_bluetooth_bus(bus), "{bus}");
        }
        for bus in ["USB", "HDAUDIO", "MMDEVAPI", "SW", ""] {
            assert!(!is_bluetooth_bus(bus), "{bus}");
        }
    }

    #[test]
    fn an_unknown_microphone_is_not_bluetooth() {
        assert!(!connected("no such microphone"));
    }
}
