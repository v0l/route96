use crate::settings::Settings;
use anyhow::Error;
use log::{debug, info, warn};
use nostr_sdk::prelude::*;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{Mutex, RwLock};
use tokio::task;

/// Constant for the Nostr event kind used for group membership (NIP-29).
const GROUP_MEMBERSHIP_KIND: Kind = Kind::Custom(39002);

/// Constant for the Nostr event kind used for group admins (NIP-29).
const GROUP_ADMINS_KIND: Kind = Kind::Custom(39001);

/// Default timeout duration for fetching events from the relay.
const FETCH_TIMEOUT: Duration = Duration::from_secs(10);

/// Timeout for the initial subscription before starting the notification handler.
const INITIAL_SUBSCRIBE_TIMEOUT: Duration = Duration::from_secs(5);

/// Represents the role of a public key within a group.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Role {
    Member,
    Admin,
}

/// Maps public keys to their roles within a specific group.
type GroupData = HashMap<PublicKey, Role>;

/// Cache for storing group state (members and admins) received from the relay.
/// This cache is populated initially and then kept up-to-date by a background task.
#[derive(Debug)]
pub struct GroupStateCache {
    /// Maps group IDs (d tag) to the GroupData (PublicKey -> Role map).
    cache: HashMap<String, GroupData>,
}

impl GroupStateCache {
    /// Initializes a new, empty cache.
    pub fn new() -> Self {
        Self {
            cache: HashMap::new(),
        }
    }

    /// Updates the cache with new data for a specific group based on an event.
    /// This method assumes it's called with a write lock held.
    /// It replaces either the members or the admins based on the event kind.
    fn update_group_data(&mut self, group_id: &str, kind: Kind, pubkeys: HashSet<PublicKey>) {
        let role_to_update = if kind == GROUP_MEMBERSHIP_KIND {
            Role::Member
        } else if kind == GROUP_ADMINS_KIND {
            Role::Admin
        } else {
            warn!("Attempted to update cache with unexpected kind: {:?}", kind);
            return; // Ignore unexpected kinds
        };

        info!(
            "Updating cache for group '{}', role '{:?}' with {} entries.",
            group_id,
            role_to_update,
            pubkeys.len()
        );

        let group_entry = self.cache.entry(group_id.to_string()).or_default();

        // Remove existing entries for the role being updated
        group_entry.retain(|_, role| *role != role_to_update);

        // Insert new entries
        for pubkey in pubkeys {
            group_entry.insert(pubkey, role_to_update);
        }
    }

    /// Checks if a public key is a member of a group based on cached data.
    /// Assumes a read lock is held.
    pub fn is_member(&self, group_id: &str, pubkey: &PublicKey) -> bool {
        self.cache.get(group_id).map_or(false, |group_data| {
            group_data.get(pubkey) == Some(&Role::Member)
        })
    }

    /// Checks if a public key is an admin of a group based on cached data.
    /// Assumes a read lock is held.
    pub fn is_admin(&self, group_id: &str, pubkey: &PublicKey) -> bool {
        self.cache.get(group_id).map_or(false, |group_data| {
            group_data.get(pubkey) == Some(&Role::Admin)
        })
    }

    /// Returns the set of members for the given group from the cache.
    /// Assumes a read lock is held.
    pub fn get_members(&self, group_id: &str) -> HashSet<PublicKey> {
        self.cache
            .get(group_id)
            .map_or_else(HashSet::new, |group_data| {
                group_data
                    .iter()
                    .filter(|(_, role)| **role == Role::Member)
                    .map(|(pubkey, _)| *pubkey)
                    .collect()
            })
    }

    /// Returns the set of admins for the given group from the cache.
    /// Assumes a read lock is held.
    pub fn get_admins(&self, group_id: &str) -> HashSet<PublicKey> {
        self.cache
            .get(group_id)
            .map_or_else(HashSet::new, |group_data| {
                group_data
                    .iter()
                    .filter(|(_, role)| **role == Role::Admin)
                    .map(|(pubkey, _)| *pubkey)
                    .collect()
            })
    }
}

/// Client for interacting with NIP-29 relays to manage group membership.
/// Relies on a cache updated by a background task.
pub struct Nip29Client {
    relay_keys: Keys,
    cache: Arc<RwLock<GroupStateCache>>,
    client: Arc<Client>,
    // Signal to gracefully shutdown the background task
    shutdown_signal: Arc<Mutex<bool>>,
}

impl Nip29Client {
    /// Determines if a public key is a member of a specified group by checking the cache.
    pub async fn is_group_member(&self, group_id: &str, pubkey: &PublicKey) -> bool {
        let cache = self.cache.read().await;
        cache.is_member(group_id, pubkey)
    }

    /// Checks if a public key is an admin of a group by checking the cache.
    pub async fn is_group_admin(&self, group_id: &str, pubkey: &PublicKey) -> bool {
        let cache = self.cache.read().await;
        cache.is_admin(group_id, pubkey)
    }

