//! Account manager (task 5).
//!
//! Ties together the [`store::TokenStorage`] backend, an [`transport::AuthTransport`]
//! and the Microsoft / offline flows into a single, easy-to-drive API used by
//! the Compose UI and FFI layer. It also performs **proactive token refresh**:
//! [`AccountManager::ensure_fresh`] transparently re-mints a Microsoft
//! account's Minecraft token before it (or its underlying MS token) expires.

use std::sync::Arc;
use std::time::Duration;

use crate::auth::microsoft::{self, DeviceCodeChallenge, DEFAULT_CLIENT_ID, DEFAULT_SCOPE};
use crate::auth::model::{now_secs, Account};
use crate::auth::offline::offline_account_model;
use crate::auth::store::TokenStorage;
use crate::auth::transport::{AuthTransport, ReqwestTransport};
use crate::error::RcResult;

/// How long before expiry we proactively refresh a Microsoft token.
pub const REFRESH_THRESHOLD_SECS: u64 = 300;

/// Default polling timeout for the device-code flow.
pub const DEVICE_CODE_TIMEOUT: Duration = Duration::from_secs(900);

/// In-memory account store + async transport for the Microsoft flows.
pub struct AccountManager {
    storage: Box<dyn TokenStorage>,
    transport: Arc<dyn AuthTransport>,
    client_id: String,
    accounts: Vec<Account>,
}

impl AccountManager {
    /// Build a manager, loading any persisted accounts from `storage`.
    pub fn new(
        storage: Box<dyn TokenStorage>,
        transport: Arc<dyn AuthTransport>,
        client_id: impl Into<String>,
    ) -> RcResult<Self> {
        let accounts = storage.load()?;
        Ok(Self {
            storage,
            transport,
            client_id: client_id.into(),
            accounts,
        })
    }

    /// Build a manager with a standalone (network) transport and the default
    /// public MSA client id.
    pub fn with_defaults(storage: Box<dyn TokenStorage>) -> RcResult<Self> {
        let transport = Arc::new(ReqwestTransport::with_defaults()?);
        Self::new(storage, transport, DEFAULT_CLIENT_ID)
    }

    /// Override the OAuth client id (e.g. a self-registered Azure app).
    pub fn set_client_id(&mut self, client_id: impl Into<String>) {
        self.client_id = client_id.into();
    }

    pub fn client_id(&self) -> &str {
        &self.client_id
    }

    /// All accounts (with secrets — use [`AccountManager::summaries`] for UI).
    pub fn accounts(&self) -> &[Account] {
        &self.accounts
    }

    /// Redacted accounts safe to send to the UI / FFI layer.
    pub fn summaries(&self) -> Vec<Account> {
        self.accounts.iter().map(|a| a.summary()).collect()
    }

    /// Find an account by uuid.
    pub fn find(&self, uuid: &str) -> Option<&Account> {
        self.accounts.iter().find(|a| a.uuid() == uuid)
    }

    fn persist(&self) -> RcResult<()> {
        self.storage.save(&self.accounts)
    }

    /// Add an offline account. Returns the stored account (clone).
    pub fn add_offline(&mut self, username: &str) -> RcResult<Account> {
        if username.trim().is_empty() {
            return Err(crate::error::RcError::Auth(
                "offline username must not be empty".into(),
            ));
        }
        let acc = offline_account_model(username);
        self.accounts.push(acc.clone());
        self.persist()?;
        Ok(acc)
    }

    /// Remove an account by uuid. Returns true if something was removed.
    pub fn remove(&mut self, uuid: &str) -> RcResult<bool> {
        let before = self.accounts.len();
        self.accounts.retain(|a| a.uuid() != uuid);
        let removed = self.accounts.len() != before;
        if removed {
            self.persist()?;
        }
        Ok(removed)
    }

    /// Step 1 of Microsoft login: obtain a device-code challenge for the UI.
    pub async fn begin_microsoft(&self) -> RcResult<DeviceCodeChallenge> {
        let c =
            microsoft::request_device_code(self.transport.as_ref(), &self.client_id, DEFAULT_SCOPE)
                .await?;
        Ok(c)
    }

