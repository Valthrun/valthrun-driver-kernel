use alloc::{string::ToString, vec::Vec};
use anyhow::{anyhow, Context};
use obfstr::obfstr;
use valthrun_driver_shared::{ByteSequencePattern, KeyboardState};
use winapi::{km::wdm::DRIVER_OBJECT, shared::ntdef::{UNICODE_STRING, PVOID}};

use crate::{kapi::{Object, UnicodeStringEx, KModule}, kdef::{IoDriverObjectType, KeyboardClassServiceCallbackFn, KEYBOARD_INPUT_DATA, KEYBOARD_FLAG_MAKE, KEYBOARD_FLAG_BREAK}, offsets::NtOffsets};


pub struct KeyboardInput {
    kb_device: Object,
    service_callback: KeyboardClassServiceCallbackFn,
}

fn keyboard_state_to_input(state: &KeyboardState) -> KEYBOARD_INPUT_DATA {
    let mut input_data: KEYBOARD_INPUT_DATA = Default::default();
    input_data.MakeCode = state.scane_code;
    input_data.Flags = if state.down { KEYBOARD_FLAG_MAKE } else { KEYBOARD_FLAG_BREAK };
    input_data
}

impl KeyboardInput {
    pub fn send_input(&self, state: &[KeyboardState]) {
        let input_data = state.iter()
            .map(keyboard_state_to_input)
            .collect::<Vec<_>>();

        let mut consumed = 0;
        let input_ptr = input_data.as_ptr_range();
        (self.service_callback)(
            self.kb_device.cast(),
            input_ptr.start,
            input_ptr.end,
            &mut consumed
        );
    }
}

fn find_keyboard_service_callback() -> anyhow::Result<KeyboardClassServiceCallbackFn> {
    let module_kdbclass = KModule::find_by_name(obfstr!("kbdclass.sys"))?
        .with_context(|| anyhow!("failed to locate {} module", obfstr!("kbdclass.sys")))?;

    let pattern = ByteSequencePattern::parse(obfstr!("48 8D 05 ? ? ? ? 48 89 45"))
        .with_context(|| obfstr!("Failed to compile KeyboardClassServiceCallback pattern").to_string())?;

    NtOffsets::locate_function(
        &module_kdbclass, obfstr!("KeyboardClassServiceCallback"), 
        &pattern, 0x03, 0x07
    )
}

pub fn create_keyboard_input() -> anyhow::Result<KeyboardInput> {
    let name = UNICODE_STRING::from_bytes(obfstr::wide!("\\Driver\\KbdClass"));
    let kb_driver = Object::reference_by_name(&name, unsafe { *IoDriverObjectType })
        .map_err(|code| anyhow!("Object::reference_by_name 0x{:X}", code))?;
    let kb_driver = kb_driver.cast::<DRIVER_OBJECT>();

    /* To get all keyboard devices we could use kb_device.NextDevice. Currently we use the first one available. */
    let kb_device = unsafe { kb_driver.DeviceObject.as_mut() };
    let kb_device = match kb_device {
        Some(device) => Object::reference(device as *mut _ as PVOID),
        None => anyhow::bail!("no keyboard device detected")
    };

    let service_callback = find_keyboard_service_callback()?;
    // unsafe {
    //     let mut input_data: KEYBOARD_INPUT_DATA = Default::default();
    //     input_data.MakeCode = 0x1E;
    //     input_data.Flags = KEYBOARD_FLAG_MAKE;
    //     let mut consumed = 0;
    //     service_callback(
    //         kb_device,
    //         &input_data,
    //         (&input_data as *const KEYBOARD_INPUT_DATA).offset(1),
    //         &mut consumed
    //     );
    // }

    Ok(KeyboardInput {
        kb_device,
        service_callback,
    })
}