    /// Retrieves the current set of members for a group from the cache.
    pub async fn get_group_members(&self, group_id: &str) -> HashSet<PublicKey> {
        let cache = self.cache.read().await;
        cache.get_members(group_id)
    }

    /// Retrieves the current set of admins for a group from the cache.
    pub async fn get_group_admins(&self, group_id: &str) -> HashSet<PublicKey> {
        let cache = self.cache.read().await;
        cache.get_admins(group_id)
    }

    /// Signals the background notification handler to shut down.
    pub async fn shutdown(&self) {
        let mut lock = self.shutdown_signal.lock().await;
        *lock = true;
        // Consider waiting for the task to actually finish if needed,
        // though dropping the client might be sufficient depending on context.
        info!("NIP-29 client shutdown signaled.");
    }
}

/// Processes a NIP-29 event (admin or member list) and updates the cache.
/// Returns the group ID and the latest timestamp if successful.
async fn process_and_update_cache_event(
    event: &Event,
    cache_writer: &mut tokio::sync::RwLockWriteGuard<'_, GroupStateCache>,
    relay_pubkey: &PublicKey,
) -> Option<(String, Timestamp)> {
    if event.pubkey != *relay_pubkey {
        debug!("Ignoring event not authored by relay key: {}", event.id);
        return None; // Should be authored by our key
    }

    // Correctly extract the identifier ('d' tag) using slice check
    let group_id_opt: Option<String> = event.tags.iter().find_map(|tag| {
        let slice = tag.as_slice();
        if slice.len() >= 2 && slice[0] == "d" {
            Some(slice[1].to_string())
        } else {
            None
        }
    });

    let group_id = match group_id_opt {
        Some(id) => id, // id is already a String here
        None => {
            warn!("Event {} missing 'd' tag (group_id)", event.id);
            return None;
        }
    };

    let mut pubkeys = HashSet::new();
    // Iterate over tags correctly and parse PublicKey from 'p' tags
    for tag in event.tags.iter() {
        let slice = tag.as_slice();
        if slice.len() >= 2 && slice[0] == "p" {
            match PublicKey::parse(&slice[1]) {
                Ok(pubkey) => {
                    pubkeys.insert(pubkey);
                }
                Err(e) => {
                    warn!(
                        "Failed to parse pubkey tag in event {}: {} ({})",
                        event.id, slice[1], e
                    );
                }
            }
        }
    }

    debug!(
        "Processing event {} for group '{}', kind {:?}, found {} pubkeys.",
        event.id,
        &group_id,
        event.kind,
        pubkeys.len()
    );
    cache_writer.update_group_data(&group_id, event.kind, pubkeys);
    Some((group_id, event.created_at)) // group_id is String
}