    /// Steps 2–6: complete the device-code flow and store the resulting
    /// account. `on_pending` is called between polls (e.g. UI progress).
    pub async fn complete_microsoft(
        &mut self,
        challenge: &DeviceCodeChallenge,
        on_pending: impl Fn(u64),
    ) -> RcResult<Account> {
        let acc = microsoft::authenticate_device_code(
            self.transport.as_ref(),
            &self.client_id,
            challenge,
            DEVICE_CODE_TIMEOUT,
            on_pending,
        )
        .await?;
        let account = Account::Microsoft(acc);
        self.accounts.push(account.clone());
        self.persist()?;
        Ok(account)
    }

    /// Refresh a stored Microsoft account from its refresh token.
    pub async fn refresh(&mut self, uuid: &str) -> RcResult<Account> {
        let idx = self
            .accounts
            .iter()
            .position(|a| a.uuid() == uuid)
            .ok_or_else(|| crate::error::RcError::Auth(format!("no account with uuid {uuid}")))?;
        let ms = match &self.accounts[idx] {
            Account::Microsoft(m) => m.clone(),
            Account::Offline(_) => {
                return Err(crate::error::RcError::Auth(
                    "cannot refresh an offline account".into(),
                ));
            }
        };
        let refreshed = microsoft::refresh_account(self.transport.as_ref(), &ms).await?;
        let account = Account::Microsoft(refreshed);
        self.accounts[idx] = account.clone();
        self.persist()?;
        Ok(account)
    }

    /// Return a fresh copy of `uuid`'s account, transparently refreshing the
    /// Microsoft token if it (or its underlying MS token) is within
    /// [`REFRESH_THRESHOLD_SECS`] of expiry. Offline accounts are returned as-is.
    pub async fn ensure_fresh(&mut self, uuid: &str) -> RcResult<Account> {
        let needs = match self.find(uuid) {
            Some(Account::Microsoft(m)) => m.needs_refresh(now_secs(), REFRESH_THRESHOLD_SECS),
            Some(Account::Offline(_)) => false,
            None => {
                return Err(crate::error::RcError::Auth(format!(
                    "no account with uuid {uuid}"
                )))
            }
        };
        if needs {
            self.refresh(uuid).await
        } else {
            Ok(self.find(uuid).unwrap().clone())
        }
    }

