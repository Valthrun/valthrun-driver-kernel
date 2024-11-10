use std::{
    ffi::c_void,
    slice,
    sync::RwLock,
};

use obfstr::{
    obfcstr,
    obfstr,
};
use valthrun_driver_protocol::{
    command::{
        DriverCommand,
        DriverCommandInitialize,
        InitializeResult,
    },
    utils::str_to_fixed_buffer,
    CommandResult,
    PROTOCOL_VERSION,
};
use windows::{
    core::PCSTR,
    Win32::{
        Foundation::{
            self,
            HANDLE,
        },
        Storage::FileSystem::{
            self,
            CreateFileA,
            FILE_FLAGS_AND_ATTRIBUTES,
        },
        System::{
            SystemServices::{
                DLL_PROCESS_ATTACH,
                DLL_PROCESS_DETACH,
            },
            IO::DeviceIoControl,
        },
    },
};

static DRIVER_INTERFACE: RwLock<HANDLE> = RwLock::new(HANDLE(-1));

#[no_mangle]
#[allow(non_snake_case)]
extern "system" fn DllMain(_dll_module: *const (), call_reason: u32, _: *mut ()) -> bool {
    match call_reason {
        DLL_PROCESS_ATTACH => {
            env_logger::init();
        }
        DLL_PROCESS_DETACH => (),
        _ => (),
    }

    true
}

#[no_mangle]
extern "C" fn execute_command(
    command_id: u32,

    payload: *mut u8,
    payload_length: usize,

    error_message: *mut u8,
    error_message_length: usize,
) -> u64 {
    let control_code = {
        (0x00000022 << 16) | // FILE_DEVICE_UNKNOWN
        (0x00000000 << 14) | // FILE_SPECIAL_ACCESS
        (0x00000001 << 13) | // Custom access code
        ((command_id & 0x3FF) << 02) |
        (0x00000003 << 00)
    };

    let payload = unsafe { slice::from_raw_parts_mut(payload, payload_length) };
    let error_message = unsafe { slice::from_raw_parts_mut(error_message, error_message_length) };

    if command_id == DriverCommandInitialize::COMMAND_ID {
        let command = unsafe { &mut *(payload.as_mut_ptr() as *mut DriverCommandInitialize) };

        /* initialize device handle */
        let handle = unsafe {
            CreateFileA(
                PCSTR::from_raw(
                    obfcstr!(cr"\\.\GLOBALROOT\Device\valthrun")
                        .to_bytes()
                        .as_ptr(),
                ),
                Foundation::GENERIC_READ.0 | Foundation::GENERIC_WRITE.0,
                FileSystem::FILE_SHARE_READ | FileSystem::FILE_SHARE_WRITE,
                None,
                FileSystem::OPEN_EXISTING,
                FILE_FLAGS_AND_ATTRIBUTES(0),
                None,
            )
        };

        let handle = match handle {
            Ok(handle) => handle,
            Err(err) => {
                command.driver_protocol_version = PROTOCOL_VERSION;
                if err.code().0 as u32 == 0x80070002 {
                    command.result = InitializeResult::Unavailable;
                    return CommandResult::Success.bits();
                } else {
                    str_to_fixed_buffer(
                        error_message,
                        &format!("{}: {:#}", obfstr!("open kernel driver"), err),
                    );
                    return CommandResult::Error.bits();
                }
            }
        };

        /*
         * Testing for the old pre v0.3.0 driver.
         * The function code 0x01 was assigned to a health check which required a
         * zero sized struct as input and a one byte long struct as output.
         *
         * The function code 0x01 is now assigned to DriverCommandProcessModules
         * which requires a different length hence this should fail for the new driver.
         */
        {
            let in_buffer = [0u8; 0x00];
            let mut out_buffer = [0u8; 0x01];
            let success = unsafe {
                const FUNCTION_ID_HEALTH_CHECK: u32 = 0x01;
                let control_code = {
                    (0x00000022 << 16) | // FILE_DEVICE_UNKNOWN
                    (0x00000000 << 14) | // FILE_SPECIAL_ACCESS
                    (0x00000001 << 13) | // Custom access code
                    ((FUNCTION_ID_HEALTH_CHECK & 0x3FF) << 02) |
                    (0x00000003 << 00)
                };

                DeviceIoControl(
                    handle,
                    control_code,
                    Some(in_buffer.as_ptr() as *const c_void),
                    in_buffer.len() as u32,
                    Some(out_buffer.as_mut_ptr() as *mut c_void),
                    out_buffer.len() as u32,
                    None,
                    None,
                )
            };

            log::debug!(
                "Pre v0.3.0 driver detection resulted in {} - {}",
                success.as_bool(),
                out_buffer[0]
            );
            if success.as_bool() && out_buffer[0] > 0 {
                /* old driver is still present... */
                str_to_fixed_buffer(error_message, &[
                    obfstr!(""),
                    obfstr!("** PLEASE READ CAREFULLY **"),
                    obfstr!("You have loaded an older version of the Valthrun Kernel Driver."),
                    obfstr!("Please update to the latest version of the Valthrun Kernel Driver."),
                    obfstr!(""),
                    obfstr!("For more information please refer to"),
                    obfstr!("https://wiki.valth.run/troubleshooting/overlay/driver_kernel_pre_v3_0_0"),
                ].join("\n"));
                return CommandResult::Error.bits();
            }
        }

        {
            let mut driver_handle = DRIVER_INTERFACE.write().unwrap();
            *driver_handle = handle;
        }
    }

    let handle = DRIVER_INTERFACE.read().unwrap();
    if handle.is_invalid() {
        str_to_fixed_buffer(error_message, "driver not initialized");
        return CommandResult::Error.bits();
    }

    let success = unsafe {
        DeviceIoControl(
            *handle,
            control_code,
            Some(payload.as_ptr() as *const c_void),
            payload.len() as u32,
            Some(payload.as_mut_ptr() as *mut c_void),
            payload.len() as u32,
            None,
            None,
        )
    };

    if success.as_bool() {
        CommandResult::Success.bits()
    } else {
        str_to_fixed_buffer(error_message, obfstr!("ioctrl error"));
        CommandResult::Error.bits()
    }
}
