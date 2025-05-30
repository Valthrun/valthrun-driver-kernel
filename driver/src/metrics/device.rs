use alloc::{
    string::String,
    vec::Vec,
};

use winapi::shared::{
    ntdef::PVOID,
    ntstatus::{
        STATUS_BUFFER_TOO_SMALL,
        STATUS_SUCCESS,
    },
};

use super::data::DeviceInfo;
use crate::{
    imports::ExGetSystemFirmwareTable,
    winver::os_info,
};

const FT_PROVIDER_RSMB: u32 = 0x52534d42; // hex for 'RSMB'
fn get_bios_unique_id() -> anyhow::Result<Option<String>> {
    let table_size = unsafe {
        let mut result = 0;
        let status_code =
            ExGetSystemFirmwareTable(FT_PROVIDER_RSMB, 0, core::ptr::null_mut(), 0, &mut result);
        if status_code != STATUS_BUFFER_TOO_SMALL {
            anyhow::bail!("recv size: {:X}", status_code);
        }

        result as usize
    };

    let mut buffer = Vec::<u8>::new();
    buffer.resize(table_size, 0);
    let table_size = unsafe {
        let mut result = 0;
        let status_code = ExGetSystemFirmwareTable(
            FT_PROVIDER_RSMB,
            0,
            buffer.as_mut_ptr() as PVOID,
            buffer.len() as u32,
            &mut result,
        );
        if status_code != STATUS_SUCCESS {
            anyhow::bail!("recv: {:X}", status_code);
        }

        result as usize
    };

    let mut offset = 0x08; // 0x08 = sizeof(RawSMBIOSData)
    while offset + 4 < table_size {
        let table_type = buffer[offset];
        let table_length = buffer[offset + 1];
        if table_length < 4 {
            break;
        }

        if table_type != 0x01 || table_length < 0x19 {
            offset += table_length as usize;

            /* skip over unformatted area */
            while offset + 2 < table_size {
                if u16::from_be_bytes(buffer[offset..offset + 2].try_into().unwrap()) == 0 {
                    /* marker found */
                    break;
                }

                offset += 1;
            }
            offset += 2;
            continue;
        }

        /* bios uuid found */
        offset += 0x08; // UUID offset

        /*
         * Note:
         * As off version 2.6 of the SMBIOS specification, the first 3 fields of the UUID are supposed to be encoded on little-endian. (para 7.2.1)
         * We ignore this here, asd it's still unique, just not in a proper uuid format.
         */
        return Ok(Some(hex::encode(&buffer[offset..offset + 16])));
    }

    Ok(None)
}

pub fn resolve_info() -> anyhow::Result<DeviceInfo> {
    let bios_uuid = get_bios_unique_id()?;
    let win = os_info();

    let csd_length = win
        .szCSDVersion
        .iter()
        .position(|c| *c == 0x00)
        .unwrap_or(0);

    Ok(DeviceInfo {
        bios_uuid,

        win_major_version: win.dwMajorVersion,
        win_minor_version: win.dwMinorVersion,
        win_build_no: win.dwBuildNumber,
        win_platform_id: win.dwPlatformId,

        win_csd_version: String::from_utf16_lossy(&win.szCSDVersion[0..csd_length]),
        win_service_pack_major: win.wServicePackMajor,
        win_service_pack_minor: win.wServicePackMinor,

        win_suite_mask: win.wSuiteMask,
        win_product_type: win.wProductType,
    })
}
