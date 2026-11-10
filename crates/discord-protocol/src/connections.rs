//! Bounded `GET /users/@me/connections` and `PATCH /users/@me/connections/{type}/{id}` decoding.
//!
//! The list route is documented for OAuth2 applications and delivered here through a normal
//! user session; the current-user response shape and the PATCH route are documented by
//! Userdoccers, not Discord's public documentation:
//! <https://github.com/discord-userdoccers/discord-userdoccers/blob/master/pages/resources/connected-accounts.mdx>
//!
//! Unknown values are kept unknown. Decoding never fills a missing flag with `false` or maps
//! an unrecognized visibility integer onto a known setting.
use crate::DecodeError;
use model::connections::{
	Connection, ConnectionMetadata, ConnectionVisibility, ConnectionsPage, MAX_CONNECTION_ID_CHARS,
	MAX_CONNECTION_KIND_CHARS, MAX_CONNECTION_METADATA_BYTES, MAX_CONNECTION_METADATA_ENTRIES,
	MAX_CONNECTION_METADATA_KEY_CHARS, MAX_CONNECTION_METADATA_VALUE_CHARS,
	MAX_CONNECTION_NAME_CHARS, MAX_CONNECTIONS, MAX_CONNECTIONS_WIRE,
};
use serde::Deserialize;
use serde_json::Value;

pub use model::connections::MAX_CONNECTIONS_WIRE as MAX_CONNECTION_WIRE;

#[derive(Deserialize)]
struct ConnectionDto {
	id: String,
	#[serde(rename = "type")]
	kind: String,
	name: String,
	#[serde(default)]
	verified: Option<Value>,
	#[serde(default)]
	revoked: Option<Value>,
	#[serde(default)]
	visibility: Option<Value>,
	#[serde(default)]
	metadata_visibility: Option<Value>,
	#[serde(default)]
	show_activity: Option<Value>,
	#[serde(default)]
	friend_sync: Option<Value>,
	#[serde(default)]
	two_way_link: Option<Value>,
	#[serde(default)]
	metadata: Option<Value>,
}
impl ConnectionDto {
	fn into_model(self) -> Connection {
		let mut limited = false;
		let kind = text(self.kind, MAX_CONNECTION_KIND_CHARS, &mut limited);
		let id = text(self.id, MAX_CONNECTION_ID_CHARS, &mut limited);
		let name = text(self.name, MAX_CONNECTION_NAME_CHARS, &mut limited);
		let (metadata, metadata_limited) = metadata(self.metadata);
		limited |= metadata_limited;
		Connection {
			kind,
			id,
			name,
			verified: flag(self.verified.as_ref()),
			revoked: flag(self.revoked.as_ref()),
			visibility: visibility(self.visibility.as_ref()),
			metadata_visibility: visibility(self.metadata_visibility.as_ref()),
			show_activity: flag(self.show_activity.as_ref()),
			friend_sync: flag(self.friend_sync.as_ref()),
			two_way_link: flag(self.two_way_link.as_ref()),
			metadata,
			limited,
		}
	}
}

fn text(value: String, chars: usize, limited: &mut bool) -> String {
	let end = value
		.char_indices()
		.nth(chars)
		.map_or(value.len(), |(index, _)| index);
	*limited |= end < value.len();
	value[..end].to_owned()
}

fn flag(value: Option<&Value>) -> Option<bool> {
	value.and_then(Value::as_bool)
}

fn visibility(value: Option<&Value>) -> ConnectionVisibility {
	ConnectionVisibility::from_raw(value.and_then(Value::as_i64))
}

