use std::path::{Path, PathBuf};
use std::process::Command;

/// The subcommand that runs an agent so that it ends with the app: `schrumbi agent-guard <app pid> <program> [args…]`.
pub const SUBCOMMAND: &str = "agent-guard";

/// The program and arguments that start `program` under the guard of this app process, so it and everything it starts end when the app ends, also when the app is killed.
pub fn wrap(program: &Path, args: &[String]) -> Option<(PathBuf, Vec<String>)> {
    let exe = super::detect::exe()?;
    let mut out = vec![SUBCOMMAND.to_string(), std::process::id().to_string(), program.to_string_lossy().to_string()];
    out.extend(args.iter().cloned());
    Some((exe, out))
}

/// Runs the guard: starts the agent on this process's standard streams and ends it with all its children once the app is gone.
pub fn main(args: Vec<String>) -> i32 {
    let mut parts = args.into_iter();
    let (Some(parent), Some(program)) = (parts.next().and_then(|p| p.parse::<u32>().ok()), parts.next()) else {
        eprintln!("usage: schrumbi {SUBCOMMAND} <app pid> <program> [args…]");
        return 2;
    };
    let rest: Vec<String> = parts.collect();
    os::contain();
    let mut command = Command::new(&program);
    command.args(&rest);
    os::prepare(&mut command);
    let mut child = match command.spawn() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{program}: {e}");
            return 127;
        }
    };
    os::watch(parent);
    let code = child.wait().ok().and_then(|s| s.code()).unwrap_or(1);
    os::finish();
    code
}

#[cfg(windows)]
mod os {
    use std::os::windows::process::CommandExt;
    use std::process::Command;

    use windows_sys::Win32::System::JobObjects::{AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE, JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation, SetInformationJobObject};
    use windows_sys::Win32::System::Threading::{CREATE_NO_WINDOW, GetCurrentProcess, INFINITE, OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject};

    pub fn contain() {
        unsafe {
            let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if job.is_null() {
                return;
            }
            let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            let size = u32::try_from(std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>()).unwrap_or(0);
            if SetInformationJobObject(job, JobObjectExtendedLimitInformation, std::ptr::from_ref(&info).cast(), size) != 0 {
                AssignProcessToJobObject(job, GetCurrentProcess());
            }
        }
    }

    pub fn prepare(command: &mut Command) {
        command.creation_flags(CREATE_NO_WINDOW);
    }

    pub fn watch(parent: u32) {
        let handle = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, parent) };
        if handle.is_null() {
            std::process::exit(1);
        }
        let handle = handle as usize;
        let _ = std::thread::Builder::new().name("guard".into()).spawn(move || {
            unsafe { WaitForSingleObject(handle as _, INFINITE) };
            std::process::exit(1);
        });
    }

    pub fn finish() {}
}

#[cfg(unix)]
mod os {
    use std::process::Command;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::Duration;

    static ORPHANED: AtomicBool = AtomicBool::new(false);

    extern "C" fn keep(_: libc::c_int) {}

    pub fn contain() {
        unsafe { libc::setpgid(0, 0) };
    }

    pub fn prepare(_: &mut Command) {}

    pub fn watch(parent: u32) {
        let first = unsafe { libc::getppid() };
        let expected = i32::try_from(parent).ok().filter(|p| *p == first).unwrap_or(first);
        unsafe { libc::signal(libc::SIGTERM, keep as extern "C" fn(libc::c_int) as libc::sighandler_t) };
        let _ = std::thread::Builder::new().name("guard".into()).spawn(move || {
            while unsafe { libc::getppid() } == expected {
                std::thread::sleep(Duration::from_millis(500));
            }
            ORPHANED.store(true, Ordering::SeqCst);
            unsafe { libc::kill(0, libc::SIGTERM) };
            std::thread::sleep(Duration::from_millis(1500));
            unsafe { libc::kill(0, libc::SIGKILL) };
        });
    }

    pub fn finish() {
        while ORPHANED.load(Ordering::SeqCst) {
            std::thread::sleep(Duration::from_millis(100));
        }
    }
}

#[cfg(not(any(windows, unix)))]
mod os {
    pub fn contain() {}
    pub fn prepare(_: &mut std::process::Command) {}
    pub fn watch(_: u32) {}
    pub fn finish() {}
}
