use alloc::{
    boxed::Box,
    string::ToString,
    vec::Vec,
};
use core::{
    mem,
    slice,
};

use anyhow::{
    anyhow,
    Context,
};
use kapi::{
    FastMutex,
    NTStatusEx,
    ObjectType,
    Process,
    UnicodeStringEx,
};
use kapi_kmodule::KModule;
use kdef::{
    OB_FLT_REGISTRATION_VERSION,
    OB_OPERATION_HANDLE_CREATE,
    OB_OPERATION_HANDLE_DUPLICATE,
    _OB_CALLBACK_REGISTRATION,
    _OB_OPERATION_REGISTRATION,
    _OB_PRE_CREATE_HANDLE_INFORMATION,
    _OB_PRE_DUPLICATE_HANDLE_INFORMATION,
    _OB_PRE_OPERATION_INFORMATION,
};
use log::Level;
use obfstr::obfstr;
use once_cell::race::OnceBox;
use pelite::{
    image::{
        IMAGE_DIRECTORY_ENTRY_LOAD_CONFIG,
        IMAGE_GUARDCF64,
        IMAGE_GUARD_CF_INSTRUMENTED,
        IMAGE_LOAD_CONFIG_DIRECTORY64,
    },
    pe::{
        Pe,
        PeObject,
        PeView,
    },
};
use utils_pattern::ByteSequencePattern;
use winapi::shared::ntdef::{
    PVOID,
    UNICODE_STRING,
};

use crate::{
    imports::{
        ObRegisterCallbacks,
        ObUnRegisterCallbacks,
    },
    offsets::get_nt_offsets,
    util::{
        self,
        ErrorResponse,
        MB_DEFBUTTON3,
        MB_ICONEXCLAMATION,
        MB_SYSTEMMODAL,
        MB_YESNOCANCEL,
    },
};

struct ProtectionState {
    ob_registration: PVOID,
    protected_process_ids: Vec<i32>,
}

unsafe impl Send for ProtectionState {}
unsafe impl Sync for ProtectionState {}

static PROCESS_PROTECTION: OnceBox<FastMutex<Option<ProtectionState>>> = OnceBox::new();

fn process_protection_state() -> &'static FastMutex<Option<ProtectionState>> {
    PROCESS_PROTECTION.get_or_init(|| Box::new(FastMutex::new(None)))
}

/*
 * _ctx will point to the method itself as we needed a jump to get here.
 * See ObRegisterCallbacks for more info.
 */
extern "system" fn process_protection_callback(
    _ctx: PVOID,
    info: *const _OB_PRE_OPERATION_INFORMATION,
) -> u32 {
    let info = unsafe { &*info };

    let current_process = Process::current();
    let target_process = Process::from_raw(info.Object, false);

    if current_process.eprocess() == target_process.eprocess() || (info.Flags & 0x01) > 0 {
        /* own attachments and attachments from the kernel are allowed */
        return 0;
    }

    let target_process_id = target_process.get_id();
    if log::log_enabled!(target: "ProcessAttachments", Level::Trace) && false {
        let current_process_name = current_process.get_image_file_name().unwrap_or_default();
        if current_process_name != obfstr!("svchost.exe") &&
            current_process_name != obfstr!("WmiPrvSE.exe")
        {
            log::trace!("process_protection_callback. Caller: {:X} ({:?}), Target: {:X} ({:?}) Flags: {:X}, Operation: {:X}", 
                current_process.get_id(), current_process_name,
                target_process_id, target_process.get_image_file_name(),
                info.Flags, info.Operation);
        }
    }

    let is_protected = {
        let context = process_protection_state().lock();
        let context = match context.as_ref() {
            Some(ctx) => ctx,
            None => return 0,
        };

        context.protected_process_ids.contains(&target_process_id)
    };

    if !is_protected {
        /* all is good :) */
        return 0;
    }

    log::debug!("Process 0x{:X} ({}) tries to open a handle to the protected process 0x{:X} ({}) (Operation: 0x{:0<2X})", 
        current_process.get_id(), current_process.get_image_file_name().unwrap_or("[[ error ]]"), 
        target_process.get_id(), target_process.get_image_file_name().unwrap_or("[[ error ]]"), 
        info.Operation
    );

    match info.Operation {
        OB_OPERATION_HANDLE_CREATE => {
            let parameters = unsafe {
                &mut *core::mem::transmute::<_, *mut _OB_PRE_CREATE_HANDLE_INFORMATION>(
                    info.Parameters,
                )
            };

            // SYNCHRONIZE | PROCESS_QUERY_LIMITED_INFORMATION
            parameters.DesiredAccess = 0x00100000 | 0x1000;
        }
        OB_OPERATION_HANDLE_DUPLICATE => {
            let parameters = unsafe {
                &mut *core::mem::transmute::<_, *mut _OB_PRE_DUPLICATE_HANDLE_INFORMATION>(
                    info.Parameters,
                )
            };

            // SYNCHRONIZE | PROCESS_QUERY_LIMITED_INFORMATION
            parameters.DesiredAccess = 0x00100000 | 0x1000;
        }
        op => log::warn!("Tried to protect {target_process_id:X} but operation {op} unknown."),
    }
    0
}

