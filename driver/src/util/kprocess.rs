use alloc::vec::Vec;

use kapi::Process;

use crate::offsets::get_nt_offsets;

pub fn iter(mut consumer: impl FnMut(&Process)) {
    #[allow(non_snake_case)]
    let PsGetNextProcess = get_nt_offsets().PsGetNextProcess;

    let mut current_peprocess = core::ptr::null_mut();
    loop {
        current_peprocess = unsafe { PsGetNextProcess(current_peprocess) };
        if current_peprocess.is_null() {
            break;
        }

        let process = Process::from_raw(current_peprocess, false);

        // let active_threads = unsafe {
        //     current_peprocess
        //         /* The ActiveThreads comes after the thread list head. Thread list head has a size of 0x10. */
        //         .byte_offset(EPROCESS_ThreadListHead as isize + 0x10)
        //         .cast::<u32>()
        //         .read_volatile()
        // };

        consumer(&process);
    }
}

pub fn find_processes_by_name(target_name: &str) -> anyhow::Result<Vec<Process>> {
    #[allow(non_snake_case)]
    let PsGetNextProcess = get_nt_offsets().PsGetNextProcess;

    #[allow(non_snake_case)]
    let EPROCESS_ThreadListHead = get_nt_offsets().EPROCESS_ThreadListHead;

    let mut cs2_candidates = Vec::with_capacity(8);

    let mut current_peprocess = core::ptr::null_mut();
    loop {
        current_peprocess = unsafe { PsGetNextProcess(current_peprocess) };
        if current_peprocess.is_null() {
            break;
        }

        let process = Process::from_raw(current_peprocess, false);
        let image_file_name = process.get_image_file_name();

        if image_file_name != Some(target_name) {
            continue;
        }

        let active_threads = unsafe {
            current_peprocess
                /* The ActiveThreads comes after the thread list head. Thread list head has a size of 0x10. */
                .byte_offset(EPROCESS_ThreadListHead as isize + 0x10)
                .cast::<u32>()
                .read_volatile()
        };

        log::trace!(
            "{} matched {:X}: {:?} ({})",
            target_name,
            current_peprocess as u64,
            image_file_name,
            active_threads
        );
        if active_threads == 0 {
            /* Process terminated / not running */
            continue;
        }

        cs2_candidates.push(Process::from_raw(current_peprocess, false));
    }

    Ok(cs2_candidates)
}
