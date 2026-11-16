# Media sandbox

Every libav call on upload data runs in a short-lived worker process, `route96-media`, that is confined before it reads any input. The server process links no libav code and never decodes untrusted media itself.

## Threat model

libav* (demuxers, decoders, scalers, the webp and mjpeg encoders) is a large C code base that parses attacker-controlled bytes. Assume a memory-corruption bug in any of it gives an attacker arbitrary code execution inside whichever process runs it.

Before this change that process was the route96 server: it holds the database credentials, the whole store (hot and cold), the Lightning and payment configuration, the admin keys and an unrestricted network. Any upload (or any file a background job re-reads) was one decoder bug away from owning all of that.

Assets protected: the server's memory and credentials, the store, the network position of the pod, other users' blobs.

Attacker: anyone who can upload a file that route96 will pass to libav (any `image/*`, `video/*`, `audio/*` upload, and anything served through the thumbnail route).

Out of scope: kernel bugs reachable through the syscalls we still allow (the allowlist keeps that surface small), side channels, denial of service by many concurrent valid uploads (limited by the existing upload limits).

## Design

```
route96 (server)                     route96-media (worker, one per job)
  media::client  --- spawn --->        sandbox::enter()   (before any input is read)
   stdin  : request frame              jobs::run()        (libav)
   fd 3   : input file, read only      stdout : response frame + payload
   stdout : response + payload         stderr : log lines, forwarded to the log
   timeout, kill process group
```

* One process per job. Nothing is shared between jobs, so a compromised worker cannot poison the next one.
* The input is the stored file opened read only by the parent and inherited as fd 3. The worker is given no path to the store. libav opens it with the `fd` protocol (`fd:` plus the `fd=3` option, `protocol_whitelist=fd`), which needs `read`, `lseek`, `fstat` and `dup`.
* Outputs (webp, jpeg, raw RGB frames, probe results) come back as bytes on stdout. The parent writes any file. The worker cannot write the store.
* Model inference stays in the parent. The labeler worker jobs only extract frames or images as raw RGB (`224x224x3` per frame, or the size asked for); the parent normalises them into tensors. pHash likewise: the worker returns raw RGB pixels and the parent hashes them with `image_hasher`.

### Protocol

Request on stdin: `u32` big endian length, then JSON (`Request`: `job`, `mime_type`, job parameters, `max_output_bytes`).

Response on stdout: `u32` big endian length, then JSON (`Response`: `Ok{ meta, payload_len }` or `Err{ kind, message, width, height }`), then `payload_len` raw bytes. The parent reads at most `max_output_bytes + header` and rejects anything malformed. The response is untrusted: the parent treats a compromised worker like any other client input (bounded length, JSON parse, size checks against the reported dimensions).

Jobs: `probe`, `compress` (image to webp, muxed into memory), `thumbnail` (512 px webp), `rgb` (decode first image to RGB, for pHash), `label_image` (decode and scale to RGB), `label_video` (seek and extract up to 10 frames as RGB), `llm_jpeg` (scale and encode a jpeg for the LLM labeler). A test only `selftest` job exists, refused unless the worker is started with `--selftest` (only the test client does that); it exercises the sandbox (see Tests).

### Failure semantics

A worker that exits non-zero, dies on a signal (SIGSYS from seccomp, SIGSEGV, SIGXCPU, SIGKILL from the OOM killer or the timeout), writes a malformed response or runs past the wall clock timeout fails the job as `MediaRejected`. Callers treat it like a decoder error on a bad file: the upload is refused (or the background job marks the file as unreadable) and the server carries on. It is never a server error and never panics the server. On timeout the parent `SIGKILL`s the worker's process group.

### Sandbox, in order (`media-worker/src/sandbox.rs`)

All of this runs in the worker, before it reads the request or the input.