    /// Convenience predicate: is the account a Microsoft account whose token is
    /// currently expired?
    pub fn is_expired(&self, uuid: &str) -> bool {
        match self.find(uuid) {
            Some(Account::Microsoft(m)) => m.is_expired(now_secs()),
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::store::MemoryTokenStorage;
    use crate::auth::transport::MockTransport;

    fn device_code_json() -> serde_json::Value {
        serde_json::json!({
            "user_code":"ABCD","device_code":"dc","verification_uri":"https://x",
            "expires_in":900,"interval":1,"message":"go"
        })
    }

    /// A transport with exactly one full device-code flow scripted.
    fn mock_transport_once() -> Arc<dyn AuthTransport> {
        let m = MockTransport::new();
        m.script_ok(microsoft::DEVICE_CODE_URL, device_code_json());
        m.script_ok(
            microsoft::TOKEN_URL,
            serde_json::json!({"access_token":"ms","refresh_token":"rt","expires_in":3600}),
        );
        m.script_ok(
            microsoft::XBL_AUTH_URL,
            serde_json::json!({"Token":"xbl","DisplayClaims":{"xui":[{"uhs":"U"}]}}),
        );
        m.script_ok(
            microsoft::XSTS_AUTH_URL,
            serde_json::json!({"Token":"xsts","DisplayClaims":{"xui":[{"uhs":"U"}]}}),
        );
        m.script_ok(
            microsoft::MC_LOGIN_URL,
            serde_json::json!({"access_token":"mc","expires_in":86400}),
        );
        m.script_ok(
            microsoft::MC_PROFILE_URL,
            serde_json::json!({"id":"uuid-1","name":"Player"}),
        );
        Arc::new(m)
    }

    /// A transport with TWO full device-code flows scripted, where the second
    /// profile answers with a different name so we can prove a refresh ran.
    fn mock_transport_twice() -> Arc<dyn AuthTransport> {
        let m = MockTransport::new();
        for _ in 0..2 {
            m.script_ok(microsoft::DEVICE_CODE_URL, device_code_json());
            m.script_ok(
                microsoft::TOKEN_URL,
                serde_json::json!({"access_token":"ms","refresh_token":"rt","expires_in":3600}),
            );
            m.script_ok(
                microsoft::XBL_AUTH_URL,
                serde_json::json!({"Token":"xbl","DisplayClaims":{"xui":[{"uhs":"U"}]}}),
            );
            m.script_ok(
                microsoft::XSTS_AUTH_URL,
                serde_json::json!({"Token":"xsts","DisplayClaims":{"xui":[{"uhs":"U"}]}}),
            );
            m.script_ok(
                microsoft::MC_LOGIN_URL,
                serde_json::json!({"access_token":"mc","expires_in":86400}),
            );
        }
        // Profiles are consumed in order: first complete -> Player, then
        // refresh -> Player2 (proves the refresh re-ran the token chain).
        m.script_ok(
            microsoft::MC_PROFILE_URL,
            serde_json::json!({"id":"uuid-1","name":"Player"}),
        );
        m.script_ok(
            microsoft::MC_PROFILE_URL,
            serde_json::json!({"id":"uuid-1","name":"Player2"}),
        );
        Arc::new(m)
    }

    fn manager() -> AccountManager {
        let t = mock_transport_once();
        AccountManager::new(Box::new(MemoryTokenStorage::new()), t, DEFAULT_CLIENT_ID).unwrap()
    }

    #[tokio::test]
    async fn offline_add_and_list() {
        let mut mg = manager();
        let a = mg.add_offline("Steve").unwrap();
        assert_eq!(a.username(), "Steve");
        assert_eq!(mg.accounts().len(), 1);
        assert!(matches!(a, Account::Offline(_)));
    }

    #[tokio::test]
    async fn full_microsoft_flow_stores_account() {
        let mut mg = manager();
        let challenge = mg.begin_microsoft().await.unwrap();
        assert_eq!(challenge.user_code, "ABCD");
        let acc = mg.complete_microsoft(&challenge, |_| {}).await.unwrap();
        assert_eq!(acc.uuid(), "uuid-1");
        assert_eq!(acc.username(), "Player");
        assert_eq!(mg.accounts().len(), 1);
    }

    #[tokio::test]
    async fn remove_account() {
        let mut mg = manager();
        let a = mg.add_offline("Steve").unwrap();
        assert!(mg.remove(a.uuid()).unwrap());
        assert!(!mg.remove(a.uuid()).unwrap());
        assert!(mg.accounts().is_empty());
    }

    #[tokio::test]
    async fn ensure_fresh_refreshes_when_expired() {
        let transport = mock_transport_twice();
        let mut mg = AccountManager::new(
            Box::new(MemoryTokenStorage::new()),
            transport,
            DEFAULT_CLIENT_ID,
        )
        .unwrap();
        let challenge = mg.begin_microsoft().await.unwrap();
        let acc = mg.complete_microsoft(&challenge, |_| {}).await.unwrap();
        assert_eq!(acc.username(), "Player");
        // Force expiry of the stored MS token.
        if let Account::Microsoft(m) = &mut mg.accounts[0] {
            m.expires_at = 1;
            m.ms_expires_at = 1;
        }
        let refreshed = mg.ensure_fresh(acc.uuid()).await.unwrap();
        // The profile name changed -> the refresh genuinely re-ran the chain.
        assert_eq!(refreshed.username(), "Player2");
    }

    #[tokio::test]
    async fn ensure_fresh_keeps_valid_token() {
        let mut mg = manager();
        let challenge = mg.begin_microsoft().await.unwrap();
        let acc = mg.complete_microsoft(&challenge, |_| {}).await.unwrap();
        // Token is fresh (expires_in ~3600s) -> no refresh, name unchanged.
        let kept = mg.ensure_fresh(acc.uuid()).await.unwrap();
        assert_eq!(kept.username(), "Player");
    }

    #[test]
    fn summaries_redact_secrets() {
        let mut mg = manager();
        let a = mg.add_offline("Steve").unwrap();
        let sum = mg.summaries();
        assert_eq!(sum.len(), 1);
        assert_eq!(sum[0].uuid(), a.uuid());
    }

    #[test]
    fn empty_username_rejected() {
        let mut mg = manager();
        assert!(matches!(
            mg.add_offline("   "),
            Err(crate::error::RcError::Auth(_))
        ));
    }

    #[test]
    fn refresh_rejects_offline() {
        let mut mg = manager();
        let a = mg.add_offline("Steve").unwrap();
        // Need a tokio runtime; use a quick block_on via a fresh manager future.
        let rt = tokio::runtime::Runtime::new().unwrap();
        let err = rt.block_on(mg.refresh(a.uuid()));
        assert!(err.is_err());
    }
}