pub fn toggle_protection(target_process_id: i32, target: bool) {
    let mut context = process_protection_state().lock();
    let context = match context.as_mut() {
        Some(ctx) => ctx,
        None => {
            log::warn!("Tried to protect process, but process protection not yet initialized");
            return;
        }
    };

    if target {
        if !context.protected_process_ids.contains(&target_process_id) {
            context.protected_process_ids.push(target_process_id);
        }

        log::debug!("Enabled process protection for {}", target_process_id);
    } else {
        if let Some(index) = context
            .protected_process_ids
            .iter()
            .position(|id| *id == target_process_id)
        {
            context.protected_process_ids.swap_remove(index);
            log::debug!("Disabled process protection for {}", target_process_id);
        }
    }
}

pub fn finalize() {
    let context = {
        let mut context = process_protection_state().lock();
        match context.take() {
            Some(ctx) => ctx,
            None => return,
        }
    };

    unsafe {
        ObUnRegisterCallbacks(context.ob_registration);
    }
}

fn get_pe_guard_config<'a>(view: &dyn PeObject<'a>) -> Option<&'a IMAGE_GUARDCF64> {
    let dict_load_config = view
        .data_directory()
        .get(IMAGE_DIRECTORY_ENTRY_LOAD_CONFIG)?;

    if (dict_load_config.Size as usize) <
        mem::size_of::<IMAGE_LOAD_CONFIG_DIRECTORY64>() + mem::size_of::<IMAGE_GUARDCF64>()
    {
        return None;
    }

    view.derva(
        dict_load_config.VirtualAddress + mem::size_of::<IMAGE_LOAD_CONFIG_DIRECTORY64>() as u32,
    )
    .ok()
}

fn find_callback_target(filter_cfg: bool) -> anyhow::Result<Option<usize>> {
    let pattern = ByteSequencePattern::parse(obfstr!("FF E1"))
        .with_context(|| obfstr!("failed to compile jmp rcx pattern").to_string())?;

    #[allow(non_snake_case)]
    let MmVerifyCallbackFunctionFlags = get_nt_offsets().MmVerifyCallbackFunctionFlags;

    log::trace!("Searching for a valid ObRegisterCallback jump target (filter_cfg = {filter_cfg})");
    for module in KModule::query_modules()? {
        if !module.is_base_data_valid() {
            continue;
        }

        if filter_cfg {
            let image = unsafe {
                slice::from_raw_parts(module.base_address as *const u8, module.module_size)
            };

            let Ok(pe_view) = PeView::from_bytes(image) else {
                continue;
            };

            if let Some(config) = self::get_pe_guard_config(&pe_view) {
                if (config.GuardFlags & IMAGE_GUARD_CF_INSTRUMENTED) > 0 {
                    log::debug!(
                        "  -> skipping {} (0x{:X}) because CFG is enabled.",
                        module.file_name,
                        module.base_address
                    );
                    continue;
                }
            }
        }

        log::trace!(
            "  -> scanning {} ({:X} - {:X})",
            module.file_name,
            module.base_address,
            module.base_address + module.module_size
        );
        let Ok(sections) = module.find_code_sections() else {
            continue;
        };

        let Some(jmp_target) = sections
            .iter()
            .filter(|section| section.is_data_valid())
            .filter(|section| {
                // log::debug!(" Testing {} at {:X} ({:X} bytes)", section.name, section.raw_data_address(), section.size_of_raw_data);
                unsafe { MmVerifyCallbackFunctionFlags(section.raw_data_address() as PVOID, 0x20) }
            })
            .find_map(|section| {
                // log::debug!("  Searching pattern");
                section.find_pattern(&pattern)
            })
        else {
            continue;
        };

        log::debug!(
            "Found ObRegisterCallback target in {} at {:X}",
            module.file_path,
            jmp_target
        );
        return Ok(Some(jmp_target));
    }

    Ok(None)
}

