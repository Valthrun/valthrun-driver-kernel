#![no_std]
#![feature(error_in_core)]
#![feature(sync_unsafe_cell)]
#![feature(pointer_byte_offsets)]
#![feature(result_flattening)]
#![feature(new_uninit)]
#![feature(const_transmute_copy)]
#![allow(dead_code)]

use alloc::{boxed::Box, string::ToString, format};
use core::cell::SyncUnsafeCell;

use device::ValthrunDevice;
use handler::HandlerRegistry;
use kapi::{
    NTStatusEx,
    UnicodeStringEx,
};
use kb::KeyboardInput;
use metrics::MetricsClient;
use mouse::MouseInput;
use obfstr::obfstr;
use valthrun_driver_shared::requests::{
    RequestCSModule,
    RequestHealthCheck,
    RequestKeyboardState,
    RequestMouseMove,
    RequestProtectionToggle,
    RequestRead,
};
use winapi::{
    km::wdm::{
        DbgPrintEx,
        DRIVER_OBJECT,
    },
    shared::{
        ntdef::{
            NTSTATUS,
            UNICODE_STRING,
        },
        ntstatus::{
            STATUS_OBJECT_NAME_COLLISION,
            STATUS_SUCCESS,
        },
    },
};

use crate::{
    handler::{
        handler_get_modules,
        handler_keyboard_state,
        handler_mouse_move,
        handler_protection_toggle,
        handler_read,
    },
    imports::GLOBAL_IMPORTS,
    kapi::device_general_irp_handler,
    kdef::{
        IoCreateDriver,
        KeGetCurrentIrql,
        DPFLTR_LEVEL,
    },
    logger::APP_LOGGER,
    offsets::initialize_nt_offsets,
    winver::{
        initialize_os_info,
        os_info,
    },
    wsk::WskInstance,
};

mod device;
mod handler;
mod imports;
mod io;
mod kapi;
mod kb;
mod kdef;
mod logger;
mod metrics;
mod mouse;
mod offsets;
mod panic_hook;
mod pmem;
mod process_protection;
mod util;
mod winver;
mod wsk;

mod status;
use status::*;

extern crate alloc;

// FIXME: Exchange SyncUnsafeCell with a RwLock
pub static WSK: SyncUnsafeCell<Option<WskInstance>> = SyncUnsafeCell::new(None);
pub static REQUEST_HANDLER: SyncUnsafeCell<Option<Box<HandlerRegistry>>> =
    SyncUnsafeCell::new(Option::None);
pub static VALTHRUN_DEVICE: SyncUnsafeCell<Option<ValthrunDevice>> =
    SyncUnsafeCell::new(Option::None);
pub static KEYBOARD_INPUT: SyncUnsafeCell<Option<KeyboardInput>> =
    SyncUnsafeCell::new(Option::None);
pub static MOUSE_INPUT: SyncUnsafeCell<Option<MouseInput>> = SyncUnsafeCell::new(Option::None);
pub static METRICS_CLIENT: SyncUnsafeCell<Option<MetricsClient>> =
    SyncUnsafeCell::new(Option::None);

#[no_mangle]
extern "system" fn driver_unload(_driver: &mut DRIVER_OBJECT) {
    log::info!("Unloading...");

    /* Remove the device */
    let device_handle = unsafe { &mut *VALTHRUN_DEVICE.get() };
    let _ = device_handle.take();

    /* Delete request handler registry */
    let request_handler = unsafe { &mut *REQUEST_HANDLER.get() };
    let _ = request_handler.take();

    /* Uninstall process protection */
    process_protection::finalize();

    let keyboard_input = unsafe { &mut *KEYBOARD_INPUT.get() };
    let _ = keyboard_input.take();

    let mouse_input = unsafe { &mut *MOUSE_INPUT.get() };
    let _ = mouse_input.take();

    let metrics = unsafe { &mut *METRICS_CLIENT.get() };
    if let Some(mut metrics) = metrics.take() {
        metrics.shutdown();
    }

    /* shutdown WSK after after everthing else has been shut down */
    let wsk = unsafe { &mut *WSK.get() };
    let _ = wsk.take();

    log::info!("Driver Unloaded");
}

extern "system" {
    pub static MmSystemRangeStart: *const ();
}