/// Initializes a NIP-29 client, populates the cache with initial data,
/// and spawns a background task to keep the cache updated.
pub async fn init_nip29_client(settings: &Settings) -> Result<Arc<Nip29Client>, Error> {
    let nip29_config = &settings.nip29_relay;
    let clean_key = nip29_config.private_key.trim().trim_matches('"');
    let keys = Keys::parse(clean_key)?;
    let relay_pubkey = keys.public_key();
    let relay_url = nip29_config.url.clone();

    let opts = Options::default()
        .autoconnect(true)
        .automatic_authentication(true);
    let client = Arc::new(Client::builder().signer(keys.clone()).opts(opts).build());

    // Add the single relay
    if client.add_relay(&relay_url).await? {
        info!("Added NIP-29 relay: {}", relay_url);
    } else {
        info!("NIP-29 relay {} already existed.", relay_url);
    }
    client.connect_relay(&relay_url).await?;
    info!("Connecting to NIP-29 relay: {}", relay_url);
    client.wait_for_connection(FETCH_TIMEOUT).await; // Wait for connection

    // --- Initial Cache Population ---
    info!(
        "Starting initial NIP-29 cache population from {}",
        relay_url
    );
    let cache = Arc::new(RwLock::new(GroupStateCache::new()));
    let initial_filter = Filter::new()
        .author(relay_pubkey)
        .kinds(vec![GROUP_ADMINS_KIND, GROUP_MEMBERSHIP_KIND]);

    let initial_events = client
        .fetch_events_from(vec![&relay_url], initial_filter.clone(), FETCH_TIMEOUT)
        .await?;

    // Clone the events for later debugging
    let debug_events = initial_events.clone();

    let mut latest_event_timestamp = Timestamp::from(0);
    {
        let mut cache_writer = cache.write().await;
        info!(
            "Fetched {} initial events for NIP-29 cache.",
            initial_events.len()
        );
        for event in initial_events {
            if let Some((_group_id, timestamp)) =
                process_and_update_cache_event(&event, &mut cache_writer, &relay_pubkey).await
            {
                if timestamp > latest_event_timestamp {
                    latest_event_timestamp = timestamp;
                }
            }
        }

        // Debug: Dump the full cache to diagnose why it might be empty
        info!("===== NIP-29 CACHE CONTENTS AFTER INITIAL POPULATION =====");
        for (group_id, group_data) in &cache_writer.cache {
            info!("Group: {} has {} entries:", group_id, group_data.len());
            for (pubkey, role) in group_data {
                info!("  - Pubkey: {}, Role: {:?}", pubkey, role);
            }
        }

        if cache_writer.cache.is_empty() {
            info!("CACHE IS EMPTY! No groups were loaded");
            info!("Event processing details:");
            for event in debug_events.iter() {
                info!(
                    "Event ID: {}, Kind: {:?}, Tags: {:?}",
                    event.id, event.kind, event.tags
                );
                info!(
                    "  - Content (truncated): {}",
                    &event.content[..event.content.len().min(30)]
                );

                // Check if this event has a 'd' tag (group identifier)
                let has_d_tag = event.tags.iter().any(|tag| {
                    let slice = tag.as_slice();
                    slice.len() >= 2 && slice[0] == "d"
                });

                // Check if this event has 'p' tags (pubkeys)
                let p_tag_count = event
                    .tags
                    .iter()
                    .filter(|tag| {
                        let slice = tag.as_slice();
                        slice.len() >= 2 && slice[0] == "p"
                    })
                    .count();

                info!(
                    "  - Has 'd' tag: {}, 'p' tag count: {}",
                    has_d_tag, p_tag_count
                );
                info!(
                    "  - Author: {} matches relay pubkey: {}",
                    event.pubkey,
                    event.pubkey == relay_pubkey
                );
            }
        }
    } // Release write lock
    info!(
        "Initial NIP-29 cache population complete. Latest event timestamp: {}",
        latest_event_timestamp.as_u64()
    );

    // --- Spawn Background Listener ---
    let background_client = client.clone();
    let background_cache = cache.clone();
    let background_keys = keys.clone(); // Clone keys for author check
    let shutdown_signal = Arc::new(Mutex::new(false));
    let task_shutdown_signal = shutdown_signal.clone();

    task::spawn(async move {
        info!("Spawning NIP-29 background listener task.");
        let relay_pubkey = background_keys.public_key();
        let subscription_filter = Filter::new()
            .author(relay_pubkey)
            .kinds(vec![GROUP_ADMINS_KIND, GROUP_MEMBERSHIP_KIND])
            .since(latest_event_timestamp + Duration::from_secs(1)); // Start slightly after last known event

        // Subscribe first
        // Pass `None` for options to create a non-auto-closing subscription.
        // Use a generated SubscriptionId.
        match background_client
            .subscribe_with_id_to(
                vec![&relay_url],
                SubscriptionId::generate(),
                subscription_filter,
                None,
            )
            .await
        {
            Ok(output) => {
                if !output.success.is_empty() {
                    info!(
                        "Successfully subscribed to NIP-29 updates on {}.",
                        relay_url
                    );
                } else {
                    warn!(
                        "Failed to subscribe to NIP-29 updates on {}: {:?}",
                        relay_url, output.failed
                    );
                    // Consider if we should retry or exit the task here
                    return;
                }
            }
            Err(e) => {
                warn!("Error subscribing to NIP-29 updates: {}", e);
                // Consider if we should retry or exit the task here
                return;
            }
        }

        // Now start handling notifications
        info!("Starting NIP-29 notification handler loop.");
        let result = background_client
            .handle_notifications(|notification| {
                let cache_clone = background_cache.clone();
                let shutdown_clone = task_shutdown_signal.clone();
                let relay_pk = relay_pubkey; // Avoid capturing background_keys directly

                async move {
                    // Check shutdown signal first
                    if *shutdown_clone.lock().await {
                        info!("NIP-29 handler received shutdown signal. Exiting loop.");
                        return Ok(true); // Exit loop
                    }

                    match notification {
                        RelayPoolNotification::Event { event, .. } => {
                            if event.kind == GROUP_ADMINS_KIND
                                || event.kind == GROUP_MEMBERSHIP_KIND
                            {
                                debug!("Received potential NIP-29 event: {}", event.id);
                                let mut cache_writer = cache_clone.write().await;
                                // Don't need the timestamp from processing here
                                let _ = process_and_update_cache_event(
                                    &event,
                                    &mut cache_writer,
                                    &relay_pk,
                                )
                                .await;
                            }
                        }
                        RelayPoolNotification::Message { message, .. } => {
                            if let RelayMessage::Notice(notice) = message {
                                info!("NIP-29 Relay Notice: {}", notice);
                            }
                        }
                        RelayPoolNotification::Shutdown => {
                            info!("NIP-29 RelayPool shutdown notification received. Exiting loop.");
                            return Ok(true); // Exit loop
                        }
                    }
                    Ok(false) // Continue loop
                }
            })
            .await;

        match result {
            Ok(_) => info!("NIP-29 notification handler loop exited gracefully."),
            Err(e) => warn!("NIP-29 notification handler loop exited with error: {}", e),
        }
    });

    info!(
        "NIP-29 client initialized with relay URL: {}",
        nip29_config.url
    );

    Ok(Arc::new(Nip29Client {
        relay_keys: keys,
        cache,
        client,
        shutdown_signal,
    }))
}
