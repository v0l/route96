use crate::settings::Settings;
use anyhow::Error;
use log::{info, warn};
use nostr_sdk::prelude::*;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;

/// Constant for the Nostr event kind used for group membership (NIP-29).
const GROUP_MEMBERSHIP_KIND: Kind = Kind::Custom(39002);

/// Default timeout duration for fetching events from the relay.
const FETCH_TIMEOUT: Duration = Duration::from_secs(10);

/// Cache for storing group membership information to reduce relay queries.
pub struct GroupMembershipCache {
    /// Maps group IDs to a tuple of (expiration timestamp, set of member public keys).
    cache: HashMap<String, (Timestamp, HashSet<PublicKey>)>,
    /// Cache expiration duration in seconds.
    cache_expiration: u64,
}

impl GroupMembershipCache {
    /// Initializes a new cache with the specified expiration time.
    pub fn new(cache_expiration: u64) -> Self {
        Self {
            cache: HashMap::new(),
            cache_expiration,
        }
    }

    /// Checks if a public key is a member of a group based on cached data.
    ///
    /// Returns:
    /// - `Some(true)` if the public key is a member and the cache is valid.
    /// - `Some(false)` if the public key is not a member and the cache is valid.
    /// - `None` if the cache is expired or the group is not cached.
    pub fn is_member(&self, group_id: &str, pubkey: &PublicKey) -> Option<bool> {
        self.cache.get(group_id).and_then(|(expiration, members)| {
            if *expiration > Timestamp::now() {
                Some(members.contains(pubkey))
            } else {
                None
            }
        })
    }

    /// Returns the set of members for the given group if the cache is valid.
    pub fn get_members(&self, group_id: &str) -> Option<HashSet<PublicKey>> {
        self.cache.get(group_id).and_then(|(expiration, members)| {
            if *expiration > Timestamp::now() {
                Some(members.clone())
            } else {
                None
            }
        })
    }

    /// Updates the cache with new membership data and sets a fresh expiration time.
    pub fn update_cache(&mut self, group_id: &str, members: HashSet<PublicKey>) {
        let expiration = Timestamp::now() + self.cache_expiration;
        self.cache
            .insert(group_id.to_string(), (expiration, members));
    }
}

/// Constant for the Nostr event kind used for group admins (NIP-29).
const GROUP_ADMINS_KIND: Kind = Kind::Custom(39001);

/// Client for interacting with NIP-29 relays to manage group membership.
pub struct Nip29Client {
    relay_keys: Keys,
    cache: Arc<RwLock<GroupMembershipCache>>,
    client: Arc<Client>,
}

impl Nip29Client {
    /// Creates a new NIP-29 client instance.
    ///
    /// # Arguments
    /// - `relay_url`: The URL of the Nostr relay to connect to.
    /// - `keys`: The cryptographic keys for relay authentication.
    /// - `cache_expiration`: Duration in seconds for cache validity.
    ///
    /// # Errors
    /// Returns an error if the relay connection fails.
    pub async fn new(
        relay_url: String,
        keys: Keys,
        cache_expiration: u64,
    ) -> Result<Self, nostr_sdk::client::Error> {
        let opts = Options::default()
            .autoconnect(true)
            .automatic_authentication(true);

        let client = Client::builder().signer(keys.clone()).opts(opts).build();
        client.add_relay(&relay_url).await?;
        client.connect().await;

        Ok(Self {
            relay_keys: keys,
            cache: Arc::new(RwLock::new(GroupMembershipCache::new(cache_expiration))),
            client: Arc::new(client),
        })
    }

    /// Retrieves the set of members for a group, using the cache or fetching from the relay.
    async fn get_group_members(&self, group_id: &str) -> Result<HashSet<PublicKey>, Error> {
        // Try to read from cache using a read lock.
        {
            let cache = self.cache.read().await;
            if let Some(members) = cache.get_members(group_id) {
                return Ok(members);
            }
        }

        // Cache miss or expired; fetch from relay.
        let filter = Filter::new()
            .kind(GROUP_MEMBERSHIP_KIND)
            .pubkey(self.relay_keys.public_key())
            .identifier(group_id);

        let events = self
            .client
            .fetch_events(vec![filter], Some(FETCH_TIMEOUT))
            .await?;

        let mut members = HashSet::new();
        for event in events {
            // Extract public key tags using as_slice()
            for tag in event.tags {
                let vec = tag.as_slice();
                if vec[0] == "p" {
                    if let Ok(pubkey) = PublicKey::parse(&vec[1]) {
                        members.insert(pubkey);
                    }
                }
            }
        }

        if members.is_empty() {
            warn!("No valid member events found for group {}", group_id);
        }

        // Update the cache with a write lock.
        {
            let mut cache = self.cache.write().await;
            cache.update_cache(group_id, members.clone());
        }

        Ok(members)
    }

