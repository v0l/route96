use crate::settings::Settings;
use anyhow::anyhow;
use log::{debug, error, info, warn};
use nostr_sdk::prelude::*;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;
use tokio::task::{self, JoinHandle};
use tokio::time::timeout;

/// Constant for the Nostr event kind used for group membership (NIP-29).
const GROUP_MEMBERSHIP_KIND: Kind = Kind::Custom(39002);

/// Constant for the Nostr event kind used for group admins (NIP-29).
const GROUP_ADMINS_KIND: Kind = Kind::Custom(39001);

/// Default timeout duration for fetching events from the relay.
const FETCH_TIMEOUT: Duration = Duration::from_secs(10);

/// Represents the role of a public key within a group.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GroupRole {
    Member,
    Admin,
}

/// Caches the state of NIP-29 groups.
#[derive(Default)]
pub struct GroupStateCache {
    /// Maps Group ID (d tag) -> (PubKey Hex -> Role)
    cache: HashMap<String, HashMap<String, GroupRole>>,
}

impl GroupStateCache {
    fn new() -> Self {
        Default::default()
    }

    fn update_role(&mut self, group_id: &str, pubkey_hex: &str, role: GroupRole) {
        self.cache
            .entry(group_id.to_string())
            .or_default()
            .insert(pubkey_hex.to_string(), role);
    }

    fn get_role(&self, group_id: &str, pubkey_hex: &str) -> Option<GroupRole> {
        self.cache
            .get(group_id)
            .and_then(|group| group.get(pubkey_hex))
            .copied()
    }

    fn get_admins(&self, group_id: &str) -> HashSet<String> {
        self.cache
            .get(group_id)
            .map(|group| {
                group
                    .iter()
                    .filter(|(_, role)| **role == GroupRole::Admin)
                    .map(|(pk, _)| pk.clone())
                    .collect()
            })
            .unwrap_or_default()
    }
}

/// Processes the tags of a NIP-29 event to extract the group ID and member pubkeys.
fn process_event_tags(event: &Event) -> Option<(String, HashSet<PublicKey>)> {
    let group_id = event.tags.iter().find_map(|tag| {
        let slice = tag.as_slice();
        if slice.len() >= 2 && slice[0] == "d" {
            Some(slice[1].clone())
        } else {
            None
        }
    })?;

    let members = event
        .tags
        .iter()
        .filter_map(|tag| {
            let slice = tag.as_slice();
            if slice.len() >= 2 && slice[0] == "p" {
                PublicKey::from_hex(&slice[1]).ok()
            } else {
                None
            }
        })
        .collect::<HashSet<PublicKey>>();

    if members.is_empty() {
        None // Need at least one member tag
    } else {
        Some((group_id, members))
    }
}

/// Updates the cache based on a single NIP-29 event.
async fn update_cache_entry(event: &Event, cache: &mut GroupStateCache, relay_pubkey: &PublicKey) {
    // Ensure the event author matches the relay's pubkey
    if event.pubkey != *relay_pubkey {
        warn!(
            "Ignoring NIP-29 event {} from wrong author: {}",
            event.id,
            event.pubkey.to_hex()
        );
        return;
    }

    if let Some((group_id, members)) = process_event_tags(event) {
        let role = match event.kind {
            kind if kind == GROUP_ADMINS_KIND => GroupRole::Admin,
            kind if kind == GROUP_MEMBERSHIP_KIND => GroupRole::Member,
            _ => return, // Ignore other kinds
        };

        debug!(
            "Updating cache for group '{}' with role {:?} for {} members from event {}",
            group_id,
            role,
            members.len(),
            event.id
        );
        for member_pk in members {
            cache.update_role(&group_id, &member_pk.to_hex(), role);
        }
    } else {
        warn!(
            "Could not process tags for NIP-29 event {}: Missing 'd' or 'p' tags?",
            event.id
        );
    }
}

/// Client for interacting with NIP-29 group information, primarily through a cache
/// updated by a background task.
pub struct Nip29Client {
    cache: Arc<RwLock<GroupStateCache>>,
}

impl Nip29Client {
    /// Determines if a public key is a member of a specified group by checking the cache.
    pub async fn is_group_member(&self, group_id: &str, pubkey: &PublicKey) -> bool {
        let cache = self.cache.read().await;
        cache
            .get_role(group_id, &pubkey.to_hex())
            .map_or(false, |role| {
                role == GroupRole::Member || role == GroupRole::Admin
            })
    }

    /// Determines if a public key is an admin of a specified group by checking the cache.
    pub async fn is_group_admin(&self, group_id: &str, pubkey: &PublicKey) -> bool {
        let cache = self.cache.read().await;
        cache
            .get_role(group_id, &pubkey.to_hex())
            .map_or(false, |role| role == GroupRole::Admin)
    }

