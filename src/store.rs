use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use rand::{RngCore, rngs::OsRng};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use subtle::ConstantTimeEq;
use uuid::Uuid;

const START_TOKEN_BYTES: usize = 24;

pub struct PairingStore {
    sessions: HashMap<Uuid, PairingSession>,
    start_index: HashMap<[u8; 32], Uuid>,
    pairing_ttl_seconds: u64,
    poll_interval_seconds: u64,
    max_active_pairings: usize,
    max_exchange_attempts: u8,
}

struct PairingSession {
    source: String,
    code_challenge: [u8; 32],
    start_token_digest: [u8; 32],
    expires_at: u64,
    approved_user_id: Option<i64>,
    consumed: bool,
    exchange_attempts: u8,
    last_exchange_at: Option<Instant>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct CreatedPairing {
    pub session_id: Uuid,
    pub start_token: String,
    pub expires_in: u64,
    pub interval: u64,
}

#[derive(Debug, PartialEq, Eq)]
pub enum CreateOutcome {
    Created(CreatedPairing),
    CapacityExceeded,
}

#[derive(Debug, PartialEq, Eq)]
pub enum ApproveOutcome {
    Approved(String),
    AlreadyApproved(String),
    DifferentUser,
    Expired,
    Consumed,
    NotFound,
}

#[derive(Debug, PartialEq, Eq)]
pub enum ExchangeOutcome {
    Pending,
    SlowDown,
    Approved {
        telegram_user_id: i64,
        source: String,
    },
    InvalidVerifier,
    TooManyAttempts,
    Expired,
    Consumed,
    NotFound,
}

impl PairingStore {
    pub fn new(
        pairing_ttl_seconds: u64,
        poll_interval_seconds: u64,
        max_active_pairings: usize,
        max_exchange_attempts: u8,
    ) -> Self {
        Self {
            sessions: HashMap::new(),
            start_index: HashMap::new(),
            pairing_ttl_seconds,
            poll_interval_seconds,
            max_active_pairings,
            max_exchange_attempts,
        }
    }

    pub fn create(&mut self, source: String, code_challenge: [u8; 32]) -> CreateOutcome {
        self.remove_expired();
        if self.sessions.len() >= self.max_active_pairings {
            return CreateOutcome::CapacityExceeded;
        }

        let session_id = Uuid::new_v4();
        let start_token = random_token(START_TOKEN_BYTES);
        let start_token_digest = digest(start_token.as_bytes());
        let expires_at = unix_now().saturating_add(self.pairing_ttl_seconds);

        self.sessions.insert(
            session_id,
            PairingSession {
                source,
                code_challenge,
                start_token_digest,
                expires_at,
                approved_user_id: None,
                consumed: false,
                exchange_attempts: 0,
                last_exchange_at: None,
            },
        );
        self.start_index.insert(start_token_digest, session_id);

        CreateOutcome::Created(CreatedPairing {
            session_id,
            start_token,
            expires_in: self.pairing_ttl_seconds,
            interval: self.poll_interval_seconds,
        })
    }

    pub fn approve(&mut self, start_token: &str, user_id: i64) -> ApproveOutcome {
        let start_token_digest = digest(start_token.as_bytes());
        let Some(session_id) = self.start_index.get(&start_token_digest).copied() else {
            return ApproveOutcome::NotFound;
        };

        let expired = self
            .sessions
            .get(&session_id)
            .is_some_and(|session| session.expires_at <= unix_now());
        if expired {
            self.sessions.remove(&session_id);
            self.start_index.remove(&start_token_digest);
            return ApproveOutcome::Expired;
        }

        let Some(session) = self.sessions.get_mut(&session_id) else {
            self.start_index.remove(&start_token_digest);
            return ApproveOutcome::NotFound;
        };
        if session.consumed {
            return ApproveOutcome::Consumed;
        }
        match session.approved_user_id {
            None => {
                session.approved_user_id = Some(user_id);
                ApproveOutcome::Approved(session.source.clone())
            }
            Some(existing) if existing == user_id => {
                ApproveOutcome::AlreadyApproved(session.source.clone())
            }
            Some(_) => ApproveOutcome::DifferentUser,
        }
    }

