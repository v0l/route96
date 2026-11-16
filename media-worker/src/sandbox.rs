use seccompiler::{
    BpfProgram, SeccompAction, SeccompCmpArgLen, SeccompCmpOp, SeccompCondition, SeccompFilter,
    SeccompRule,
};
use std::collections::BTreeMap;
use std::convert::TryInto;
use std::sync::atomic::{AtomicBool, Ordering};

pub const STATE_NAMESPACES: i64 = 1;
pub const STATE_SECCOMP: i64 = 2;

static NAMESPACES_ENTERED: AtomicBool = AtomicBool::new(false);
static SECCOMP_ACTIVE: AtomicBool = AtomicBool::new(false);

pub fn state() -> i64 {
    let mut s = 0;
    if NAMESPACES_ENTERED.load(Ordering::Relaxed) {
        s |= STATE_NAMESPACES;
    }
    if SECCOMP_ACTIVE.load(Ordering::Relaxed) {
        s |= STATE_SECCOMP;
    }
    s
}

pub struct Config {
    pub timeout_secs: u64,
    pub memory_mb: u64,
    pub namespaces: bool,
}

pub fn enter(cfg: &Config) -> Result<(), String> {
    unsafe {
        libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL as libc::c_ulong, 0, 0, 0);
    }
    set_rlimit(libc::RLIMIT_CORE, 0, 0)?;
    if cfg.namespaces {
        match enter_namespaces() {
            Ok(()) => NAMESPACES_ENTERED.store(true, Ordering::Relaxed),
            Err(e) => eprintln!("route96-media: namespaces unavailable ({e}), using seccomp only"),
        }
    }
    unsafe {
        libc::prctl(libc::PR_SET_DUMPABLE, 0, 0, 0, 0);
    }
    close_inherited_fds();
    if unsafe { libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) } != 0 {
        return Err(format!("no_new_privs: {}", std::io::Error::last_os_error()));
    }
    drop_capabilities();
    let memory = cfg.memory_mb.saturating_mul(1024 * 1024);
    set_rlimit(libc::RLIMIT_AS, memory, memory)?;
    set_rlimit(libc::RLIMIT_CPU, cfg.timeout_secs, cfg.timeout_secs + 1)?;
    set_rlimit(libc::RLIMIT_FSIZE, 0, 0)?;
    set_rlimit(libc::RLIMIT_NOFILE, 16, 16)?;
    set_rlimit(libc::RLIMIT_MEMLOCK, 0, 0)?;
    install_seccomp()?;
    SECCOMP_ACTIVE.store(true, Ordering::Relaxed);
    Ok(())
}

fn set_rlimit(resource: libc::__rlimit_resource_t, soft: u64, hard: u64) -> Result<(), String> {
    let lim = libc::rlimit {
        rlim_cur: soft,
        rlim_max: hard,
    };
    if unsafe { libc::setrlimit(resource, &lim) } != 0 {
        return Err(format!("setrlimit: {}", std::io::Error::last_os_error()));
    }
    Ok(())
}

fn close_inherited_fds() {
    unsafe {
        libc::syscall(libc::SYS_close_range, 4u32, u32::MAX, 0u32);
    }
}

fn drop_capabilities() {
    unsafe {
        for cap in 0..64 {
            libc::prctl(libc::PR_CAPBSET_DROP, cap as libc::c_ulong, 0, 0, 0);
        }
        #[repr(C)]
        struct Header {
            version: u32,
            pid: i32,
        }
        #[repr(C)]
        #[derive(Clone, Copy)]
        struct Data {
            effective: u32,
            permitted: u32,
            inheritable: u32,
        }
        const LINUX_CAPABILITY_VERSION_3: u32 = 0x2008_0522;
        let header = Header {
            version: LINUX_CAPABILITY_VERSION_3,
            pid: 0,
        };
        let data = [Data {
            effective: 0,
            permitted: 0,
            inheritable: 0,
        }; 2];
        libc::syscall(libc::SYS_capset, &header, data.as_ptr());
    }
}

