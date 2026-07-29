use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

const DOCUMENT_VERSION: u8 = 1;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct Subscriber {
    user_id: i64,
    chat_id: i64,
    subscribed_at: u64,
}

#[derive(Debug, Serialize, Deserialize)]
struct SubscriberDocument {
    version: u8,
    subscribers: Vec<Subscriber>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum SubscribeOutcome {
    Added,
    Updated,
    AlreadySubscribed,
}

pub struct SubscriberStore {
    path: PathBuf,
    subscribers: BTreeMap<i64, Subscriber>,
}

impl SubscriberStore {
    pub fn load(path: PathBuf) -> Result<Self> {
        if !path.exists() {
            return Ok(Self {
                path,
                subscribers: BTreeMap::new(),
            });
        }

        let content = fs::read_to_string(&path)
            .with_context(|| format!("read subscriber store {}", path.display()))?;
        let document: SubscriberDocument = serde_json::from_str(&content)
            .with_context(|| format!("decode subscriber store {}", path.display()))?;
        if document.version != DOCUMENT_VERSION {
            bail!("unsupported subscriber store version {}", document.version);
        }

        let mut subscribers = BTreeMap::new();
        for subscriber in document.subscribers {
            if subscriber.user_id == 0 || subscriber.chat_id == 0 {
                bail!("subscriber store contains an invalid Telegram identity");
            }
            if subscribers.insert(subscriber.user_id, subscriber).is_some() {
                bail!("subscriber store contains a duplicate Telegram user");
            }
        }

        Ok(Self { path, subscribers })
    }

    pub fn len(&self) -> usize {
        self.subscribers.len()
    }

    pub fn is_subscribed(&self, user_id: i64) -> bool {
        self.subscribers.contains_key(&user_id)
    }

    pub fn chat_ids(&self) -> Vec<i64> {
        self.subscribers
            .values()
            .map(|subscriber| subscriber.chat_id)
            .collect()
    }

    pub fn subscribe(&mut self, user_id: i64, chat_id: i64) -> Result<SubscribeOutcome> {
        if user_id == 0 || chat_id == 0 {
            bail!("Telegram user and chat ids must not be zero");
        }
        let previous = self.subscribers.get(&user_id).cloned();
        let outcome = match previous.as_ref() {
            None => SubscribeOutcome::Added,
            Some(existing) if existing.chat_id == chat_id => SubscribeOutcome::AlreadySubscribed,
            Some(_) => SubscribeOutcome::Updated,
        };
        if outcome == SubscribeOutcome::AlreadySubscribed {
            return Ok(outcome);
        }

        self.subscribers.insert(
            user_id,
            Subscriber {
                user_id,
                chat_id,
                subscribed_at: unix_now(),
            },
        );
        if let Err(error) = self.persist() {
            match previous {
                Some(previous) => {
                    self.subscribers.insert(user_id, previous);
                }
                None => {
                    self.subscribers.remove(&user_id);
                }
            }
            return Err(error);
        }
        Ok(outcome)
    }

    pub fn unsubscribe(&mut self, user_id: i64) -> Result<bool> {
        let previous = self.subscribers.remove(&user_id);
        let Some(previous) = previous else {
            return Ok(false);
        };
        if let Err(error) = self.persist() {
            self.subscribers.insert(user_id, previous);
            return Err(error);
        }
        Ok(true)
    }

    fn persist(&self) -> Result<()> {
        let parent = self
            .path
            .parent()
            .context("subscriber store path has no parent")?;
        fs::create_dir_all(parent)
            .with_context(|| format!("create subscriber store directory {}", parent.display()))?;
        let document = SubscriberDocument {
            version: DOCUMENT_VERSION,
            subscribers: self.subscribers.values().cloned().collect(),
        };
        let content = serde_json::to_vec_pretty(&document).context("encode subscriber store")?;
        let temporary = temporary_path(parent);
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .with_context(|| {
                format!("create temporary subscriber store {}", temporary.display())
            })?;
        set_private_permissions(&file)?;
        file.write_all(&content)
            .context("write temporary subscriber store")?;
        file.sync_all().context("sync temporary subscriber store")?;
        fs::rename(&temporary, &self.path)
            .with_context(|| format!("replace subscriber store {}", self.path.display()))?;
        Ok(())
    }
}

fn temporary_path(parent: &Path) -> PathBuf {
    parent.join(format!(
        ".subscribers-{}-{}.tmp",
        std::process::id(),
        unix_now()
    ))
}

#[cfg(unix)]
fn set_private_permissions(file: &fs::File) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    file.set_permissions(fs::Permissions::from_mode(0o600))
        .context("set subscriber store permissions")
}

#[cfg(not(unix))]
fn set_private_permissions(_file: &fs::File) -> Result<()> {
    Ok(())
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn persists_multiple_subscribers_and_unsubscribes_independently() {
        let root = std::env::temp_dir().join(format!(
            "infrabot-subscribers-{}-{}",
            std::process::id(),
            unix_now()
        ));
        let path = root.join("subscribers.json");
        let mut store = SubscriberStore::load(path.clone()).unwrap();
        assert_eq!(store.subscribe(42, 42).unwrap(), SubscribeOutcome::Added);
        assert_eq!(store.subscribe(77, 77).unwrap(), SubscribeOutcome::Added);
        assert_eq!(store.len(), 2);

        let mut restored = SubscriberStore::load(path).unwrap();
        assert!(restored.is_subscribed(42));
        assert!(restored.is_subscribed(77));
        assert!(restored.unsubscribe(42).unwrap());
        assert!(!restored.is_subscribed(42));
        assert!(restored.is_subscribed(77));
        let _ = fs::remove_dir_all(root);
    }
}
