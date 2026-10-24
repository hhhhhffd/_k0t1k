use md5::{Digest, Md5};

/// The identity passed to the game.
#[derive(Clone)]
pub struct Account {
    pub name: String,
    /// 32 hex characters, no dashes.
    pub uuid: String,
    pub access_token: String,
    /// `${auth_session}`: `-` for offline accounts, `token:<access token>:<uuid>` otherwise.
    pub session: String,
    /// `${user_type}`: `offline`, `msa` or `mojang`.
    pub user_type: String,
}

impl Account {
    pub fn offline(name: &str) -> Self {
        Self {
            name: name.to_owned(),
            uuid: offline_uuid(name),
            access_token: "0".to_owned(),
            session: "-".to_owned(),
            user_type: "offline".to_owned(),
        }
    }
}

impl Account {
    /// A signed-in account; `user_type` is `msa` for Microsoft and `mojang` for Yggdrasil servers.
    pub fn authenticated(name: &str, uuid: &str, access_token: &str, user_type: &str) -> Self {
        Self {
            name: name.to_owned(),
            uuid: uuid.to_owned(),
            access_token: access_token.to_owned(),
            session: format!("token:{access_token}:{uuid}"),
            user_type: user_type.to_owned(),
        }
    }
}

// The access token must never end up in logs, so it is left out of the debug output.
impl std::fmt::Debug for Account {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Account")
            .field("name", &self.name)
            .field("uuid", &self.uuid)
            .field("user_type", &self.user_type)
            .finish_non_exhaustive()
    }
}

/// Java's `UUID.nameUUIDFromBytes("OfflinePlayer:<name>")` as 32 hex characters without dashes.
pub fn offline_uuid(name: &str) -> String {
    let mut digest: [u8; 16] = Md5::digest(format!("OfflinePlayer:{name}")).into();
    digest[6] = (digest[6] & 0x0f) | 0x30;
    digest[8] = (digest[8] & 0x3f) | 0x80;
    hex::encode(digest)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offline_uuid_matches_java_name_uuid_from_bytes() {
        // Computed with `UUID.nameUUIDFromBytes(("OfflinePlayer:" + name).getBytes("UTF-8"))`.
        assert_eq!(offline_uuid("Notch"), "b50ad385829d3141a2167e7d7539ba7f");
        assert_eq!(offline_uuid("Player"), "a01e3843e5213998958af459800e4d11");
    }

    #[test]
    fn debug_output_hides_the_token() {
        let mut account = Account::offline("Notch");
        account.access_token = "secret-token".to_owned();
        assert!(!format!("{account:?}").contains("secret-token"));
    }
}
