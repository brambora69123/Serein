//! Bounded current-account OAuth2 authorization metadata. This is a session-only view; no
//! access or refresh token is retained, and absent or unrecognized fields stay unknown.
pub const MAX_AUTHORIZED_APPS: usize = 64;
pub const MAX_AUTHORIZED_APPS_WIRE: usize = 256 * 1024;
pub const MAX_AUTHORIZED_APPS_BYTES: usize = 512 * 1024;
pub const MAX_AUTHORIZATION_ID_CHARS: usize = 32;
pub const MAX_APPLICATION_ID_CHARS: usize = 32;
pub const MAX_APPLICATION_NAME_CHARS: usize = 128;
pub const MAX_APPLICATION_DESCRIPTION_CHARS: usize = 1024;
pub const MAX_APPLICATION_ICON_CHARS: usize = 128;
pub const MAX_SCOPES: usize = 64;
pub const MAX_SCOPE_CHARS: usize = 64;
pub const MAX_SCOPES_BYTES: usize = 4 * 1024;
pub const MAX_POLICY_URL_CHARS: usize = 512;
pub const MAX_AUTHORIZED_AT_CHARS: usize = 64;

/// The partial application attached to one authorization. Optional fields remain optional so a
/// missing or unrecognized value is shown as unknown rather than invented.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthorizedApplication {
	pub id: String,
	pub name: String,
	pub description: Option<String>,
	pub icon: Option<String>,
	pub is_verified: Option<bool>,
	pub terms_of_service_url: Option<String>,
	pub privacy_policy_url: Option<String>,
}
impl AuthorizedApplication {
	pub fn bytes(&self) -> usize {
		size_of::<Self>()
			+ self.id.capacity()
			+ self.name.capacity()
			+ self.description.as_ref().map_or(0, String::capacity)
			+ self.icon.as_ref().map_or(0, String::capacity)
			+ self
				.terms_of_service_url
				.as_ref()
				.map_or(0, String::capacity)
			+ self.privacy_policy_url.as_ref().map_or(0, String::capacity)
	}
	pub fn valid(&self) -> bool {
		valid_application_id(&self.id)
			&& !self.name.is_empty()
			&& self.name.chars().count() <= MAX_APPLICATION_NAME_CHARS
			&& self
				.description
				.as_ref()
				.is_none_or(|value| value.chars().count() <= MAX_APPLICATION_DESCRIPTION_CHARS)
			&& self
				.icon
				.as_ref()
				.is_none_or(|value| value.chars().count() <= MAX_APPLICATION_ICON_CHARS)
			&& self
				.terms_of_service_url
				.as_ref()
				.is_none_or(|value| value.chars().count() <= MAX_POLICY_URL_CHARS)
			&& self
				.privacy_policy_url
				.as_ref()
				.is_none_or(|value| value.chars().count() <= MAX_POLICY_URL_CHARS)
	}
}

/// One OAuth2 authorization from `GET /oauth2/tokens`. `scopes` is `None` when the service did
/// not report a usable list, which is distinct from an explicitly empty list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthorizedAppGrant {
	pub id: String,
	pub application: AuthorizedApplication,
	pub scopes: Option<Vec<String>>,
	pub authorized_at: Option<String>,
	/// At least one presentation field was truncated or dropped to stay in bounds.
	pub limited: bool,
}
impl AuthorizedAppGrant {
	pub fn bytes(&self) -> usize {
		size_of::<Self>()
			+ self.id.capacity()
			+ self.application.bytes()
			+ self.scopes.as_ref().map_or(0, |scopes| {
				scopes.capacity() * size_of::<String>()
					+ scopes.iter().map(String::capacity).sum::<usize>()
			}) + self.authorized_at.as_ref().map_or(0, String::capacity)
	}
	pub fn valid(&self) -> bool {
		valid_authorization_id(&self.id)
			&& self.application.valid()
			&& self.scopes.as_ref().is_none_or(|scopes| {
				scopes.len() <= MAX_SCOPES
					&& scopes
						.iter()
						.all(|scope| !scope.is_empty() && scope.chars().count() <= MAX_SCOPE_CHARS)
					&& scopes.iter().map(String::capacity).sum::<usize>() <= MAX_SCOPES_BYTES
			}) && self
			.authorized_at
			.as_ref()
			.is_none_or(|value| value.chars().count() <= MAX_AUTHORIZED_AT_CHARS)
			&& self.bytes() <= MAX_AUTHORIZED_APPS_BYTES
	}
	/// Whether this row has enough bounded state to offer a real deauthorization control.
	pub fn can_deauthorize(&self) -> bool {
		!self.limited && valid_authorization_id(&self.id)
	}
}

/// The bounded current-account authorization list and whether any row lost a field.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AuthorizedAppsPage {
	pub items: Vec<AuthorizedAppGrant>,
	pub limited: bool,
}
impl AuthorizedAppsPage {
	pub fn bytes(&self) -> usize {
		size_of::<Self>()
			+ self.items.capacity() * size_of::<AuthorizedAppGrant>()
			+ self
				.items
				.iter()
				.map(AuthorizedAppGrant::bytes)
				.sum::<usize>()
	}
	pub fn valid(&self) -> bool {
		self.items.len() <= MAX_AUTHORIZED_APPS
			&& self.items.iter().all(AuthorizedAppGrant::valid)
			&& self.bytes() <= MAX_AUTHORIZED_APPS_BYTES
	}
}

/// Authorization IDs are snowflakes. Anything outside the digit alphabet is rejected rather
/// than escaped into a different route.
pub fn valid_authorization_id(id: &str) -> bool {
	!id.is_empty()
		&& id.len() <= MAX_AUTHORIZATION_ID_CHARS
		&& id.bytes().all(|byte| byte.is_ascii_digit())
}

/// Application IDs are snowflakes too; they are display metadata, not a route segment here.
pub fn valid_application_id(id: &str) -> bool {
	!id.is_empty()
		&& id.len() <= MAX_APPLICATION_ID_CHARS
		&& id.bytes().all(|byte| byte.is_ascii_digit())
}

/// The detail and delete path for one authorization when its ID is safe.
pub fn authorization_path(id: &str) -> Option<String> {
	valid_authorization_id(id).then(|| format!("/oauth2/tokens/{id}"))
}

/// Only bounded HTTPS policy links are offered as clickable. Other values stay plain text.
pub fn safe_policy_url(url: &str) -> Option<&str> {
	let url = url.trim();
	(url.len() <= MAX_POLICY_URL_CHARS
		&& url.starts_with("https://")
		&& !url
			.chars()
			.any(|character| character.is_control() || character.is_whitespace()))
	.then_some(url)
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn authorization_paths_accept_only_bounded_snowflakes() {
		assert_eq!(
			authorization_path("123456789012345678").as_deref(),
			Some("/oauth2/tokens/123456789012345678")
		);
		assert!(authorization_path("").is_none());
		assert!(authorization_path("../tokens").is_none());
		assert!(authorization_path("123/456").is_none());
		assert!(authorization_path(&"1".repeat(MAX_AUTHORIZATION_ID_CHARS + 1)).is_none());
		assert_eq!(
			safe_policy_url("https://example.com/privacy"),
			Some("https://example.com/privacy")
		);
		assert!(safe_policy_url("http://example.com/privacy").is_none());
		assert!(safe_policy_url("https://example.com/a b").is_none());
	}
}