    // Helper to get admin set (if needed elsewhere)
    pub async fn get_group_admins(&self, group_id: &str) -> HashSet<String> {
        let cache = self.cache.read().await;
        cache.get_admins(group_id)
    }
}

/// Initializes the NIP-29 client, populates the initial cache, and starts the background task.
///
/// Returns both the client handle (for cache access) and the background task JoinHandle.
pub async fn init_nip29_client(
    settings: &Settings,
    keys: Keys,
) -> anyhow::Result<(Arc<Nip29Client>, JoinHandle<()>)> {
    let relay_url = settings.nip29_relay.url.clone();

    info!("Initializing NIP-29 client for relay: {}", relay_url);

    let opts = Options::default();
    let client = Client::builder().signer(keys.clone()).opts(opts).build();

    if let Err(e) = client.add_relay(relay_url.clone()).await {
        return Err(anyhow!("Failed to add NIP-29 relay {}: {}", relay_url, e));
    }
    client.connect().await;

    let cache = Arc::new(RwLock::new(GroupStateCache::new()));
    let relay_pubkey = keys.public_key();

    let initial_filter = Filter::new()
        .author(relay_pubkey)
        .kinds(vec![GROUP_ADMINS_KIND, GROUP_MEMBERSHIP_KIND]);

    info!("Fetching initial NIP-29 events...");
    // Fetch events using fetch_events_from
    match timeout(
        FETCH_TIMEOUT + Duration::from_secs(2), // Outer timeout guard
        client.fetch_events_from(
            vec![relay_url.clone()],
            initial_filter.clone(),
            FETCH_TIMEOUT,
        ),
    )
    .await
    {
        Ok(Ok(events)) => {
            info!("Fetched {} initial NIP-29 events", events.len());
            let mut cache_writer = cache.write().await;
            for event in events {
                update_cache_entry(&event, &mut cache_writer, &relay_pubkey).await;
            }
        }
        Ok(Err(e)) => {
            warn!(
                "Error fetching initial NIP-29 events from {}: {}. Cache might be stale.",
                relay_url, e
            );
        }
        Err(_) => {
            warn!(
                "Timeout fetching initial NIP-29 events from {} after {:?}. Cache might be stale.",
                relay_url, FETCH_TIMEOUT
            );
        }
    }

    // Spawn background task to listen for updates
    let background_client = client.clone();
    let background_cache = cache.clone();
    let background_relay_url = relay_url.clone();

    let join_handle: JoinHandle<()> = task::spawn(async move {
        info!(
            "Starting NIP-29 background notification handler for {}...",
            background_relay_url
        );

        // Subscribe to future updates
        let sub_opts = SubscribeAutoCloseOptions::default();
        let subscription_filter = Filter::new()
            .author(relay_pubkey)
            .kinds(vec![GROUP_ADMINS_KIND, GROUP_MEMBERSHIP_KIND]);

        if let Err(e) = background_client
            .subscribe_to(
                vec![background_relay_url.clone()],
                subscription_filter,
                Some(sub_opts),
            )
            .await
        {
            error!(
                "Failed to subscribe to NIP-29 updates on {}: {}",
                background_relay_url, e
            );
            return;
        }
        info!("Subscribed to NIP-29 updates on {}.", background_relay_url);

        // Handle notifications (rest of the handler remains the same)
        if let Err(e) = background_client
            .handle_notifications(|notification| {
                let cache_clone = background_cache.clone();
                let relay_pk = relay_pubkey;

                async move {
                    match notification {
                        RelayPoolNotification::Event { event, .. } => {
                            if event.kind == GROUP_ADMINS_KIND
                                || event.kind == GROUP_MEMBERSHIP_KIND
                            {
                                debug!(
                                    "Received NIP-29 update event: kind={}, id={}",
                                    event.kind, event.id
                                );
                                let mut cache_writer = cache_clone.write().await;
                                update_cache_entry(&event, &mut cache_writer, &relay_pk).await;
                            }
                        }
                        RelayPoolNotification::Shutdown => {
                            info!("NIP-29 notification handler received shutdown signal.");
                            return Ok(true);
                        }
                        _ => {}
                    }
                    Ok(false)
                }
            })
            .await
        {
            error!("NIP-29 notification handler error: {}", e);
        }
        info!(
            "NIP-29 background notification handler finished for {}.",
            background_relay_url
        );
    });

    let nip29_client_instance = Nip29Client { cache };

    info!("NIP-29 client initialized successfully.");
    Ok((Arc::new(nip29_client_instance), join_handle))
}