#[no_mangle]
pub extern "system" fn driver_entry(
    driver: *mut DRIVER_OBJECT,
    registry_path: *const UNICODE_STRING,
) -> NTSTATUS {
    log::set_max_level(log::LevelFilter::Trace);
    if log::set_logger(&APP_LOGGER).is_err() {
        unsafe {
            DbgPrintEx(
                0,
                DPFLTR_LEVEL::ERROR as u32,
                obfstr!("[VT] Failed to initialize app logger!\n\0").as_ptr(),
            );
        }

        return CSTATUS_LOG_INIT_FAILED;
    }

    if let Err(error) = initialize_os_info() {
        log::error!("{} {}", obfstr!("Failed to load OS version info:"), error);
        return CSTATUS_DRIVER_PREINIT_FAILED;
    }

    match unsafe { driver.as_mut() } {
        Some(driver) => internal_driver_entry(driver, registry_path),
        None => {
            let target_driver_entry = internal_driver_entry as usize;
            log::info!("{}", obfstr!("Manually mapped driver."));
            log::debug!(
                "  System range start is {:X}, driver entry mapped at {:X}.",
                unsafe { MmSystemRangeStart } as u64,
                target_driver_entry
            );
            log::debug!("  IRQL level at {:X}", unsafe { KeGetCurrentIrql() });

            // TODO(low): May improve hiding via:
            // https://research.checkpoint.com/2021/a-deep-dive-into-doublefeature-equation-groups-post-exploitation-dashboard/
            let driver_name =
                UNICODE_STRING::from_bytes(obfstr::wide!("\\Driver\\valthrun-driver"));
            let result = unsafe { IoCreateDriver(&driver_name, target_driver_entry as *const _) };
            if let Err(code) = result.ok() {
                if code == STATUS_OBJECT_NAME_COLLISION {
                    log::error!("{}", obfstr!("Failed to create valthrun driver as a driver with this name is already loaded."));
                    CSTATUS_DRIVER_ALREADY_LOADED
                } else {
                    log::error!(
                        "{} {:X}",
                        obfstr!("Failed to create new driver for manually mapped driver:"),
                        code
                    );
                    CSTATUS_DRIVER_PREINIT_FAILED
                }
            } else {
                STATUS_SUCCESS
            }

            // To unload (Unload is not called):
            // if(gDriverObject->DriverUnload) {
            // gDriverObject->DriverUnload(gDriverObject);
            // }

            // ObMakeTemporaryObject (gDriverObject);
            // IoDeleteDriver (gDriverObject);
            // gDriverObject = NULL;
        }
    }
}

fn wsk_dummy() -> anyhow::Result<()> {
    // if let Some(metrics) = unsafe { &*METRICS_CLIENT.get() } {
    //     for i in 0..10 {
    //         metrics.add_record("testing".to_string(), format!("my test paload {}", i));
    //     }
    // }

    // let wsk = unsafe { &*WSK.get() };
    // let wsk = wsk.as_ref().context("missing WSK instance")?;
    // match metrics::send_report(&wsk, "/report", "{ \"message\": \"Hello World?\" }") {
    //     Ok(_) => {
    //         log::debug!("Success!");
    //     }
    //     Err(error) => {
    //         log::debug!("Fail: {:#}", error);
    //     }
    // }

    Ok(())
}