/// Scalar service metadata only. Nested objects and arrays are skipped and mark the row
/// limited rather than being flattened into invented key/value pairs.
fn metadata(value: Option<Value>) -> (Vec<ConnectionMetadata>, bool) {
	let Some(value) = value else {
		return (Vec::new(), false);
	};
	let Value::Object(fields) = value else {
		return (Vec::new(), true);
	};
	let mut entries = Vec::new();
	let mut bytes = 0;
	let mut limited = fields.len() > MAX_CONNECTION_METADATA_ENTRIES;
	for (key, value) in fields {
		if entries.len() >= MAX_CONNECTION_METADATA_ENTRIES {
			limited = true;
			break;
		}
		let mut entry_limited = false;
		let key = text(key, MAX_CONNECTION_METADATA_KEY_CHARS, &mut entry_limited);
		let value = match value {
			Value::String(value) => value,
			Value::Number(value) => value.to_string(),
			Value::Bool(value) => value.to_string(),
			_ => {
				limited = true;
				continue;
			}
		};
		let value = text(
			value,
			MAX_CONNECTION_METADATA_VALUE_CHARS,
			&mut entry_limited,
		);
		let size = key.capacity() + value.capacity();
		if bytes + size > MAX_CONNECTION_METADATA_BYTES {
			limited = true;
			break;
		}
		bytes += size;
		limited |= entry_limited;
		entries.push(ConnectionMetadata { key, value });
	}
	(entries, limited)
}

pub fn decode_connections(bytes: &[u8]) -> Result<ConnectionsPage, DecodeError> {
	if bytes.len() > MAX_CONNECTIONS_WIRE {
		return Err(DecodeError);
	}
	let rows: Vec<ConnectionDto> = serde_json::from_slice(bytes).map_err(|_| DecodeError)?;
	if rows.len() > MAX_CONNECTIONS {
		return Err(DecodeError);
	}
	let mut limited = false;
	let items: Vec<_> = rows
		.into_iter()
		.map(|row| {
			let connection = row.into_model();
			limited |= connection.limited;
			connection
		})
		.collect();
	let page = ConnectionsPage { items, limited };
	page.valid().then_some(page).ok_or(DecodeError)
}

/// A PATCH response is one complete connection. Its fields are re-checked by the caller
/// against the requested row before the visible setting changes.
pub fn decode_connection(bytes: &[u8]) -> Result<Connection, DecodeError> {
	if bytes.len() > MAX_CONNECTIONS_WIRE {
		return Err(DecodeError);
	}
	let row: ConnectionDto = serde_json::from_slice(bytes).map_err(|_| DecodeError)?;
	let connection = row.into_model();
	connection.valid().then_some(connection).ok_or(DecodeError)
}

#[cfg(test)]
mod tests {
	use super::*;
	use model::connections::{ConnectionVisibility, connection_path};

	#[test]
	fn unknown_flags_and_visibility_stay_unknown() {
		let page = decode_connections(
			br#"[
				{"type":"reddit","id":"run&hide","name":"synthetic","verified":true,"visibility":9,"metadata_visibility":0,
				 "show_activity":true,"metadata":{"karma":"20223","nested":{"x":1}}}
			]"#,
		)
		.unwrap();
		let connection = &page.items[0];
		assert_eq!(connection.verified, Some(true));
		assert_eq!(connection.revoked, None);
		assert_eq!(connection.friend_sync, None);
		assert_eq!(connection.show_activity, Some(true));
		assert_eq!(connection.visibility, ConnectionVisibility::Unknown);
		assert_eq!(
			connection.metadata_visibility,
			ConnectionVisibility::OnlyYou
		);
		assert_eq!(connection.metadata[0].key, "karma");
		assert!(connection.limited, "nested metadata is not flattened");
		assert_eq!(
			connection_path("reddit", "run&hide").as_deref(),
			Some("/users/@me/connections/reddit/run%26hide")
		);
		assert!(connection_path("reddit", "..").is_some());
		assert!(connection_path("REDDIT", "id").is_none());
		assert!(connection_path("reddit", "").is_none());
	}

	#[test]
	fn lists_are_capped_and_truncated_rows_lose_actions() {
		let rows: Vec<_> = (0..=MAX_CONNECTIONS)
			.map(|index| {
				format!(
					r#"{{"type":"github","id":"id{index}","name":"synthetic","verified":true}}"#
				)
			})
			.collect();
		assert!(
			decode_connections(format!("[{}]", rows.join(",")).as_bytes()).is_err(),
			"one row over the cap is rejected, never silently dropped"
		);
		let long = "x".repeat(MAX_CONNECTION_ID_CHARS + 1);
		let page = decode_connections(
			format!(
				r#"[{{"type":"github","id":"{long}","name":"synthetic","verified":true,"visibility":1}}]"#
			)
			.as_bytes(),
		)
		.unwrap();
		assert!(page.items[0].limited);
		assert!(!page.items[0].can_change_visibility());
		assert!(page.limited);
	}
}
