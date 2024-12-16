use clap::Parser;
use log::{info, warn};
use nostr::serde_json;
use nostr_cursor::cursor::NostrCursor;
use regex::Regex;
use rocket::futures::StreamExt;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use tokio::fs::File;
use tokio::io::AsyncWriteExt;
use uuid::Uuid;

#[derive(Parser, Debug)]
#[command(version, about)]
struct ProgramArgs {
    /// Directory pointing to archives to scan
    #[arg(short, long)]
    pub archive: PathBuf,

    /// Output path .csv
    #[arg(short, long)]
    pub output: PathBuf,
}

#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    pretty_env_logger::init();

    let args: ProgramArgs = ProgramArgs::parse();

    let mut report: HashMap<String, HashSet<String>> = HashMap::new();

    let mut binding = NostrCursor::new(args.archive);
    let mut cursor = Box::pin(binding.walk());
    let matcher = Regex::new(r"void\.cat/d/(\w+)")?;
    while let Some(Ok(e)) = cursor.next().await {
        if e.content.contains("void.cat") {
            let links = matcher.captures_iter(&e.content).collect::<Vec<_>>();
            for link in links {
                let g = link.get(1).unwrap().as_str();
                let base58 = if let Ok(b) = nostr::bitcoin::base58::decode(g) {
                    b
                } else {
                    warn!("Invalid base58 id {}", g);
                    continue;
                };
                let _uuid = if let Ok(u) = Uuid::from_slice_le(base58.as_slice()) {
                    u
                } else {
                    warn!("Invalid uuid {}", g);
                    continue;
                };
                info!("Got link: {} => {}", g, e.pubkey);
                if let Some(ur) = report.get_mut(&e.pubkey) {
                    ur.insert(g.to_string());
                } else {
                    report.insert(e.pubkey.clone(), HashSet::from([g.to_string()]));
                }
            }
        }
    }

    let json = serde_json::to_string(&report)?;
    File::create(args.output)
        .await?
        .write_all(json.as_bytes())
        .await?;

    Ok(())
}