extern "C" fn internal_driver_entry(
    driver: &mut DRIVER_OBJECT,
    registry_path: *const UNICODE_STRING,
) -> NTSTATUS {
    let registry_path = unsafe { registry_path.as_ref() }.map(|path| path.as_string_lossy());
    {
        let registry_path = registry_path
            .as_ref()
            .map(|path| path.as_str())
            .unwrap_or("None");

        log::info!(
            "Initialize driver at {:X} ({:?}). WinVer {}.",
            driver as *mut _ as u64,
            registry_path,
            os_info().dwBuildNumber
        );
    }

    driver.DriverUnload = Some(driver_unload);
    if let Err(error) = GLOBAL_IMPORTS.resolve() {
        log::error!(
            "{}: {:#}",
            obfstr!("Failed to load the global import table"),
            error
        );
        return CSTATUS_DRIVER_INIT_FAILED;
    }

    if let Err(error) = kapi::setup_seh() {
        log::error!("{}: {:#}", obfstr!("Failed to initialize SEH"), error);
        return CSTATUS_DRIVER_INIT_FAILED;
    }

    if let Err(error) = kapi::mem::init() {
        log::error!(
            "{}: {:#}",
            obfstr!("Failed to initialize mem functions"),
            error
        );
        return CSTATUS_DRIVER_INIT_FAILED;
    }

    // {
    //     let mut buffer = [ 0x76u8; 32 ];
    //     let x = WskBuffer::create(&mut buffer);
    //     log::debug!("WSK Buffer error: {:?}", x.err());
    // }
    // return CSTATUS_DRIVER_INIT_FAILED;

    /* Needs to be done first as it's assumed to be init */
    if let Err(error) = initialize_nt_offsets() {
        log::error!(
            "{}: {:#}",
            obfstr!("Failed to initialize NT_OFFSETS"),
            error
        );
        return CSTATUS_DRIVER_INIT_FAILED;
    }

    for function in driver.MajorFunction.iter_mut() {
        *function = Some(device_general_irp_handler);
    }

    match WskInstance::create(1 << 8) {
        Ok(wsk) => {
            unsafe { *WSK.get() = Some(wsk) };
        }
        Err(err) => {
            log::error!("{}: {:#}", obfstr!("WSK initialize error"), err);
            return CSTATUS_DRIVER_INIT_FAILED;
        }
    }

    match metrics::initialize() {
        Err(error) => {
            log::error!("{}: {:#}", obfstr!("Failed to initialize metrics"), error);
            return CSTATUS_DRIVER_INIT_FAILED;
        }
        Ok(client) => {
            unsafe { *METRICS_CLIENT.get() = Some(client) };
        }
    }

    if let Err(err) = wsk_dummy() {
        log::error!("{}: {:#}", obfstr!("WSK dummy error"), err);
        return CSTATUS_DRIVER_INIT_FAILED;
    }

    // match kb::create_keyboard_input() {
    //     Err(error) => {
    //         log::error!(
    //             "{} {:#}",
    //             obfstr!("Failed to initialize keyboard input:"),
    //             error
    //         );
    //         return CSTATUS_DRIVER_INIT_FAILED;
    //     }
    //     Ok(keyboard) => {
    //         unsafe { *KEYBOARD_INPUT.get() = Some(keyboard) };
    //     }
    // }

    // match mouse::create_mouse_input() {
    //     Err(error) => {
    //         log::error!(
    //             "{} {:#}",
    //             obfstr!("Failed to initialize mouse input:"),
    //             error
    //         );
    //         return CSTATUS_DRIVER_INIT_FAILED;
    //     }
    //     Ok(mouse) => {
    //         unsafe { *MOUSE_INPUT.get() = Some(mouse) };
    //     }
    // }

    // if let Err(error) = process_protection::initialize() {
    //     log::error!(
    //         "{} {:#}",
    //         obfstr!("Failed to initialized process protection:"),
    //         error
    //     );
    //     return CSTATUS_DRIVER_INIT_FAILED;
    // };

    let device = match ValthrunDevice::create(driver) {
        Ok(device) => device,
        Err(error) => {
            log::error!("{} {:#}", obfstr!("Failed to initialize device:"), error);
            return CSTATUS_DRIVER_INIT_FAILED;
        }
    };
    log::debug!(
        "{} device Object at 0x{:X} (Handle at 0x{:X})",
        obfstr!("Valthrun"),
        device.device_handle.device as *const _ as u64,
        &*device.device_handle as *const _ as u64
    );
    unsafe { *VALTHRUN_DEVICE.get() = Some(device) };

    let mut handler = Box::new(HandlerRegistry::new());

    handler.register::<RequestHealthCheck>(&|_req, res| {
        res.success = true;
        Ok(())
    });
    handler.register::<RequestCSModule>(&handler_get_modules);
    handler.register::<RequestRead>(&handler_read);
    handler.register::<RequestProtectionToggle>(&handler_protection_toggle);
    handler.register::<RequestMouseMove>(&handler_mouse_move);
    handler.register::<RequestKeyboardState>(&handler_keyboard_state);

    unsafe { *REQUEST_HANDLER.get() = Some(handler) };

    log::info!("Driver Initialized");
    STATUS_SUCCESS
}
