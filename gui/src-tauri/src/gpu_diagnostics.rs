//! Windows GPU wiring diagnostics for the front-door spec sheet.
//!
//! The AllMyStuff node remains the source of the ordinary inventory. CEC
//! Support adds these Windows-only facts itself because they describe the
//! local physical installation: DXCore identifies integrated/discrete
//! adapters, DXGI maps desktop monitors to the adapter driving them, and
//! SetupAPI exposes the negotiated and maximum PCIe link widths.

use serde_json::Value;

#[derive(Clone, Debug, PartialEq, Eq)]
struct GpuObservation {
    name: String,
    integrated: Option<bool>,
    current_link_width: Option<u32>,
    maximum_link_width: Option<u32>,
    owns_primary_monitor: bool,
}

/// Add locally observed Windows-only fields to the node's existing `gpus`
/// array. A failed or ambiguous probe leaves the inventory untouched: an
/// unavailable reading must never become a red hardware warning.
pub fn augment_machine_specs(specs: &mut Value) {
    #[cfg(windows)]
    let observations = platform::collect().unwrap_or_default();
    #[cfg(not(windows))]
    let observations: Vec<GpuObservation> = Vec::new();

    augment_with(specs, &observations);
}

fn augment_with(specs: &mut Value, observations: &[GpuObservation]) {
    let Some(gpus) = specs.get_mut("gpus").and_then(Value::as_array_mut) else {
        return;
    };

    let primary_name = observations
        .iter()
        .find(|gpu| gpu.owns_primary_monitor)
        .map(|gpu| gpu.name.clone());
    let mut used = vec![false; observations.len()];

    for gpu in gpus {
        let Some(object) = gpu.as_object_mut() else {
            continue;
        };
        let Some(inventory_name) = object.get("name").and_then(Value::as_str) else {
            continue;
        };
        let Some(index) = best_name_match(inventory_name, observations, &used) else {
            continue;
        };
        used[index] = true;
        let observed = &observations[index];

        let kind = match observed.integrated {
            Some(true) => "integrated",
            Some(false) => "discrete",
            None => "unknown",
        };
        object.insert("kind".into(), Value::String(kind.into()));

        // The extra physical checks belong specifically under a discrete GPU.
        // Unknown classification stays neutral instead of guessing from a
        // vendor name or an arbitrary VRAM threshold.
        if observed.integrated != Some(false) {
            continue;
        }

        object.insert(
            "link_width".into(),
            observed.current_link_width.map_or(Value::Null, Value::from),
        );
        object.insert(
            "max_link_width".into(),
            observed.maximum_link_width.map_or(Value::Null, Value::from),
        );
        object.insert(
            "primary_monitor".into(),
            if primary_name.is_some() {
                Value::Bool(observed.owns_primary_monitor)
            } else {
                Value::Null
            },
        );
        object.insert(
            "primary_monitor_adapter".into(),
            primary_name.clone().map_or(Value::Null, Value::String),
        );
    }
}

fn best_name_match(
    inventory_name: &str,
    observations: &[GpuObservation],
    used: &[bool],
) -> Option<usize> {
    let wanted = normalized_name(inventory_name);
    observations
        .iter()
        .enumerate()
        .find(|(index, gpu)| !used[*index] && normalized_name(&gpu.name) == wanted)
        .map(|(index, _)| index)
        .or_else(|| {
            observations
                .iter()
                .enumerate()
                .filter(|(index, _)| !used[*index])
                .filter(|(_, gpu)| {
                    let candidate = normalized_name(&gpu.name);
                    candidate.contains(&wanted) || wanted.contains(&candidate)
                })
                .map(|(index, _)| index)
                .next()
        })
}

