#![allow(dead_code)]

#[path = "../../src/media/protocol.rs"]
mod protocol;

mod jobs;
mod sandbox;
mod selftest;

use protocol::{JobError, Request, Response, read_frame, write_frame};
use std::fs::File;
use std::io::{Read, Write};
use std::os::unix::io::FromRawFd;

struct Args {
    timeout_secs: u64,
    memory_mb: u64,
    namespaces: bool,
    selftest: bool,
}

fn parse_args() -> Args {
    let mut args = Args {
        timeout_secs: 120,
        memory_mb: 2048,
        namespaces: true,
        selftest: false,
    };
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "--timeout-secs" => args.timeout_secs = it.next().and_then(|v| v.parse().ok()).unwrap_or(120),
            "--memory-mb" => args.memory_mb = it.next().and_then(|v| v.parse().ok()).unwrap_or(2048),
            "--no-namespaces" => args.namespaces = false,
            "--selftest" => args.selftest = true,
            _ => {}
        }
    }
    args
}

fn respond(out: &mut impl Write, response: &Response, payload: &[u8]) -> std::io::Result<()> {
    write_frame(out, response)?;
    out.write_all(payload)?;
    out.flush()
}

fn main() {
    let args = parse_args();
    let cfg = sandbox::Config {
        timeout_secs: args.timeout_secs,
        memory_mb: args.memory_mb,
        namespaces: args.namespaces,
    };
    if let Err(e) = sandbox::enter(&cfg) {
        eprintln!("route96-media: sandbox setup failed: {e}");
        std::process::exit(70);
    }

    let mut stdin = unsafe { File::from_raw_fd(0) };
    let mut stdout = unsafe { File::from_raw_fd(1) };
    let request: Request = match read_frame(&mut stdin) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("route96-media: bad request: {e}");
            std::process::exit(64);
        }
    };
    let mut drain = Vec::new();
    let _ = stdin.read_to_end(&mut drain);

    let result = match request.job {
        protocol::Job::Selftest { .. } if !args.selftest => Err(JobError::Failed {
            message: "selftest disabled".to_string(),
        }),
        job => jobs::run(job),
    };

    let written = match result {
        Ok((_, payload)) if payload.len() as u64 > request.max_output_bytes => respond(
            &mut stdout,
            &Response::Err {
                error: JobError::Failed {
                    message: "output exceeds limit".to_string(),
                },
            },
            &[],
        ),
        Ok((meta, payload)) => respond(
            &mut stdout,
            &Response::Ok {
                meta,
                payload_len: payload.len() as u64,
            },
            &payload,
        ),
        Err(error) => respond(&mut stdout, &Response::Err { error }, &[]),
    };
    std::process::exit(if written.is_ok() { 0 } else { 65 });
}