fn enter_namespaces() -> Result<(), String> {
    let uid = unsafe { libc::getuid() };
    let gid = unsafe { libc::getgid() };
    let flags = libc::CLONE_NEWUSER
        | libc::CLONE_NEWNS
        | libc::CLONE_NEWIPC
        | libc::CLONE_NEWNET
        | libc::CLONE_NEWUTS
        | libc::CLONE_NEWPID;
    if unsafe { libc::unshare(flags) } != 0 {
        return Err(format!("unshare: {}", std::io::Error::last_os_error()));
    }
    std::fs::write("/proc/self/setgroups", "deny").map_err(|e| format!("setgroups: {e}"))?;
    std::fs::write("/proc/self/uid_map", format!("65534 {uid} 1"))
        .map_err(|e| format!("uid_map: {e}"))?;
    std::fs::write("/proc/self/gid_map", format!("65534 {gid} 1"))
        .map_err(|e| format!("gid_map: {e}"))?;
    become_pid_one()?;
    if let Err(e) = empty_root() {
        eprintln!("route96-media: empty root failed: {e}");
    }
    Ok(())
}

fn become_pid_one() -> Result<(), String> {
    match unsafe { libc::fork() } {
        -1 => Err(format!("fork: {}", std::io::Error::last_os_error())),
        0 => {
            unsafe {
                libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL as libc::c_ulong, 0, 0, 0);
            }
            Ok(())
        }
        child => {
            let mut status = 0;
            unsafe {
                libc::waitpid(child, &mut status, 0);
                if libc::WIFSIGNALED(status) {
                    let sig = libc::WTERMSIG(status);
                    libc::signal(sig, libc::SIG_DFL);
                    libc::raise(sig);
                    libc::_exit(128 + sig);
                }
                libc::_exit(libc::WEXITSTATUS(status));
            }
        }
    }
}

fn mount(
    source: &str,
    target: &str,
    fstype: Option<&str>,
    flags: libc::c_ulong,
) -> Result<(), String> {
    let s = std::ffi::CString::new(source).unwrap();
    let t = std::ffi::CString::new(target).unwrap();
    let f = fstype.map(|f| std::ffi::CString::new(f).unwrap());
    let rc = unsafe {
        libc::mount(
            s.as_ptr(),
            t.as_ptr(),
            f.as_ref().map_or(std::ptr::null(), |f| f.as_ptr()),
            flags,
            std::ptr::null(),
        )
    };
    if rc != 0 {
        return Err(format!(
            "mount {target}: {}",
            std::io::Error::last_os_error()
        ));
    }
    Ok(())
}

fn empty_root() -> Result<(), String> {
    mount("none", "/", None, libc::MS_REC | libc::MS_PRIVATE)?;
    let new_root = ["/tmp", "/mnt", "/run", "/dev", "/proc", "/"]
        .into_iter()
        .find(|d| std::fs::metadata(d).map(|m| m.is_dir()).unwrap_or(false))
        .ok_or("no mount point")?;
    mount("tmpfs", new_root, Some("tmpfs"), libc::MS_NOSUID | libc::MS_NODEV)?;
    let old = format!("{}/.old", new_root.trim_end_matches('/'));
    std::fs::create_dir(&old).map_err(|e| format!("mkdir old root: {e}"))?;
    let n = std::ffi::CString::new(new_root).unwrap();
    let o = std::ffi::CString::new(old.clone()).unwrap();
    if unsafe { libc::syscall(libc::SYS_pivot_root, n.as_ptr(), o.as_ptr()) } != 0 {
        return Err(format!("pivot_root: {}", std::io::Error::last_os_error()));
    }
    std::env::set_current_dir("/").map_err(|e| format!("chdir: {e}"))?;
    let detached = std::ffi::CString::new("/.old").unwrap();
    if unsafe { libc::umount2(detached.as_ptr(), libc::MNT_DETACH) } != 0 {
        return Err(format!("umount old root: {}", std::io::Error::last_os_error()));
    }
    let _ = std::fs::remove_dir("/.old");
    mount(
        "none",
        "/",
        None,
        libc::MS_REMOUNT | libc::MS_BIND | libc::MS_RDONLY | libc::MS_NOSUID | libc::MS_NODEV,
    )
}