fn normalized_name(name: &str) -> String {
    name.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

#[cfg(windows)]
mod platform {
    use super::GpuObservation;
    use std::ffi::c_void;
    use std::mem::size_of;
    use windows::core::{GUID, PCWSTR};
    use windows::Win32::Devices::DeviceAndDriverInstallation::{
        SetupDiDestroyDeviceInfoList, SetupDiEnumDeviceInfo, SetupDiGetClassDevsW,
        SetupDiGetDevicePropertyW, DIGCF_PRESENT, GUID_DEVCLASS_DISPLAY, HDEVINFO, SP_DEVINFO_DATA,
    };
    use windows::Win32::Devices::Properties::{
        DEVPKEY_Device_HardwareIds, DEVPROPTYPE, DEVPROP_TYPE_STRING_LIST, DEVPROP_TYPE_UINT32,
    };
    use windows::Win32::Foundation::DEVPROPKEY;
    use windows::Win32::Graphics::DXCore::{
        DXCoreCreateAdapterFactory, IDXCoreAdapter, IDXCoreAdapterFactory, IsIntegrated,
    };
    use windows::Win32::Graphics::Dxgi::{
        CreateDXGIFactory1, IDXGIFactory1, DXGI_ADAPTER_FLAG_SOFTWARE,
    };
    use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, MONITORINFO};
    use windows::Win32::UI::WindowsAndMessaging::MONITORINFOF_PRIMARY;

    // PCI device properties from pciprop.h. They are intentionally declared
    // here because the generated windows-rs constants currently live behind
    // an unrelated Wi-Fi feature gate. The GUID/PIDs are Microsoft's stable
    // device-property contract.
    const PCI_DEVICE_PROPERTY_GUID: GUID = GUID::from_u128(0x3ab22e31_8264_4b4e_9af5_a8d2d8e33e62);
    const DEVPKEY_PCI_CURRENT_LINK_WIDTH: DEVPROPKEY = DEVPROPKEY {
        fmtid: PCI_DEVICE_PROPERTY_GUID,
        pid: 10,
    };
    const DEVPKEY_PCI_MAX_LINK_WIDTH: DEVPROPKEY = DEVPROPKEY {
        fmtid: PCI_DEVICE_PROPERTY_GUID,
        pid: 12,
    };

    #[derive(Clone, Debug)]
    struct PciDisplayDevice {
        vendor_id: u32,
        device_id: u32,
        subsystem_id: Option<u32>,
        current_link_width: Option<u32>,
        maximum_link_width: Option<u32>,
    }

    pub(super) fn collect() -> Result<Vec<GpuObservation>, String> {
        // DXCore is available on supported Windows 10/11 versions. Failure is
        // not fatal: DXGI can still enumerate adapters, but their type remains
        // unknown and the UI therefore stays neutral.
        let dxcore = unsafe { DXCoreCreateAdapterFactory::<IDXCoreAdapterFactory>() }.ok();
        let pci_devices = enumerate_pci_display_devices().unwrap_or_default();
        let factory = unsafe { CreateDXGIFactory1::<IDXGIFactory1>() }
            .map_err(|error| format!("DXGI factory: {error}"))?;
        let mut result = Vec::new();

        for adapter_index in 0.. {
            let Ok(adapter) = (unsafe { factory.EnumAdapters1(adapter_index) }) else {
                break;
            };
            let Ok(description) = (unsafe { adapter.GetDesc1() }) else {
                continue;
            };
            if description.Flags & DXGI_ADAPTER_FLAG_SOFTWARE.0 as u32 != 0 {
                continue;
            }

            let integrated = dxcore.as_ref().and_then(|factory| unsafe {
                factory
                    .GetAdapterByLuid::<IDXCoreAdapter>(&description.AdapterLuid)
                    .ok()
                    .and_then(|adapter| dxcore_bool(&adapter, IsIntegrated))
            });
            let owns_primary_monitor = adapter_owns_primary_monitor(&adapter);
            let pci = matching_pci_device(
                description.VendorId,
                description.DeviceId,
                description.SubSysId,
                &pci_devices,
            );

            result.push(GpuObservation {
                name: utf16_string(&description.Description),
                integrated,
                current_link_width: pci.and_then(|device| device.current_link_width),
                maximum_link_width: pci.and_then(|device| device.maximum_link_width),
                owns_primary_monitor,
            });
        }

        Ok(result)
    }

    unsafe fn dxcore_bool(
        adapter: &IDXCoreAdapter,
        property: windows::Win32::Graphics::DXCore::DXCoreAdapterProperty,
    ) -> Option<bool> {
        if !unsafe { adapter.IsPropertySupported(property) } {
            return None;
        }
        let mut value = false;
        unsafe {
            adapter
                .GetProperty(
                    property,
                    size_of::<bool>(),
                    (&mut value as *mut bool).cast::<c_void>(),
                )
                .ok()?;
        }
        Some(value)
    }

    fn adapter_owns_primary_monitor(
        adapter: &windows::Win32::Graphics::Dxgi::IDXGIAdapter1,
    ) -> bool {
        for output_index in 0.. {
            let Ok(output) = (unsafe { adapter.EnumOutputs(output_index) }) else {
                break;
            };
            let Ok(description) = (unsafe { output.GetDesc() }) else {
                continue;
            };
            if !description.AttachedToDesktop.as_bool() {
                continue;
            }
            let mut monitor = MONITORINFO {
                cbSize: size_of::<MONITORINFO>() as u32,
                ..Default::default()
            };
            if unsafe { GetMonitorInfoW(description.Monitor, &mut monitor) }.as_bool()
                && monitor.dwFlags & MONITORINFOF_PRIMARY != 0
            {
                return true;
            }
        }
        false
    }

    fn enumerate_pci_display_devices() -> Result<Vec<PciDisplayDevice>, String> {
        let set = unsafe {
            SetupDiGetClassDevsW(
                Some(&GUID_DEVCLASS_DISPLAY),
                PCWSTR::null(),
                None,
                DIGCF_PRESENT,
            )
        }
        .map_err(|error| format!("SetupAPI display set: {error}"))?;
        let _guard = DeviceInfoSet(set);
        let mut devices = Vec::new();

        for index in 0.. {
            let mut info = SP_DEVINFO_DATA {
                cbSize: size_of::<SP_DEVINFO_DATA>() as u32,
                ..Default::default()
            };
            if unsafe { SetupDiEnumDeviceInfo(set, index, &mut info) }.is_err() {
                break;
            }
            let Some(hardware_ids) = device_property(set, &info, &DEVPKEY_Device_HardwareIds)
                .and_then(|(kind, bytes)| {
                    (kind == DEVPROP_TYPE_STRING_LIST).then(|| utf16_strings(&bytes))
                })
            else {
                continue;
            };
            let Some((vendor_id, device_id, subsystem_id)) = hardware_ids
                .iter()
                .find_map(|hardware_id| parse_pci_hardware_id(hardware_id))
            else {
                continue;
            };

            devices.push(PciDisplayDevice {
                vendor_id,
                device_id,
                subsystem_id,
                current_link_width: device_u32(set, &info, &DEVPKEY_PCI_CURRENT_LINK_WIDTH),
                maximum_link_width: device_u32(set, &info, &DEVPKEY_PCI_MAX_LINK_WIDTH),
            });
        }

        Ok(devices)
    }

    struct DeviceInfoSet(HDEVINFO);

    impl Drop for DeviceInfoSet {
        fn drop(&mut self) {
            let _ = unsafe { SetupDiDestroyDeviceInfoList(self.0) };
        }
    }

    fn device_u32(set: HDEVINFO, info: &SP_DEVINFO_DATA, key: &DEVPROPKEY) -> Option<u32> {
        let (kind, bytes) = device_property(set, info, key)?;
        if kind != DEVPROP_TYPE_UINT32 || bytes.len() < size_of::<u32>() {
            return None;
        }
        Some(u32::from_le_bytes(bytes[..4].try_into().ok()?))
    }

    fn device_property(
        set: HDEVINFO,
        info: &SP_DEVINFO_DATA,
        key: &DEVPROPKEY,
    ) -> Option<(DEVPROPTYPE, Vec<u8>)> {
        let mut kind = DEVPROPTYPE::default();
        let mut required = 0u32;
        let _ = unsafe {
            SetupDiGetDevicePropertyW(set, info, key, &mut kind, None, Some(&mut required), 0)
        };
        if required == 0 {
            return None;
        }

        let mut bytes = vec![0u8; required as usize];
        unsafe {
            SetupDiGetDevicePropertyW(
                set,
                info,
                key,
                &mut kind,
                Some(&mut bytes),
                Some(&mut required),
                0,
            )
            .ok()?;
        }
        bytes.truncate(required as usize);
        Some((kind, bytes))
    }

    fn matching_pci_device(
        vendor_id: u32,
        device_id: u32,
        subsystem_id: u32,
        devices: &[PciDisplayDevice],
    ) -> Option<&PciDisplayDevice> {
        devices
            .iter()
            .find(|device| {
                device.vendor_id == vendor_id
                    && device.device_id == device_id
                    && device.subsystem_id == Some(subsystem_id)
            })
            .or_else(|| {
                let mut matches = devices.iter().filter(|device| {
                    device.vendor_id == vendor_id && device.device_id == device_id
                });
                let first = matches.next()?;
                matches.next().is_none().then_some(first)
            })
    }

    fn parse_pci_hardware_id(value: &str) -> Option<(u32, u32, Option<u32>)> {
        let uppercase = value.to_ascii_uppercase();
        if !uppercase.starts_with("PCI\\") {
            return None;
        }
        let vendor = hardware_id_hex(&uppercase, "VEN_", 4)?;
        let device = hardware_id_hex(&uppercase, "DEV_", 4)?;
        let subsystem = hardware_id_hex(&uppercase, "SUBSYS_", 8);
        Some((vendor, device, subsystem))
    }

    fn hardware_id_hex(value: &str, marker: &str, digits: usize) -> Option<u32> {
        let start = value.find(marker)? + marker.len();
        let end = start.checked_add(digits)?;
        u32::from_str_radix(value.get(start..end)?, 16).ok()
    }

    fn utf16_string(value: &[u16]) -> String {
        String::from_utf16_lossy(
            &value[..value.iter().position(|c| *c == 0).unwrap_or(value.len())],
        )
        .trim()
        .to_owned()
    }

    fn utf16_strings(bytes: &[u8]) -> Vec<String> {
        let words: Vec<u16> = bytes
            .chunks_exact(2)
            .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
            .collect();
        words
            .split(|word| *word == 0)
            .filter(|part| !part.is_empty())
            .map(String::from_utf16_lossy)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn observation(
        name: &str,
        integrated: Option<bool>,
        current_link_width: Option<u32>,
        maximum_link_width: Option<u32>,
        owns_primary_monitor: bool,
    ) -> GpuObservation {
        GpuObservation {
            name: name.into(),
            integrated,
            current_link_width,
            maximum_link_width,
            owns_primary_monitor,
        }
    }

    #[test]
    fn augments_only_the_discrete_gpu_with_physical_checks() {
        let mut specs = json!({
            "gpus": [
                { "name": "AMD Radeon(TM) Graphics", "vram_bytes": null },
                { "name": "NVIDIA GeForce RTX 4070", "vram_bytes": 12884901888u64 }
            ]
        });
        augment_with(
            &mut specs,
            &[
                observation("AMD Radeon(TM) Graphics", Some(true), None, None, true),
                observation(
                    "NVIDIA GeForce RTX 4070",
                    Some(false),
                    Some(8),
                    Some(16),
                    false,
                ),
            ],
        );

        assert_eq!(specs["gpus"][0]["kind"], "integrated");
        assert!(specs["gpus"][0].get("link_width").is_none());
        assert_eq!(specs["gpus"][1]["kind"], "discrete");
        assert_eq!(specs["gpus"][1]["link_width"], 8);
        assert_eq!(specs["gpus"][1]["max_link_width"], 16);
        assert_eq!(specs["gpus"][1]["primary_monitor"], false);
        assert_eq!(
            specs["gpus"][1]["primary_monitor_adapter"],
            "AMD Radeon(TM) Graphics"
        );
    }

    #[test]
    fn unknown_monitor_and_link_readings_stay_neutral() {
        let mut specs = json!({
            "gpus": [{ "name": "GeForce RTX 4090", "vram_bytes": 1 }]
        });
        augment_with(
            &mut specs,
            &[observation(
                "NVIDIA GeForce RTX 4090",
                Some(false),
                None,
                None,
                false,
            )],
        );

        assert_eq!(specs["gpus"][0]["kind"], "discrete");
        assert!(specs["gpus"][0]["link_width"].is_null());
        assert!(specs["gpus"][0]["max_link_width"].is_null());
        assert!(specs["gpus"][0]["primary_monitor"].is_null());
    }

    #[test]
    fn does_not_guess_when_an_inventory_name_does_not_match() {
        let mut specs = json!({
            "gpus": [{ "name": "Mystery Display Adapter", "vram_bytes": null }]
        });
        augment_with(
            &mut specs,
            &[observation(
                "NVIDIA GeForce RTX 4070",
                Some(false),
                Some(16),
                Some(16),
                true,
            )],
        );

        assert!(specs["gpus"][0].get("kind").is_none());
    }
}