#[allow(unused)]
pub fn initialize() -> anyhow::Result<bool> {
    let mut context = process_protection_state().lock();
    if context.is_some() {
        anyhow::bail!("{}", obfstr!("process protection already initialized"));
    }

    let jmp_target = if let Some(target) = self::find_callback_target(true)? {
        target
    } else {
        let result = util::show_msgbox(
            obfstr!("Valthrun Kernel Driver"),
            &[
                obfstr!("Failed to find a CFG compatible jump target for ObRegisterCallbacks."),
                obfstr!(""),
                obfstr!("Would you like to ignore the CFG compatibility?"),
                obfstr!("This will cause a BSOD when your kernel has CFG enabled."),
                obfstr!(""),
                obfstr!("For more information please refer to"),
                obfstr!("https://wiki.valth.run/link/vtdk-2"),
                obfstr!(""),
                obfstr!("Press \"No\" to disable the process protection module."),
            ]
            .join("\n"),
            MB_YESNOCANCEL | MB_DEFBUTTON3 | MB_ICONEXCLAMATION | MB_SYSTEMMODAL,
        );
        match result {
            ErrorResponse::Yes => {
                if let Some(target) = self::find_callback_target(false)? {
                    target
                } else {
                    anyhow::bail!("failed to find a jump target")
                }
            }
            ErrorResponse::No => {
                log::info!("Process protection disabled.");
                return Ok(false);
            }
            _ => {
                anyhow::bail!("failed to find a CFG compatible jump target")
            }
        }
    };

    let mut reg_handle = core::ptr::null_mut();
    *context = unsafe {
        let mut operation_reg = core::mem::zeroed::<_OB_OPERATION_REGISTRATION>();
        operation_reg.ObjectType = ObjectType::PsProcessType.resolve_system_type();
        operation_reg.Operations = OB_OPERATION_HANDLE_CREATE | OB_OPERATION_HANDLE_DUPLICATE;
        operation_reg.PostOperation = None;

        let mut callback_reg = core::mem::zeroed::<_OB_CALLBACK_REGISTRATION>();
        callback_reg.Version = OB_FLT_REGISTRATION_VERSION;
        callback_reg.Altitude = UNICODE_STRING::from_bytes(obfstr::wide!("1")); /* Yes we want to be one of the first */
        callback_reg.OperationRegistration = &operation_reg;
        callback_reg.OperationRegistrationCount = 1;

        // https://www.unknowncheats.me/forum/2350590-post9.html
        operation_reg.PreOperation = Some(core::mem::transmute(jmp_target));
        callback_reg.RegistrationContext = process_protection_callback as PVOID;

        // An anticheat which registers a lowest and highest altitude callback
        // can just reset the desiered permissions (especially with file name filtering).
        // Therefore this "protection" is easily removeable. Anyhow this requires a kernel module!
        ObRegisterCallbacks(&callback_reg, &mut reg_handle)
            .ok()
            .map_err(|err| {
                anyhow!(
                    "ObRegisterCallbacks ({:X}) {:X}",
                    operation_reg.PreOperation.unwrap() as usize,
                    err
                )
            })?;

        Some(ProtectionState {
            ob_registration: reg_handle,
            protected_process_ids: Default::default(),
        })
    };

    Ok(true)
}