1. `prctl(PR_SET_PDEATHSIG, SIGKILL)` and `PR_SET_DUMPABLE=0`.
2. Namespaces, best effort: `unshare` of user, mount, IPC, network and UTS namespaces, uid/gid map to a single unprivileged id, then fork so that the worker is PID 1 of a new PID namespace. The mount namespace is switched to an empty tmpfs root (`pivot_root` onto a tmpfs, old root detached), so the filesystem is gone except for the inherited fds. If `unshare` is refused (it is under Docker/containerd's default seccomp profile, so inside the production pod), this step is skipped, logged once at `warn`, and the remaining steps carry the whole sandbox. They are designed to be sufficient on their own: the allowlist contains no `open`/`openat`, `socket`, `execve`, `fork`, `mount` or `ptrace`, so the namespaces are defence in depth, not the primary control.
3. `prctl(PR_SET_NO_NEW_PRIVS)` and drop every capability (bounding set, then effective, permitted and inheritable).
4. rlimits: `RLIMIT_AS` (default 2 GiB, setting `media_worker_memory_mb`), `RLIMIT_CPU` (the job timeout, setting `media_worker_timeout_secs`), `RLIMIT_CORE=0`, `RLIMIT_FSIZE=0`, `RLIMIT_NOFILE=16`, `RLIMIT_MEMLOCK=0`.
5. seccomp-bpf allowlist via the `seccompiler` crate, default action `KILL_PROCESS`. Installed with `TSYNC` so it covers all threads. In-process, no runtime dependency (bubblewrap/nsjail were rejected: they need `CAP_SYS_ADMIN` or unprivileged user namespaces, which the hardened pod does not give, so they would not work in the one place that matters).

Threads: libav and libwebp/dav1d may start worker threads. `clone` is allowed only with `CLONE_THREAD` set (and without any `CLONE_NEW*` flag); `clone3` returns `ENOSYS`, so glibc falls back to `clone`. `fork`, `vfork` and a plain `clone` are killed.

### Syscall allowlist

Allowed (everything else is `KILL_PROCESS`, delivered as SIGSYS): `read`, `write`, `readv`, `writev`, `pread64`, `lseek`, `close`, `dup`, `dup3`, `fstat`, `newfstatat`, `statx`, `fcntl`, `ioctl` (answered `ENOTTY`), `mmap` and `mprotect` (without `PROT_EXEC`), `munmap`, `mremap`, `brk`, `madvise`, `futex`, `set_robust_list`, `rseq`, `sched_yield`, `sched_getaffinity`, `nanosleep`, `clock_nanosleep`, `clock_gettime`, `clock_getres`, `gettimeofday`, `getrandom`, `rt_sigaction`, `rt_sigprocmask`, `rt_sigreturn`, `sigaltstack`, `exit`, `exit_group`, `getpid`, `gettid`, `sysinfo`, `prlimit64` (query only, on self), `prctl` (`PR_SET_NAME` only), `clone` (threads only, see below), `clone3` (answered `ENOSYS`).

Denied with `EPERM` instead of killing (so libav can fail cleanly on a probe that tries them): `open`, `openat`, `openat2`, `access`, `faccessat`, `faccessat2`, `readlink`, `readlinkat`, `stat`, `lstat`, and `newfstatat`/`statx` on a path (an fd stat with `AT_EMPTY_PATH` is allowed). Everything else not listed, including `socket`, `connect`, `execve`, `execveat`, `fork`, `ptrace`, `mount`, `kill`, `bpf`, `io_uring_setup`, is `KILL_PROCESS`.

`mmap` and `mprotect` are allowed only without `PROT_EXEC`, and `mremap`/`munmap`/`brk`/`madvise` are plain. Shared libraries are mapped by the dynamic loader before the filter is installed, so no executable mapping is needed afterwards. The filters are installed in the order EPERM filter, ENOSYS/ENOTTY filters, then the allowlist last, so the earlier ones need no `seccomp` or `prctl` allowance; once the allowlist is on, `seccomp` itself is killed.

### Defence in depth in k8s

`~/git/v0l-k8s/nostr.download/app.yml`: the pod runs as a fixed non-root uid (`runAsUser: 10001`, `runAsNonRoot`), `readOnlyRootFilesystem: true` with `emptyDir` volumes for `/tmp` and the model cache, `allowPrivilegeEscalation: false`, `capabilities.drop: [ALL]` and `seccompProfile: RuntimeDefault`. User namespaces do not work under these settings (the runtime default seccomp profile denies `unshare` and `mount` without `CAP_SYS_ADMIN`), which is why the in-process seccomp allowlist is the real boundary and the namespace step is optional. `docker run --security-opt seccomp=default` reproduces this locally; `tests/media_sandbox.rs` has a test that runs the worker with namespaces forced off to prove the fallback alone blocks `open`, `socket` and `execve`.

The Dockerfile creates the `route96` user (uid 10001), ships `route96-media` next to `route96` (found next to the server executable, or via `ROUTE96_MEDIA_BIN`), and runs as that user. `/data` is chowned to it; the pod mounts PVCs there (fsGroup 10001) and an emptyDir on `/tmp`. Verified locally with `docker run --user 10001 --read-only --cap-drop ALL --security-opt no-new-privileges`: `unshare` is refused, the worker logs that and runs with seccomp only, and the self-test reports seccomp active.

## What stays out of the server binary

The libav-touching code lives only in the `media-worker/` workspace crate (binary `route96-media`). `ffmpeg-rs-raw` is not a dependency of the `route96` package at all. The library (used by the server and `r96util`) contains the protocol and client in `src/media/`; the worker `#[path]`-includes `src/media/protocol.rs` so both sides share the wire types. `tests/media_sandbox.rs` scans every source file under `src/` for `ffmpeg_rs_raw` and `ffmpeg_sys` and fails if one appears.

The `ffmpeg` CLI is only spawned by tests (to generate fixtures), never by server code.

## Tests

`tests/media_sandbox.rs` (integration, starts the real worker binary):

* `selftest` attempts `open("/etc/passwd")` (EPERM), `socket()` (killed), `execve` (killed), `fork` (killed), `mmap(PROT_EXEC)` (killed), `ptrace` (killed), and a normal read of fd 3 (works). Each ran failing first before the sandbox existed.
* A worker that spins forever, one that segfaults and one that allocates past `RLIMIT_AS` all give `MediaRejected` and leave the test process alive.
* Garbage, truncated and hostile (HLS, concat, DASH) inputs give clean errors, as do PNG and BMP files declaring up to 2^31 x 2^31 pixels. A valid 12000x12000 PNG decodes with a 4 GiB cap and is killed under a 256 MiB cap, proving the rlimit stops a decoder-level allocation.
* Probe, compress, thumbnail, pHash and label frames from the worker equal the previous in-process results on the sample corpus (`test-data/` plus generated fixtures).

## Overhead

Release worker, 50 probe jobs over the corpus (images, mp4/mkv/webm, audio) plus `sample.mp4`, measured by `tests/media_sandbox.rs::overhead_per_job` (`--ignored --nocapture`, `ROUTE96_MEDIA_BIN` pointing at the release worker):

| | mean | median | max |
|---|---|---|---|
| probe job, end to end | 22.7 ms | 22.0 ms | 54 ms |
| no-op job with namespaces (spawn, sandbox, exit) | 21.0 ms | | |
| no-op job, seccomp only | 17.0 ms | | |
| bare exec of the worker with libav linked, no sandbox | about 21 ms | | |

Nearly all of the cost is the dynamic linker loading libav; the sandbox adds 0 to 4 ms. That is at the 20 ms line from the issue, but every job in the server is followed by decoding, encoding or hashing that costs more, and uploads are not latency critical at that scale. A pre-spawned pool would need the input fd handed to an already confined worker, which needs `recvmsg` over a socket in the allowlist; not worth widening the sandbox for about 20 ms, so no pool was built.
