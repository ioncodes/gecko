use super::{Registers, handle};
use std::ffi::c_void;
use std::ptr;
use std::sync::OnceLock;

static PREVIOUS: OnceLock<[libc::sigaction; 2]> = OnceLock::new();

struct SignalStack {
    address: *mut c_void,
    len: usize,
    previous: libc::stack_t,
}

impl Drop for SignalStack {
    fn drop(&mut self) {
        unsafe {
            let mut current: libc::stack_t = std::mem::zeroed();
            if libc::sigaltstack(ptr::null(), &mut current) != 0 {
                return;
            }
            if current.ss_sp == self.address {
                if libc::sigaltstack(&self.previous, ptr::null_mut()) != 0 {
                    return;
                }
            }
            libc::munmap(self.address, self.len);
        }
    }
}

pub(super) fn prepare_thread() -> bool {
    thread_local! {
        static STACK: std::cell::OnceCell<Option<SignalStack>> = const { std::cell::OnceCell::new() };
    }

    STACK.with(|stack| {
        stack
            .get_or_init(|| unsafe {
                let mut previous: libc::stack_t = std::mem::zeroed();
                if libc::sigaltstack(ptr::null(), &mut previous) != 0 || previous.ss_flags & libc::SS_ONSTACK != 0 {
                    return None;
                }

                let len = 16 << 20;
                let address = libc::mmap(
                    ptr::null_mut(),
                    len,
                    libc::PROT_READ | libc::PROT_WRITE,
                    libc::MAP_PRIVATE | libc::MAP_ANON,
                    -1,
                    0,
                );
                if address == libc::MAP_FAILED {
                    return None;
                }

                let new = libc::stack_t {
                    ss_sp: address,
                    ss_flags: 0,
                    ss_size: len,
                };
                if libc::sigaltstack(&new, ptr::null_mut()) != 0 {
                    libc::munmap(address, len);
                    return None;
                }

                Some(SignalStack { address, len, previous })
            })
            .is_some()
    })
}

pub(super) fn install() -> bool {
    static INSTALLED: OnceLock<bool> = OnceLock::new();

    *INSTALLED.get_or_init(|| unsafe {
        let mut old: [libc::sigaction; 2] = std::mem::zeroed();
        if libc::sigaction(libc::SIGSEGV, ptr::null(), &mut old[0]) != 0
            || libc::sigaction(libc::SIGBUS, ptr::null(), &mut old[1]) != 0
        {
            return false;
        }
        PREVIOUS.set(old).ok();

        let mut action: libc::sigaction = std::mem::zeroed();
        action.sa_sigaction = handler as *const () as usize;
        action.sa_flags = libc::SA_SIGINFO | libc::SA_ONSTACK;

        libc::sigemptyset(&mut action.sa_mask);
        if libc::sigaction(libc::SIGSEGV, &action, ptr::null_mut()) != 0 {
            return false;
        }

        if libc::sigaction(libc::SIGBUS, &action, ptr::null_mut()) != 0 {
            libc::sigaction(libc::SIGSEGV, &PREVIOUS.get().unwrap()[0], ptr::null_mut());
            return false;
        }

        true
    })
}

unsafe extern "C" fn handler(signal: i32, info: *mut libc::siginfo_t, context: *mut c_void) {
    let mut registers = unsafe { read_context(context) };
    if handle(unsafe { (*info).si_addr() } as usize, &mut registers) {
        unsafe {
            write_context(context, &registers);
        }
        return;
    }

    unsafe {
        chain(signal, info, context);
    }
}

