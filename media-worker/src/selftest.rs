use crate::protocol::{JobError, Meta, SelftestOp};
use crate::sandbox;

type JobResult = Result<(Meta, Vec<u8>), JobError>;

fn errno_result(rc: i64) -> JobResult {
    let value = if rc < 0 {
        -(std::io::Error::last_os_error().raw_os_error().unwrap_or(0) as i64)
    } else {
        rc
    };
    Ok((Meta::Selftest { value }, Vec::new()))
}

pub fn run(op: SelftestOp) -> JobResult {
    unsafe {
        match op {
            SelftestOp::OpenPasswd => {
                let path = c"/etc/passwd";
                let fd = libc::open(path.as_ptr(), libc::O_RDONLY);
                errno_result(fd as i64)
            }
            SelftestOp::Socket => {
                let fd = libc::socket(libc::AF_INET, libc::SOCK_STREAM, 0);
                errno_result(fd as i64)
            }
            SelftestOp::Exec => {
                let path = c"/bin/true";
                let argv = [path.as_ptr(), std::ptr::null()];
                let rc = libc::execv(path.as_ptr(), argv.as_ptr());
                errno_result(rc as i64)
            }
            SelftestOp::Fork => {
                let pid = libc::fork();
                if pid == 0 {
                    libc::_exit(0);
                }
                errno_result(pid as i64)
            }
            SelftestOp::MmapExec => {
                let p = libc::mmap(
                    std::ptr::null_mut(),
                    4096,
                    libc::PROT_READ | libc::PROT_EXEC,
                    libc::MAP_PRIVATE | libc::MAP_ANONYMOUS,
                    -1,
                    0,
                );
                errno_result(if p == libc::MAP_FAILED { -1 } else { 0 })
            }
            SelftestOp::MprotectExec => {
                let p = libc::mmap(
                    std::ptr::null_mut(),
                    4096,
                    libc::PROT_READ | libc::PROT_WRITE,
                    libc::MAP_PRIVATE | libc::MAP_ANONYMOUS,
                    -1,
                    0,
                );
                let rc = libc::mprotect(p, 4096, libc::PROT_READ | libc::PROT_EXEC);
                errno_result(rc as i64)
            }
            SelftestOp::Ptrace => {
                let rc = libc::ptrace(libc::PTRACE_TRACEME, 0, 0, 0);
                errno_result(rc)
            }
            SelftestOp::Mount => {
                let rc = libc::mount(
                    c"none".as_ptr(),
                    c"/".as_ptr(),
                    std::ptr::null(),
                    libc::MS_REMOUNT,
                    std::ptr::null(),
                );
                errno_result(rc as i64)
            }
            SelftestOp::Unshare => {
                let rc = libc::unshare(libc::CLONE_NEWUSER);
                errno_result(rc as i64)
            }
            SelftestOp::ReadInput => {
                let mut buf = vec![0u8; 8];
                let n = libc::read(3, buf.as_mut_ptr() as *mut _, buf.len());
                buf.truncate(n.max(0) as usize);
                Ok((Meta::Selftest { value: n as i64 }, buf))
            }
            SelftestOp::ListEnv => {
                let rc = libc::open(c"/proc/self/environ".as_ptr(), libc::O_RDONLY);
                errno_result(rc as i64)
            }
            SelftestOp::Spin => loop {
                std::hint::spin_loop();
            },
            SelftestOp::Segv => {
                std::ptr::write_volatile(std::ptr::null_mut::<u8>(), 1);
                errno_result(0)
            }
            SelftestOp::AllocHuge => {
                let mut v: Vec<u8> = Vec::new();
                for _ in 0..64 {
                    v.extend(std::iter::repeat_n(1u8, 64 * 1024 * 1024));
                }
                errno_result(v.len() as i64)
            }
            SelftestOp::SandboxState => Ok((
                Meta::Selftest {
                    value: sandbox::state(),
                },
                Vec::new(),
            )),
            SelftestOp::Garbage => {
                let junk = b"this is not a frame";
                libc::write(1, junk.as_ptr() as *const _, junk.len());
                libc::_exit(0);
            }
            SelftestOp::Thread => {
                let value = std::thread::spawn(|| 41 + 1).join().map(|v| v as i64).unwrap_or(-1);
                Ok((Meta::Selftest { value }, Vec::new()))
            }
        }
    }
}
