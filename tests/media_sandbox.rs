#![cfg(all(target_os = "linux", feature = "media-compression"))]

use route96::media::protocol::{Job, Meta, SelftestOp};
use route96::media::{MediaError, MediaSandbox, RejectReason};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Once;
use std::time::{Duration, Instant};

fn worker() -> PathBuf {
    static BUILD: Once = Once::new();
    BUILD.call_once(|| {
        let status = Command::new(env!("CARGO"))
            .args(["build", "-p", "route96-media"])
            .current_dir(env!("CARGO_MANIFEST_DIR"))
            .status()
            .expect("cargo build worker");
        assert!(status.success());
    });
    let exe = std::env::current_exe().unwrap();
    exe.ancestors()
        .skip(1)
        .take(2)
        .map(|d| d.join("route96-media"))
        .find(|p| p.is_file())
        .expect("worker binary")
}

fn sandbox() -> MediaSandbox {
    MediaSandbox::from_config()
        .with_worker(worker())
        .with_selftest(true)
        .with_timeout(Duration::from_secs(20))
}

fn corpus(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("test-data/corpus")
        .join(name)
}

fn selftest(sb: &MediaSandbox, op: SelftestOp) -> Result<i64, MediaError> {
    match sb.run(&corpus("a.jpg"), Job::Selftest { op })? {
        (Meta::Selftest { value }, _) => Ok(value),
        (other, _) => panic!("unexpected {other:?}"),
    }
}

fn signal(r: Result<i64, MediaError>) -> i32 {
    match r {
        Err(MediaError::Rejected(RejectReason::Signal(s))) => s,
        other => panic!("expected kill by signal, got {other:?}"),
    }
}

const SIGSYS: i32 = 31;

fn both_modes() -> Vec<MediaSandbox> {
    vec![sandbox(), sandbox().with_namespaces(false)]
}

#[test]
fn open_is_denied_with_eperm() {
    for sb in both_modes() {
        assert_eq!(selftest(&sb, SelftestOp::OpenPasswd).unwrap(), -(libc::EPERM as i64));
        assert_eq!(selftest(&sb, SelftestOp::ListEnv).unwrap(), -(libc::EPERM as i64));
    }
}

#[test]
fn socket_is_killed() {
    for sb in both_modes() {
        assert_eq!(signal(selftest(&sb, SelftestOp::Socket)), SIGSYS);
    }
}

#[test]
fn execve_is_killed() {
    for sb in both_modes() {
        assert_eq!(signal(selftest(&sb, SelftestOp::Exec)), SIGSYS);
    }
}

#[test]
fn fork_is_killed() {
    for sb in both_modes() {
        assert_eq!(signal(selftest(&sb, SelftestOp::Fork)), SIGSYS);
    }
}

#[test]
fn executable_mappings_are_killed() {
    for sb in both_modes() {
        assert_eq!(signal(selftest(&sb, SelftestOp::MmapExec)), SIGSYS);
        assert_eq!(signal(selftest(&sb, SelftestOp::MprotectExec)), SIGSYS);
    }
}

#[test]
fn ptrace_mount_and_unshare_are_killed() {
    for sb in both_modes() {
        assert_eq!(signal(selftest(&sb, SelftestOp::Ptrace)), SIGSYS);
        assert_eq!(signal(selftest(&sb, SelftestOp::Mount)), SIGSYS);
        assert_eq!(signal(selftest(&sb, SelftestOp::Unshare)), SIGSYS);
    }
}

#[test]
fn input_fd_is_readable_and_threads_work() {
    let sb = sandbox();
    let (meta, bytes) = sb
        .run(&corpus("a.jpg"), Job::Selftest { op: SelftestOp::ReadInput })
        .unwrap();
    assert_eq!(meta, Meta::Selftest { value: 8 });
    assert_eq!(&bytes[..2], &[0xff, 0xd8]);
    assert_eq!(selftest(&sb, SelftestOp::Thread).unwrap(), 42);
}

#[test]
fn seccomp_is_active_and_namespaces_are_optional() {
    let seccomp_only = selftest(&sandbox().with_namespaces(false), SelftestOp::SandboxState).unwrap();
    assert_eq!(seccomp_only & 2, 2);
    assert_eq!(seccomp_only & 1, 0);
}

#[test]
fn selftest_is_disabled_by_default() {
    let sb = sandbox().with_selftest(false);
    assert!(matches!(
        selftest(&sb, SelftestOp::OpenPasswd),
        Err(MediaError::Job(_))
    ));
}

#[test]
fn spinning_worker_times_out_and_server_survives() {
    let sb = sandbox().with_timeout(Duration::from_secs(2));
    let start = Instant::now();
    let r = selftest(&sb, SelftestOp::Spin);
    assert!(matches!(r, Err(MediaError::Rejected(_))), "{r:?}");
    assert!(start.elapsed() < Duration::from_secs(10));
    assert!(selftest(&sb, SelftestOp::SandboxState).is_ok());
}

#[test]
fn crashing_worker_is_rejected() {
    let r = selftest(&sandbox(), SelftestOp::Segv);
    assert_eq!(signal(r), libc::SIGSEGV);
}