type Rules = BTreeMap<i64, Vec<SeccompRule>>;

fn cond(arg: u8, len: SeccompCmpArgLen, op: SeccompCmpOp, value: u64) -> SeccompCondition {
    SeccompCondition::new(arg, len, op, value).expect("valid seccomp condition")
}

fn rule(conditions: Vec<SeccompCondition>) -> Vec<SeccompRule> {
    vec![SeccompRule::new(conditions).expect("valid seccomp rule")]
}

fn without_flag(arg: u8, flag: u64) -> SeccompCondition {
    cond(
        arg,
        SeccompCmpArgLen::Dword,
        SeccompCmpOp::MaskedEq(flag),
        0,
    )
}

const CLONE_NEW_MASK: u64 = (libc::CLONE_NEWNS
    | libc::CLONE_NEWCGROUP
    | libc::CLONE_NEWUTS
    | libc::CLONE_NEWIPC
    | libc::CLONE_NEWUSER
    | libc::CLONE_NEWPID
    | libc::CLONE_NEWNET) as u64;

pub fn allowed_syscalls() -> Vec<(&'static str, i64)> {
    let mut list = vec![
        ("read", libc::SYS_read),
        ("write", libc::SYS_write),
        ("readv", libc::SYS_readv),
        ("writev", libc::SYS_writev),
        ("pread64", libc::SYS_pread64),
        ("lseek", libc::SYS_lseek),
        ("close", libc::SYS_close),
        ("dup", libc::SYS_dup),
        ("dup3", libc::SYS_dup3),
        ("fstat", libc::SYS_fstat),
        ("newfstatat", libc::SYS_newfstatat),
        ("statx", libc::SYS_statx),
        ("fcntl", libc::SYS_fcntl),
        ("mmap", libc::SYS_mmap),
        ("mprotect", libc::SYS_mprotect),
        ("munmap", libc::SYS_munmap),
        ("mremap", libc::SYS_mremap),
        ("brk", libc::SYS_brk),
        ("madvise", libc::SYS_madvise),
        ("futex", libc::SYS_futex),
        ("set_robust_list", libc::SYS_set_robust_list),
        ("rseq", libc::SYS_rseq),
        ("sched_yield", libc::SYS_sched_yield),
        ("sched_getaffinity", libc::SYS_sched_getaffinity),
        ("nanosleep", libc::SYS_nanosleep),
        ("clock_nanosleep", libc::SYS_clock_nanosleep),
        ("clock_gettime", libc::SYS_clock_gettime),
        ("clock_getres", libc::SYS_clock_getres),
        ("gettimeofday", libc::SYS_gettimeofday),
        ("getrandom", libc::SYS_getrandom),
        ("rt_sigaction", libc::SYS_rt_sigaction),
        ("rt_sigprocmask", libc::SYS_rt_sigprocmask),
        ("rt_sigreturn", libc::SYS_rt_sigreturn),
        ("sigaltstack", libc::SYS_sigaltstack),
        ("exit", libc::SYS_exit),
        ("exit_group", libc::SYS_exit_group),
        ("getpid", libc::SYS_getpid),
        ("gettid", libc::SYS_gettid),
        ("sysinfo", libc::SYS_sysinfo),
        ("prlimit64", libc::SYS_prlimit64),
        ("prctl", libc::SYS_prctl),
        ("clone", libc::SYS_clone),
        ("clone3", libc::SYS_clone3),
    ];
    list.extend(path_probing_syscalls());
    list
}

#[cfg(target_arch = "x86_64")]
fn path_probing_syscalls() -> Vec<(&'static str, i64)> {
    vec![
        ("open", libc::SYS_open),
        ("openat", libc::SYS_openat),
        ("openat2", libc::SYS_openat2),
        ("access", libc::SYS_access),
        ("faccessat", libc::SYS_faccessat),
        ("faccessat2", libc::SYS_faccessat2),
        ("readlink", libc::SYS_readlink),
        ("readlinkat", libc::SYS_readlinkat),
        ("stat", libc::SYS_stat),
        ("lstat", libc::SYS_lstat),
    ]
}