unsafe fn chain(signal: i32, info: *mut libc::siginfo_t, context: *mut c_void) {
    use std::sync::atomic::{AtomicBool, Ordering};

    static RESET: [AtomicBool; 2] = [AtomicBool::new(false), AtomicBool::new(false)];

    let index = usize::from(signal == libc::SIGBUS);
    let old = &PREVIOUS.get().unwrap()[index];
    if old.sa_sigaction == libc::SIG_IGN {
        return;
    }

    let reset = old.sa_flags & libc::SA_RESETHAND != 0 && RESET[index].swap(true, Ordering::Relaxed);
    if old.sa_sigaction == libc::SIG_DFL || reset {
        unsafe {
            libc::signal(signal, libc::SIG_DFL);
            libc::raise(signal);
        }
        return;
    }

    unsafe {
        let mut prior_mask: libc::sigset_t = std::mem::zeroed();
        libc::sigprocmask(libc::SIG_BLOCK, &old.sa_mask, &mut prior_mask);
        if old.sa_flags & libc::SA_NODEFER != 0 && libc::sigismember(&old.sa_mask, signal) == 0 {
            let mut unblocked: libc::sigset_t = std::mem::zeroed();
            libc::sigemptyset(&mut unblocked);
            libc::sigaddset(&mut unblocked, signal);
            libc::sigprocmask(libc::SIG_UNBLOCK, &unblocked, ptr::null_mut());
        }

        if old.sa_flags & libc::SA_SIGINFO != 0 {
            let previous: unsafe extern "C" fn(i32, *mut libc::siginfo_t, *mut c_void) =
                std::mem::transmute(old.sa_sigaction);
            previous(signal, info, context);
        } else {
            let previous: unsafe extern "C" fn(i32) = std::mem::transmute(old.sa_sigaction);
            previous(signal);
        }

        libc::sigprocmask(libc::SIG_SETMASK, &prior_mask, ptr::null_mut());
    }
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
const GPRS: [usize; 16] = [
    libc::REG_RAX as usize,
    libc::REG_RCX as usize,
    libc::REG_RDX as usize,
    libc::REG_RBX as usize,
    libc::REG_RSP as usize,
    libc::REG_RBP as usize,
    libc::REG_RSI as usize,
    libc::REG_RDI as usize,
    libc::REG_R8 as usize,
    libc::REG_R9 as usize,
    libc::REG_R10 as usize,
    libc::REG_R11 as usize,
    libc::REG_R12 as usize,
    libc::REG_R13 as usize,
    libc::REG_R14 as usize,
    libc::REG_R15 as usize,
];

#[cfg(target_os = "linux")]
unsafe fn read_context(context: *mut c_void) -> Registers {
    let context = unsafe { &(*context.cast::<libc::ucontext_t>()).uc_mcontext };
    let mut result = Registers::default();

    #[cfg(target_arch = "x86_64")]
    {
        result.pc = context.gregs[libc::REG_RIP as usize] as u64;
        for (i, reg) in GPRS.iter().enumerate() {
            result.gpr[i] = context.gregs[*reg] as u64;
        }
    }

    #[cfg(target_arch = "aarch64")]
    {
        result.pc = context.pc;
        result.gpr[..31].copy_from_slice(&context.regs);
        result.gpr[31] = context.sp;
    }

    result
}

#[cfg(target_os = "linux")]
unsafe fn write_context(context: *mut c_void, registers: &Registers) {
    let context = unsafe { &mut (*context.cast::<libc::ucontext_t>()).uc_mcontext };
    #[cfg(target_arch = "x86_64")]
    {
        context.gregs[libc::REG_RIP as usize] = registers.pc as libc::greg_t;
        for (i, reg) in GPRS.iter().enumerate() {
            context.gregs[*reg] = registers.gpr[i] as libc::greg_t;
        }
    }
    #[cfg(target_arch = "aarch64")]
    {
        context.pc = registers.pc;
        context.regs.copy_from_slice(&registers.gpr[..31]);
        context.sp = registers.gpr[31];
    }
}

#[cfg(all(target_os = "macos", target_arch = "x86_64"))]
fn gprs(context: &mut libc::__darwin_x86_thread_state64) -> [&mut u64; 16] {
    [
        &mut context.__rax,
        &mut context.__rcx,
        &mut context.__rdx,
        &mut context.__rbx,
        &mut context.__rsp,
        &mut context.__rbp,
        &mut context.__rsi,
        &mut context.__rdi,
        &mut context.__r8,
        &mut context.__r9,
        &mut context.__r10,
        &mut context.__r11,
        &mut context.__r12,
        &mut context.__r13,
        &mut context.__r14,
        &mut context.__r15,
    ]
}

#[cfg(all(target_os = "macos", target_arch = "x86_64"))]
unsafe fn read_context(context: *mut c_void) -> Registers {
    let context = unsafe { &mut (*(*context.cast::<libc::ucontext_t>()).uc_mcontext).__ss };
    let mut result = Registers {
        pc: context.__rip,
        ..Registers::default()
    };
    for (value, slot) in result.gpr.iter_mut().zip(gprs(context)) {
        *value = *slot;
    }
    result
}

#[cfg(all(target_os = "macos", target_arch = "x86_64"))]
unsafe fn write_context(context: *mut c_void, registers: &Registers) {
    let context = unsafe { &mut (*(*context.cast::<libc::ucontext_t>()).uc_mcontext).__ss };
    context.__rip = registers.pc;
    for (value, slot) in registers.gpr.iter().zip(gprs(context)) {
        *slot = *value;
    }
}

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
#[repr(C)]
struct DarwinContext {
    onstack: i32,
    mask: libc::sigset_t,
    stack: libc::stack_t,
    link: *mut c_void,
    size: usize,
    machine: *mut DarwinMachine,
}

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
#[repr(C)]
struct DarwinMachine {
    fault_address: u64,
    syndrome: u32,
    exception: u32,
    gpr: [u64; 32],
    pc: u64,
    cpsr: u32,
    flags: u32,
}

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
unsafe fn read_context(context: *mut c_void) -> Registers {
    let machine = unsafe { &*(*context.cast::<DarwinContext>()).machine };
    Registers {
        pc: machine.pc,
        gpr: machine.gpr,
    }
}

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
unsafe fn write_context(context: *mut c_void, registers: &Registers) {
    let machine = unsafe { &mut *(*context.cast::<DarwinContext>()).machine };
    machine.pc = registers.pc;
    machine.gpr = registers.gpr;
}
