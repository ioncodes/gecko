use super::{Registers, handle};
use std::sync::OnceLock;
use windows_sys::Win32::Foundation::EXCEPTION_ACCESS_VIOLATION;
use windows_sys::Win32::System::Diagnostics::Debug::*;
use windows_sys::Win32::System::SystemServices::EXCEPTION_EXECUTE_FAULT;

#[cfg(target_arch = "aarch64")]
use super::ARM64_SP;

const CALL_FIRST: u32 = 1;
const ACCESS_VIOLATION_PARAMETERS: u32 = 2;

pub(super) fn install() -> bool {
    static INSTALLED: OnceLock<bool> = OnceLock::new();

    *INSTALLED.get_or_init(|| unsafe { !AddVectoredExceptionHandler(CALL_FIRST, Some(handler)).is_null() })
}

unsafe extern "system" fn handler(exception: *mut EXCEPTION_POINTERS) -> i32 {
    let exception = unsafe { &mut *exception };
    let record = unsafe { &*exception.ExceptionRecord };
    if record.ExceptionCode != EXCEPTION_ACCESS_VIOLATION || record.NumberParameters < ACCESS_VIOLATION_PARAMETERS {
        return EXCEPTION_CONTINUE_SEARCH;
    }

    let [kind, fault_address, ..] = record.ExceptionInformation;
    if kind == EXCEPTION_EXECUTE_FAULT as usize {
        return EXCEPTION_CONTINUE_SEARCH;
    }

    let context = unsafe { &mut *exception.ContextRecord };
    let mut registers = Registers::default();

    #[cfg(target_arch = "x86_64")]
    {
        registers.pc = context.Rip;
        for (value, slot) in registers.gpr.iter_mut().zip(gprs(context)) {
            *value = *slot;
        }
    }
    #[cfg(target_arch = "aarch64")]
    {
        registers.pc = context.Pc;
        registers.gpr[..ARM64_SP].copy_from_slice(unsafe { &context.Anonymous.X });
        registers.gpr[ARM64_SP] = context.Sp;
    }

    if !handle(fault_address, &mut registers) {
        return EXCEPTION_CONTINUE_SEARCH;
    }

    #[cfg(target_arch = "x86_64")]
    {
        context.Rip = registers.pc;
        for (value, slot) in registers.gpr.iter().zip(gprs(context)) {
            *slot = *value;
        }
    }
    #[cfg(target_arch = "aarch64")]
    {
        context.Pc = registers.pc;
        context.Anonymous.X.copy_from_slice(&registers.gpr[..ARM64_SP]);
        context.Sp = registers.gpr[ARM64_SP];
    }

    EXCEPTION_CONTINUE_EXECUTION
}

#[cfg(target_arch = "x86_64")]
fn gprs(context: &mut CONTEXT) -> [&mut u64; 16] {
    [
        &mut context.Rax,
        &mut context.Rcx,
        &mut context.Rdx,
        &mut context.Rbx,
        &mut context.Rsp,
        &mut context.Rbp,
        &mut context.Rsi,
        &mut context.Rdi,
        &mut context.R8,
        &mut context.R9,
        &mut context.R10,
        &mut context.R11,
        &mut context.R12,
        &mut context.R13,
        &mut context.R14,
        &mut context.R15,
    ]
}