    pub fn exchange(&mut self, session_id: Uuid, verifier: &str) -> ExchangeOutcome {
        let expired_digest = self.sessions.get(&session_id).and_then(|session| {
            (session.expires_at <= unix_now()).then_some(session.start_token_digest)
        });
        if let Some(start_token_digest) = expired_digest {
            self.sessions.remove(&session_id);
            self.start_index.remove(&start_token_digest);
            return ExchangeOutcome::Expired;
        }

        let Some(session) = self.sessions.get_mut(&session_id) else {
            return ExchangeOutcome::NotFound;
        };
        if session.consumed {
            return ExchangeOutcome::Consumed;
        }
        if session.last_exchange_at.is_some_and(|previous| {
            previous.elapsed() < Duration::from_secs(self.poll_interval_seconds)
        }) {
            return ExchangeOutcome::SlowDown;
        }
        session.last_exchange_at = Some(Instant::now());

        let candidate = digest(verifier.as_bytes());
        if !bool::from(candidate.ct_eq(&session.code_challenge)) {
            session.exchange_attempts = session.exchange_attempts.saturating_add(1);
            if session.exchange_attempts >= self.max_exchange_attempts {
                session.consumed = true;
                return ExchangeOutcome::TooManyAttempts;
            }
            return ExchangeOutcome::InvalidVerifier;
        }

        let Some(telegram_user_id) = session.approved_user_id else {
            return ExchangeOutcome::Pending;
        };
        session.consumed = true;
        self.start_index.remove(&session.start_token_digest);
        ExchangeOutcome::Approved {
            telegram_user_id,
            source: session.source.clone(),
        }
    }

    fn remove_expired(&mut self) {
        let now = unix_now();
        self.sessions.retain(|_, session| session.expires_at > now);
        self.start_index
            .retain(|_, session_id| self.sessions.contains_key(session_id));
    }
}

pub fn decode_challenge(value: &str) -> Option<[u8; 32]> {
    let decoded = URL_SAFE_NO_PAD.decode(value).ok()?;
    decoded.try_into().ok()
}

fn random_token(bytes: usize) -> String {
    let mut value = vec![0_u8; bytes];
    OsRng.fill_bytes(&mut value);
    URL_SAFE_NO_PAD.encode(value)
}

fn digest(value: &[u8]) -> [u8; 32] {
    Sha256::digest(value).into()
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

    fn verifier_and_challenge() -> (String, [u8; 32]) {
        let verifier = random_token(32);
        let challenge = digest(verifier.as_bytes());
        (verifier, challenge)
    }

    #[test]
    fn exchanges_only_after_approval() {
        let (verifier, challenge) = verifier_and_challenge();
        let mut store = PairingStore::new(300, 0, 10, 5);
        let CreateOutcome::Created(created) = store.create("primary".into(), challenge) else {
            panic!("pairing was not created");
        };

        assert_eq!(
            store.exchange(created.session_id, &verifier),
            ExchangeOutcome::Pending
        );
        assert_eq!(
            store.approve(&created.start_token, 42),
            ApproveOutcome::Approved("primary".into())
        );
        assert_eq!(
            store.exchange(created.session_id, &verifier),
            ExchangeOutcome::Approved {
                telegram_user_id: 42,
                source: "primary".into(),
            }
        );
        assert_eq!(
            store.exchange(created.session_id, &verifier),
            ExchangeOutcome::Consumed
        );
    }

    #[test]
    fn independent_sources_do_not_replace_each_other() {
        let (first_verifier, first_challenge) = verifier_and_challenge();
        let (second_verifier, second_challenge) = verifier_and_challenge();
        let mut store = PairingStore::new(300, 0, 10, 5);
        let CreateOutcome::Created(first) = store.create("primary".into(), first_challenge) else {
            panic!("first pairing was not created");
        };
        let CreateOutcome::Created(second) = store.create("secondary".into(), second_challenge) else {
            panic!("second pairing was not created");
        };

        assert!(matches!(
            store.approve(&first.start_token, 42),
            ApproveOutcome::Approved(source) if source == "primary"
        ));
        assert!(matches!(
            store.approve(&second.start_token, 42),
            ApproveOutcome::Approved(source) if source == "secondary"
        ));
        assert!(matches!(
            store.exchange(first.session_id, &first_verifier),
            ExchangeOutcome::Approved { source, .. } if source == "primary"
        ));
        assert!(matches!(
            store.exchange(second.session_id, &second_verifier),
            ExchangeOutcome::Approved { source, .. } if source == "secondary"
        ));
    }

    #[test]
    fn rejects_wrong_verifier() {
        let (_, challenge) = verifier_and_challenge();
        let mut store = PairingStore::new(300, 0, 10, 5);
        let CreateOutcome::Created(created) = store.create("primary".into(), challenge) else {
            panic!("pairing was not created");
        };

        assert_eq!(
            store.exchange(created.session_id, "wrong"),
            ExchangeOutcome::InvalidVerifier
        );
    }

    #[test]
    fn decodes_sha256_challenge() {
        let challenge = URL_SAFE_NO_PAD.encode([7_u8; 32]);
        assert_eq!(decode_challenge(&challenge), Some([7_u8; 32]));
        assert!(decode_challenge("invalid").is_none());
    }
}
