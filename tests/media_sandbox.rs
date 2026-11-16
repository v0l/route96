#![cfg(all(target_os = "linux", feature = "media-compression"))]

use route96::media::protocol::{Job, Meta, SelftestOp};
use route96::media::{MediaError, MediaSandbox, RejectReason};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Once;
use std::time::{Duration, Instant};

fn worker() -> PathBuf {
    if let Some(p) = std::env::var_os("ROUTE96_MEDIA_BIN") {
        return PathBuf::from(p);
    }
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

#[test]
#[ignore = "measurement, run with --release worker and --nocapture"]
fn overhead_per_job() {
    let sb = sandbox();
    let mut files: Vec<PathBuf> = std::fs::read_dir(Path::new(env!("CARGO_MANIFEST_DIR")).join("test-data/corpus"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    files.push(resolve("sample.mp4"));
    files.push(resolve("glass_sculpture.webp"));
    let mut samples = Vec::new();
    while samples.len() < 50 {
        for f in &files {
            if samples.len() == 50 {
                break;
            }
            let t = Instant::now();
            let _ = sb.run(f, Job::Probe);
            samples.push(t.elapsed());
        }
    }
    let idle = {
        let t = Instant::now();
        for _ in 0..50 {
            let _ = selftest(&sb, SelftestOp::SandboxState);
        }
        t.elapsed() / 50
    };
    let no_ns = {
        let sb = sb.clone().with_namespaces(false);
        let t = Instant::now();
        for _ in 0..50 {
            let _ = selftest(&sb, SelftestOp::SandboxState);
        }
        t.elapsed() / 50
    };
    println!("no-op without namespaces: {:?}", no_ns);
    samples.sort();
    let total: Duration = samples.iter().sum();
    println!(
        "probe x50: mean {:?} median {:?} max {:?}; no-op job (spawn+sandbox+exit) mean {:?}",
        total / 50,
        samples[25],
        samples[49],
        idle
    );
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xffff_ffffu32;
    for b in data {
        crc ^= *b as u32;
        for _ in 0..8 {
            crc = if crc & 1 != 0 { (crc >> 1) ^ 0xedb8_8320 } else { crc >> 1 };
        }
    }
    !crc
}

fn png_chunk(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
    let mut out = (body.len() as u32).to_be_bytes().to_vec();
    let mut tagged = kind.to_vec();
    tagged.extend_from_slice(body);
    out.extend_from_slice(&tagged);
    out.extend_from_slice(&crc32(&tagged).to_be_bytes());
    out
}

fn png_declaring(width: u32, height: u32) -> Vec<u8> {
    let mut ihdr = width.to_be_bytes().to_vec();
    ihdr.extend_from_slice(&height.to_be_bytes());
    ihdr.extend_from_slice(&[8, 2, 0, 0, 0]);
    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    out.extend(png_chunk(b"IHDR", &ihdr));
    out.extend(png_chunk(b"IDAT", &[0x78, 0x01, 0x01, 0x00]));
    out.extend(png_chunk(b"IEND", &[]));
    out
}

fn bmp_declaring(width: i32, height: i32) -> Vec<u8> {
    let mut out = b"BM".to_vec();
    out.extend_from_slice(&(54u32 + 16).to_le_bytes());
    out.extend_from_slice(&[0; 4]);
    out.extend_from_slice(&54u32.to_le_bytes());
    out.extend_from_slice(&40u32.to_le_bytes());
    out.extend_from_slice(&width.to_le_bytes());
    out.extend_from_slice(&height.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&24u16.to_le_bytes());
    out.extend_from_slice(&[0; 24]);
    out.extend_from_slice(&[0; 16]);
    out
}

fn assert_survives(sb: &MediaSandbox, label: &str, bytes: Vec<u8>, jobs: Vec<Job>) {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("hostile");
    std::fs::write(&p, bytes).unwrap();
    for job in jobs {
        let start = Instant::now();
        let r = sb.run(&p, job.clone());
        assert!(r.is_err(), "{label} {job:?} was accepted: {:?}", r.map(|(m, _)| m));
        assert!(
            start.elapsed() < Duration::from_secs(15),
            "{label} {job:?} ran {:?}",
            start.elapsed()
        );
    }
    assert!(sb.run(&corpus("a.jpg"), Job::Probe).is_ok(), "{label}: next job must still work");
}

#[test]
fn huge_declared_png_is_refused_within_limits() {
    let sb = sandbox().with_memory_mb(256).with_timeout(Duration::from_secs(10));
    for (w, h) in [(30000u32, 30000u32), (65535, 65535), (0x7fff_ffff, 0x7fff_ffff)] {
        assert_survives(
            &sb,
            &format!("png {w}x{h}"),
            png_declaring(w, h),
            vec![
                Job::Rgb,
                Job::Thumbnail,
                Job::Compress { mime_type: "image/png".into() },
                Job::LabelImage { width: 224, height: 224 },
            ],
        );
    }
}

#[test]
fn huge_declared_bmp_is_refused_within_limits() {
    let sb = sandbox().with_memory_mb(256).with_timeout(Duration::from_secs(10));
    for (w, h) in [(20000i32, 20000i32), (i32::MAX, 3)] {
        assert_survives(
            &sb,
            &format!("bmp {w}x{h}"),
            bmp_declaring(w, h),
            vec![Job::Rgb, Job::Thumbnail, Job::LabelImage { width: 224, height: 224 }],
        );
    }
}

#[test]
fn allocation_over_the_cap_kills_the_worker_not_the_server() {
    let sb = sandbox().with_memory_mb(128);
    let r = selftest(&sb, SelftestOp::AllocHuge);
    assert!(matches!(r, Err(MediaError::Rejected(_))), "{r:?}");
}

fn generated(name: &str, args: &[&str]) -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join(name);
    let status = Command::new("ffmpeg")
        .args(["-v", "error", "-y"])
        .args(args)
        .arg(&out)
        .status()
        .expect("ffmpeg");
    assert!(status.success(), "ffmpeg could not make {name}");
    (dir, out)
}

#[test]
fn animated_webp_and_other_legit_media_probe_in_the_worker() {
    let sb = sandbox();
    let video = ["-f", "lavfi", "-i", "testsrc=size=64x64:rate=5:duration=1"];
    let cases: Vec<(&str, Vec<&str>)> = vec![
        ("a.webp", [&video[..], &["-loop", "0"]].concat()),
        ("a.mkv", [&video[..], &["-c:v", "mpeg4"]].concat()),
        ("a.gif", video.to_vec()),
        ("a.webm", [&video[..], &["-c:v", "libvpx"]].concat()),
    ];
    for (name, args) in cases {
        let (_dir, path) = generated(name, &args);
        match sb.run(&path, Job::Probe) {
            Ok((Meta::Probe(p), _)) => assert!(p.stream_count > 0, "{name}"),
            other => panic!("{name}: {other:?}"),
        }
    }
}

const DASH: &str = "<?xml version=\"1.0\"?><MPD xmlns=\"urn:mpeg:dash:schema:mpd:2011\" type=\"static\" mediaPresentationDuration=\"PT1S\" minBufferTime=\"PT1S\" profiles=\"urn:mpeg:dash:profile:isoff-on-demand:2011\"><Period><AdaptationSet mimeType=\"video/mp4\"><Representation id=\"1\" bandwidth=\"1000\"><SegmentList duration=\"1\"><SegmentURL media=\"secret.mp4\"/></SegmentList></Representation></AdaptationSet></Period></MPD>";

#[test]
fn dash_manifest_and_concat_through_compress_are_refused() {
    let sb = sandbox();
    let dir = tempfile::tempdir().unwrap();
    std::fs::copy(corpus("a.mp4"), dir.path().join("secret.mp4")).unwrap();
    for (name, body) in [
        ("dash.mp4", DASH),
        ("concat.jpg", "ffconcat version 1.0\nfile /etc/hostname\n"),
    ] {
        let p = dir.path().join(name);
        std::fs::write(&p, body).unwrap();
        for job in [
            Job::Probe,
            Job::Rgb,
            Job::Thumbnail,
            Job::Compress { mime_type: "image/jpeg".into() },
            Job::LabelVideo { width: 224, height: 224 },
        ] {
            let r = sb.run(&p, job.clone());
            assert!(matches!(r, Err(MediaError::Job(_))), "{name} {job:?}: {r:?}");
        }
    }
}

#[test]
fn valid_but_decoder_hungry_image_is_stopped_by_the_memory_cap() {
    let (_dir, path) = generated(
        "big.png",
        &["-f", "lavfi", "-i", "color=c=gray:size=12000x12000", "-frames:v", "1"],
    );
    assert!(std::fs::metadata(&path).unwrap().len() < 16 * 1024 * 1024);
    let job = Job::LabelImage { width: 224, height: 224 };

    let uncapped = sandbox().with_memory_mb(4096).with_timeout(Duration::from_secs(60));
    let ok = uncapped.run(&path, job.clone());
    assert!(matches!(ok, Ok((Meta::Image { .. }, _))), "control run: {ok:?}");

    let capped = sandbox().with_memory_mb(256).with_timeout(Duration::from_secs(60));
    let start = Instant::now();
    let r = capped.run(&path, job);
    assert!(r.is_err(), "decode must not fit in 256 MiB");
    assert!(start.elapsed() < Duration::from_secs(30));
    assert!(capped.run(&corpus("a.jpg"), Job::Probe).is_ok());
}
