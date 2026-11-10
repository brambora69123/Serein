//! Bounded current-account connection metadata. This is a session-only view; nothing here is
//! persisted, and absent or unrecognized service state stays unknown.
pub const MAX_CONNECTIONS: usize = 64;
pub const MAX_CONNECTIONS_WIRE: usize = 256 * 1024;
pub const MAX_CONNECTIONS_BYTES: usize = 512 * 1024;
pub const MAX_CONNECTION_KIND_CHARS: usize = 32;
pub const MAX_CONNECTION_ID_CHARS: usize = 128;
pub const MAX_CONNECTION_NAME_CHARS: usize = 128;
pub const MAX_CONNECTION_METADATA_ENTRIES: usize = 16;
pub const MAX_CONNECTION_METADATA_KEY_CHARS: usize = 64;
pub const MAX_CONNECTION_METADATA_VALUE_CHARS: usize = 256;
pub const MAX_CONNECTION_METADATA_BYTES: usize = 8 * 1024;

/// A connection's audience. A value the client does not recognize stays unknown rather than
/// being mapped to a plausible-looking setting.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ConnectionVisibility {
	#[default]
	Unknown,
	/// `0`: visible only to the account owner.
	OnlyYou,
	/// `1`: visible to everyone.
	Everyone,
}
impl ConnectionVisibility {
	pub fn from_raw(value: Option<i64>) -> Self {
		match value {
			Some(0) => Self::OnlyYou,
			Some(1) => Self::Everyone,
			_ => Self::Unknown,
		}
	}
	pub fn raw(self) -> Option<i64> {
		match self {
			Self::Unknown => None,
			Self::OnlyYou => Some(0),
			Self::Everyone => Some(1),
		}
	}
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConnectionMetadata {
	pub key: String,
	pub value: String,
}
impl ConnectionMetadata {
	pub(crate) fn bytes(&self) -> usize {
		self.key.capacity() + self.value.capacity()
	}
}

/// One linked account from `GET /users/@me/connections`. Every optional service flag remains
/// optional so a missing or unrecognized field is displayed as unknown, never false.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Connection {
	pub kind: String,
	/// Account identifier on the connected service. Bounded; not safe to assume URL-safe.
	pub id: String,
	pub name: String,
	pub verified: Option<bool>,
	pub revoked: Option<bool>,
	pub visibility: ConnectionVisibility,
	pub metadata_visibility: ConnectionVisibility,
	pub show_activity: Option<bool>,
	pub friend_sync: Option<bool>,
	pub two_way_link: Option<bool>,
	pub metadata: Vec<ConnectionMetadata>,
	/// At least one presentation field was truncated or dropped to stay in bounds.
	pub limited: bool,
}
impl Connection {
	pub fn bytes(&self) -> usize {
		size_of::<Self>()
			+ self.kind.capacity()
			+ self.id.capacity()
			+ self.name.capacity()
			+ self.metadata.capacity() * size_of::<ConnectionMetadata>()
			+ self
				.metadata
				.iter()
				.map(ConnectionMetadata::bytes)
				.sum::<usize>()
	}
	/// Whether a PATCH response row is bounded and structurally complete.
	pub fn valid(&self) -> bool {
		!self.kind.is_empty()
			&& !self.id.is_empty()
			&& self.kind.chars().count() <= MAX_CONNECTION_KIND_CHARS
			&& self.id.chars().count() <= MAX_CONNECTION_ID_CHARS
			&& self.name.chars().count() <= MAX_CONNECTION_NAME_CHARS
			&& self.metadata.len() <= MAX_CONNECTION_METADATA_ENTRIES
			&& self.metadata.iter().all(|entry| {
				entry.key.chars().count() <= MAX_CONNECTION_METADATA_KEY_CHARS
					&& entry.value.chars().count() <= MAX_CONNECTION_METADATA_VALUE_CHARS
			}) && self
			.metadata
			.iter()
			.map(ConnectionMetadata::bytes)
			.sum::<usize>()
			<= MAX_CONNECTION_METADATA_BYTES
	}
	/// Whether the service identifier can be placed in a PATCH path and this row has enough
	/// state to show a real control.
	pub fn can_change_visibility(&self) -> bool {
		!self.limited
			&& self.revoked != Some(true)
			&& self.visibility != ConnectionVisibility::Unknown
			&& connection_path(self.kind.as_str(), self.id.as_str()).is_some()
	}
}

/// The bounded current-account list and whether any row lost a field to truncation.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ConnectionsPage {
	pub items: Vec<Connection>,
	pub limited: bool,
}
impl ConnectionsPage {
	pub fn bytes(&self) -> usize {
		size_of::<Self>()
			+ self.items.capacity() * size_of::<Connection>()
			+ self.items.iter().map(Connection::bytes).sum::<usize>()
	}
	pub fn valid(&self) -> bool {
		self.items.len() <= MAX_CONNECTIONS
			&& self.items.iter().all(Connection::valid)
			&& self.bytes() <= MAX_CONNECTIONS_BYTES
	}
}

/// Connection kinds are fixed path segments; anything outside the documented alphabet is
/// rejected rather than escaped into a different route.
pub fn valid_connection_kind(kind: &str) -> bool {
	!kind.is_empty()
		&& kind.chars().count() <= MAX_CONNECTION_KIND_CHARS
		&& kind
			.bytes()
			.all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

/// Encodes an account ID as one path segment. Unreserved bytes pass through; everything else,
/// including dots, is percent-encoded so `..`, slashes and query/control characters cannot
/// change the route.
pub fn connection_path_segment(value: &str) -> Option<String> {
	if value.is_empty() || value.chars().count() > MAX_CONNECTION_ID_CHARS {
		return None;
	}
	let mut encoded = String::with_capacity(value.len());
	for byte in value.bytes() {
		if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'~') {
			encoded.push(char::from(byte));
		} else {
			encoded.push('%');
			encoded.push_str(&format!("{byte:02X}"));
		}
	}
	Some(encoded)
}

/// The PATCH path for one connection when both segments are safe.
pub fn connection_path(kind: &str, id: &str) -> Option<String> {
	valid_connection_kind(kind).then_some(())?;
	let id = connection_path_segment(id)?;
	Some(format!("/users/@me/connections/{kind}/{id}"))
}