    /// Retrieves the set of admins for a group from the relay.
    ///
    /// # Arguments
    /// - `group_id`: The identifier of the group.
    ///
    /// # Returns
    /// - A HashSet of admin public keys for the group
    /// - Err if fetching events fails.
    async fn get_group_admins(&self, group_id: &str) -> Result<HashSet<PublicKey>, Error> {
        info!("Fetching admin list for group {}", group_id);
        
        // Fetch the kind 39001 (group admins) event for this group
        let filter = Filter::new()
            .kind(GROUP_ADMINS_KIND)
            .pubkey(self.relay_keys.public_key())
            .identifier(group_id);
            
        info!("Using filter: kind={:?}, pubkey={}, identifier={}", 
             GROUP_ADMINS_KIND, self.relay_keys.public_key(), group_id);

        let events = match self.client.fetch_events(vec![filter], Some(FETCH_TIMEOUT)).await {
            Ok(e) => {
                info!("Received {} events for group admin query", e.len());
                e
            },
            Err(e) => {
                warn!("Failed to fetch group admin events: {}", e);
                return Err(Error::msg(format!("Failed to fetch admin events: {e}")));
            }
        };

        // Extract admins from the most recent event
        let mut admins = HashSet::new();
        if let Some(event) = events.iter().max_by_key(|e| e.created_at) {
            info!("Processing admin event: id={}, created_at={}", 
                 event.id, event.created_at);
            info!("Event content: {}", event.content);
            info!("Event has {} tags", event.tags.len());
            
            for tag in event.tags.iter() {
                let vec = tag.as_slice();
                info!("Checking tag: {:?}", vec);
                
                if vec.len() >= 2 && vec[0] == "p" {
                    match PublicKey::parse(&vec[1]) {
                        Ok(pubkey) => {
                            info!("Found admin pubkey: {}", pubkey);
                            admins.insert(pubkey);
                        },
                        Err(e) => {
                            warn!("Failed to parse pubkey '{}': {}", vec[1], e);
                        }
                    }
                }
            }
        } else {
            warn!("No events found for group admin query");
        }

        if admins.is_empty() {
            warn!("No admins found for group {}", group_id);
        } else {
            info!("Found {} admins for group {}", admins.len(), group_id);
        }

        Ok(admins)
    }

    /// Determines if a public key is a member of a specified group.
    ///
    /// # Arguments
    /// - `group_id`: The identifier of the group.
    /// - `pubkey`: The public key to check membership for.
    ///
    /// # Returns
    /// - `Ok(true)` if the public key is a member.
    /// - `Ok(false)` if not a member or no membership data exists.
    /// - `Err` if fetching events fails.
    pub async fn is_group_member(&self, group_id: &str, pubkey: &PublicKey) -> Result<bool, Error> {
        let members = self.get_group_members(group_id).await?;
        Ok(members.contains(pubkey))
    }

    /// Checks if a public key is an admin of a group (from kind:39001 events)
    ///
    /// # Arguments
    /// - `group_id`: The identifier of the group.
    /// - `pubkey`: The public key to check admin status for.
    ///
    /// # Returns
    /// - `Ok(true)` if the public key is an admin
    /// - `Ok(false)` if not an admin or no admin data exists
    /// - `Err` if fetching events fails
    pub async fn is_group_admin(&self, group_id: &str, pubkey: &PublicKey) -> Result<bool, Error> {
        info!("Checking if {} is admin of group {}", pubkey, group_id);
        let admins = match self.get_group_admins(group_id).await {
            Ok(a) => {
                info!("Found {} admins for group {}", a.len(), group_id);
                for admin in &a {
                    info!("Admin: {}", admin);
                }
                a
            },
            Err(e) => {
                info!("Error getting admins for group {}: {}", group_id, e);
                return Err(e);
            }
        };
        let is_admin = admins.contains(pubkey);
        info!("User {} is {} admin of group {}", pubkey, if is_admin { "an" } else { "not an" }, group_id);
        Ok(is_admin)
    }
}

/// Initializes a NIP-29 client based on application settings.
///
/// # Arguments
/// - `settings`: Configuration settings containing NIP-29 relay details.
///
/// # Returns
/// - `Some(Arc<Nip29Client>)` if initialization succeeds.
/// - `None` if configuration is missing or initialization fails.
pub async fn init_nip29_client(settings: &Settings) -> Result<Arc<Nip29Client>> {
    let nip29_config = &settings.nip29_relay;

    let keys = Keys::parse(&nip29_config.private_key)?;

    let cache_expiration = nip29_config.cache_expiration.unwrap_or(300);

    let nip_29_client = Nip29Client::new(nip29_config.url.clone(), keys, cache_expiration).await?;

    info!(
        "NIP-29 client initialized with relay URL: {}",
        nip29_config.url
    );

    Ok(Arc::new(nip_29_client))
}