#[test]
fn memory_hog_is_rejected() {
    let sb = sandbox().with_memory_mb(256);
    assert!(matches!(
        selftest(&sb, SelftestOp::AllocHuge),
        Err(MediaError::Rejected(_))
    ));
}

#[test]
fn garbage_response_is_rejected() {
    assert!(matches!(
        selftest(&sandbox(), SelftestOp::Garbage),
        Err(MediaError::Rejected(RejectReason::Malformed(_)))
    ));
}

#[test]
fn missing_worker_is_unavailable_not_a_panic() {
    let sb = sandbox().with_worker("/nonexistent/route96-media");
    assert!(matches!(
        selftest(&sb, SelftestOp::SandboxState),
        Err(MediaError::Unavailable(_))
    ));
}

#[test]
fn hostile_manifests_fail_cleanly() {
    let sb = sandbox();
    let dir = tempfile::tempdir().unwrap();
    for (name, body) in [
        ("p.m3u8", "#EXTM3U\n#EXT-X-TARGETDURATION:1\n#EXTINF:1,\nfile:/etc/hostname\n#EXT-X-ENDLIST\n"),
        ("c.txt", "ffconcat version 1.0\nfile /etc/hostname\n"),
    ] {
        let p = dir.path().join(name);
        std::fs::write(&p, body).unwrap();
        for job in [Job::Probe, Job::Thumbnail, Job::Rgb] {
            let r = sb.run(&p, job);
            assert!(matches!(r, Err(MediaError::Job(_))), "{name}: {r:?}");
        }
    }
    for name in ["random.bin", "truncated.jpg"] {
        let r = sb.run(&corpus(name), Job::Probe);
        assert!(!matches!(r, Err(MediaError::Rejected(_))), "{name}: {r:?}");
    }
}

#[test]
fn server_source_does_not_touch_libav() {
    fn scan(dir: &Path, hits: &mut Vec<PathBuf>) {
        for e in std::fs::read_dir(dir).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                scan(&p, hits);
            } else if p.extension().is_some_and(|x| x == "rs") {
                let text = std::fs::read_to_string(&p).unwrap();
                if text.contains("ffmpeg_rs_raw") || text.contains("ffmpeg_sys") {
                    hits.push(p);
                }
            }
        }
    }
    let mut hits = vec![];
    scan(&Path::new(env!("CARGO_MANIFEST_DIR")).join("src"), &mut hits);
    assert!(hits.is_empty(), "libav referenced outside the worker: {hits:?}");
    let manifest = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml")).unwrap();
    assert!(!manifest.contains("ffmpeg-rs-raw"));
}

fn golden() -> Vec<Vec<String>> {
    let text = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("test-data/golden.tsv")).unwrap();
    text.lines().map(|l| l.split('\t').map(str::to_string).collect()).collect()
}

fn sha(b: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    hex::encode(Sha256::digest(b))
}

fn resolve(name: &str) -> PathBuf {
    let c = corpus(name);
    if c.exists() {
        c
    } else {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("test-data").join(name)
    }
}

#[test]
fn worker_outputs_match_the_in_process_golden_corpus() {
    let sb = sandbox();
    for row in golden() {
        let (name, mime) = (&row[0], &row[1]);
        let path = resolve(name);

        let probe = sb.run(&path, Job::Probe);
        if row[2] == "ERR" {
            assert!(probe.is_err(), "{name}");
        } else {
            let Ok((Meta::Probe(p), _)) = probe else { panic!("{name}: {probe:?}") };
            let bv = p.best_video.as_ref();
            let got = format!(
                "{}|{}|{}|{}x{}|{}",
                p.format,
                p.duration,
                p.bitrate,
                bv.map(|v| v.width).unwrap_or(0),
                bv.map(|v| v.height).unwrap_or(0),
                p.stream_count
            );
            assert_eq!(got, row[2], "probe {name}");
        }

        if row[3] != "NA" {
            let r = sb.run(&path, Job::Compress { mime_type: mime.clone() });
            if row[3] == "ERR" {
                assert!(r.is_err(), "compress {name}");
            } else {
                let (Meta::Compressed { width, height, .. }, bytes) = r.unwrap() else { panic!() };
                assert_eq!(format!("image/webp|{width}x{height}|{}", sha(&bytes)), row[3], "compress {name}");
            }
        }

        let thumb = sb.run(&path, Job::Thumbnail);
        match (&thumb, row[4].as_str()) {
            (Err(_), "ERR") => {}
            (Ok((_, bytes)), want) => assert_eq!(sha(bytes), want, "thumb {name}"),
            (Err(e), want) => panic!("thumb {name}: {e} want {want}"),
        }

        if row[5] != "NA" {
            match sb.run(&path, Job::Rgb) {
                Ok((Meta::Image { width, height }, px)) => {
                    let buf = image::ImageBuffer::<image::Rgb<u8>, Vec<u8>>::from_raw(width, height, px).unwrap();
                    let h = route96::phash::make_hasher().hash_image(&buf);
                    assert_eq!(hex::encode(h.as_bytes()), row[5], "phash {name}");
                }
                other => assert_eq!(row[5], "ERR", "{name}: {other:?}"),
            }
        }
    }
}