#[cfg(not(target_arch = "x86_64"))]
fn path_probing_syscalls() -> Vec<(&'static str, i64)> {
    vec![
        ("openat", libc::SYS_openat),
        ("openat2", libc::SYS_openat2),
        ("faccessat", libc::SYS_faccessat),
        ("faccessat2", libc::SYS_faccessat2),
        ("readlinkat", libc::SYS_readlinkat),
    ]
}

fn allow_rules() -> Rules {
    let mut rules: Rules = allowed_syscalls()
        .into_iter()
        .map(|(_, nr)| (nr, vec![]))
        .collect();
    let at_empty_path = libc::AT_EMPTY_PATH as u64;
    let _ = at_empty_path;
    let no_exec = |arg| rule(vec![without_flag(arg, libc::PROT_EXEC as u64)]);
    rules.insert(libc::SYS_mmap, no_exec(2));
    rules.insert(libc::SYS_mprotect, no_exec(2));
    rules.insert(
        libc::SYS_clone,
        rule(vec![
            cond(
                0,
                SeccompCmpArgLen::Qword,
                SeccompCmpOp::MaskedEq(libc::CLONE_THREAD as u64),
                libc::CLONE_THREAD as u64,
            ),
            cond(
                0,
                SeccompCmpArgLen::Qword,
                SeccompCmpOp::MaskedEq(CLONE_NEW_MASK),
                0,
            ),
        ]),
    );
    rules.insert(
        libc::SYS_prctl,
        rule(vec![cond(
            0,
            SeccompCmpArgLen::Dword,
            SeccompCmpOp::Eq,
            libc::PR_SET_NAME as u64,
        )]),
    );
    rules.insert(
        libc::SYS_prlimit64,
        rule(vec![
            cond(0, SeccompCmpArgLen::Dword, SeccompCmpOp::Eq, 0),
            cond(2, SeccompCmpArgLen::Qword, SeccompCmpOp::Eq, 0),
        ]),
    );
    rules
}

fn deny_with_eperm_rules() -> Rules {
    let mut rules: Rules = path_probing_syscalls()
        .into_iter()
        .map(|(_, nr)| (nr, vec![]))
        .collect();
    rules.insert(
        libc::SYS_newfstatat,
        rule(vec![without_flag(3, libc::AT_EMPTY_PATH as u64)]),
    );
    rules.insert(
        libc::SYS_statx,
        rule(vec![without_flag(2, libc::AT_EMPTY_PATH as u64)]),
    );
    rules
}

fn compile(
    rules: Rules,
    mismatch: SeccompAction,
    matched: SeccompAction,
) -> Result<BpfProgram, String> {
    let arch = std::env::consts::ARCH
        .try_into()
        .map_err(|e| format!("seccomp arch: {e:?}"))?;
    SeccompFilter::new(rules, mismatch, matched, arch)
        .map_err(|e| format!("seccomp filter: {e}"))?
        .try_into()
        .map_err(|e| format!("seccomp compile: {e}"))
}

pub fn install_seccomp() -> Result<(), String> {
    let eperm = compile(
        deny_with_eperm_rules(),
        SeccompAction::Allow,
        SeccompAction::Errno(libc::EPERM as u32),
    )?;
    let no_clone3 = compile(
        BTreeMap::from([(libc::SYS_clone3, vec![])]),
        SeccompAction::Allow,
        SeccompAction::Errno(libc::ENOSYS as u32),
    )?;
    let no_tty = compile(
        BTreeMap::from([(libc::SYS_ioctl, vec![])]),
        SeccompAction::Allow,
        SeccompAction::Errno(libc::ENOTTY as u32),
    )?;
    let mut allowlist_rules = allow_rules();
    allowlist_rules.insert(libc::SYS_ioctl, vec![]);
    let allowlist = compile(allowlist_rules, SeccompAction::KillProcess, SeccompAction::Allow)?;
    for program in [&eperm, &no_clone3, &no_tty, &allowlist] {
        seccompiler::apply_filter_all_threads(program).map_err(|e| format!("seccomp: {e}"))?;
    }
    Ok(())
}
