//! Bounded `GET /oauth2/tokens` and `GET /oauth2/tokens/{id}` decoding.
//!
//! The routes are documented by Userdoccers, not Discord's public documentation, and are
//! delivered here through a normal user session:
//! <https://docs.discord.food/topics/oauth2>
//!
//! Unknown values stay unknown. Decoding never fills a missing scope list, approval timestamp,
//! verification flag or policy link with an invented value.
use crate::DecodeError;
use model::authorized_apps::{
	AuthorizedAppGrant, AuthorizedApplication, AuthorizedAppsPage,
	MAX_APPLICATION_DESCRIPTION_CHARS, MAX_APPLICATION_ICON_CHARS, MAX_APPLICATION_ID_CHARS,
	MAX_APPLICATION_NAME_CHARS, MAX_AUTHORIZATION_ID_CHARS, MAX_AUTHORIZED_APPS,
	MAX_AUTHORIZED_APPS_WIRE, MAX_AUTHORIZED_AT_CHARS, MAX_POLICY_URL_CHARS, MAX_SCOPE_CHARS,
	MAX_SCOPES, MAX_SCOPES_BYTES,
};
use serde::Deserialize;
use serde_json::Value;

pub use model::authorized_apps::MAX_AUTHORIZED_APPS_WIRE as MAX_AUTHORIZED_APP_WIRE;

#[derive(Deserialize)]
struct GrantDto {
	id: String,
	application: ApplicationDto,
	#[serde(default)]
	scopes: Option<Value>,
	#[serde(default)]
	created_at: Option<Value>,
	#[serde(default)]
	authorized_at: Option<Value>,
}
impl GrantDto {
	fn into_model(self) -> AuthorizedAppGrant {
		let Self {
			id,
			application,
			scopes: scopes_value,
			created_at,
			authorized_at,
		} = self;
		let mut limited = false;
		let id = text(id, MAX_AUTHORIZATION_ID_CHARS, &mut limited);
		let application = application.into_model(&mut limited);
		let (scopes, scopes_limited) = scopes(scopes_value);
		limited |= scopes_limited;
		let authorized_at = timestamp(created_at.as_ref().or(authorized_at.as_ref()), &mut limited);
		AuthorizedAppGrant {
			id,
			application,
			scopes,
			authorized_at,
			limited,
		}
	}
}

#[derive(Deserialize)]
struct ApplicationDto {
	id: String,
	name: String,
	#[serde(default)]
	description: Option<Value>,
	#[serde(default)]
	icon: Option<Value>,
	#[serde(default)]
	is_verified: Option<Value>,
	#[serde(default)]
	terms_of_service_url: Option<Value>,
	#[serde(default)]
	privacy_policy_url: Option<Value>,
}
impl ApplicationDto {
	fn into_model(self, limited: &mut bool) -> AuthorizedApplication {
		AuthorizedApplication {
			id: text(self.id, MAX_APPLICATION_ID_CHARS, limited),
			name: text(self.name, MAX_APPLICATION_NAME_CHARS, limited),
			description: text_opt(self.description, MAX_APPLICATION_DESCRIPTION_CHARS, limited),
			icon: text_opt(self.icon, MAX_APPLICATION_ICON_CHARS, limited),
			is_verified: flag(self.is_verified),
			terms_of_service_url: text_opt(
				self.terms_of_service_url,
				MAX_POLICY_URL_CHARS,
				limited,
			),
			privacy_policy_url: text_opt(self.privacy_policy_url, MAX_POLICY_URL_CHARS, limited),
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

fn text_opt(value: Option<Value>, chars: usize, limited: &mut bool) -> Option<String> {
	match value {
		None | Some(Value::Null) => None,
		Some(Value::String(value)) => Some(text(value, chars, limited)),
		Some(_) => {
			*limited = true;
			None
		}
	}
}

fn flag(value: Option<Value>) -> Option<bool> {
	value.and_then(|value| value.as_bool())
}

fn timestamp(value: Option<&Value>, limited: &mut bool) -> Option<String> {
	match value {
		None | Some(Value::Null) => None,
		Some(Value::String(value)) => Some(text(value.clone(), MAX_AUTHORIZED_AT_CHARS, limited)),
		Some(_) => {
			*limited = true;
			None
		}
	}
}

/// Scope arrays are optional. A missing field stays unknown; a present but malformed field is
/// dropped and marks the row limited rather than being flattened into invented scopes.
fn scopes(value: Option<Value>) -> (Option<Vec<String>>, bool) {
	let Some(value) = value else {
		return (None, false);
	};
	let Value::Array(values) = value else {
		return (None, true);
	};
	let mut limited = values.len() > MAX_SCOPES;
	let mut entries = Vec::new();
	let mut bytes = 0;
	for value in values {
		if entries.len() >= MAX_SCOPES {
			limited = true;
			break;
		}
		let Value::String(value) = value else {
			limited = true;
			continue;
		};
		let mut entry_limited = false;
		let value = text(value, MAX_SCOPE_CHARS, &mut entry_limited);
		if value.is_empty() {
			limited = true;
			continue;
		}
		let size = value.capacity();
		if bytes + size > MAX_SCOPES_BYTES {
			limited = true;
			break;
		}
		bytes += size;
		limited |= entry_limited;
		entries.push(value);
	}
	(Some(entries), limited)
}

pub fn decode_authorized_apps(bytes: &[u8]) -> Result<AuthorizedAppsPage, DecodeError> {
	if bytes.len() > MAX_AUTHORIZED_APPS_WIRE {
		return Err(DecodeError);
	}
	let rows: Vec<GrantDto> = serde_json::from_slice(bytes).map_err(|_| DecodeError)?;
	let mut limited = rows.len() > MAX_AUTHORIZED_APPS;
	let items: Vec<_> = rows
		.into_iter()
		.take(MAX_AUTHORIZED_APPS)
		.map(|row| {
			let grant = row.into_model();
			limited |= grant.limited;
			grant
		})
		.collect();
	let page = AuthorizedAppsPage { items, limited };
	page.valid().then_some(page).ok_or(DecodeError)
}

/// A detail response is one complete authorization. Its ID is re-checked by the caller against
/// the requested row before the visible state changes.
pub fn decode_authorized_app(bytes: &[u8]) -> Result<AuthorizedAppGrant, DecodeError> {
	if bytes.len() > MAX_AUTHORIZED_APPS_WIRE {
		return Err(DecodeError);
	}
	let row: GrantDto = serde_json::from_slice(bytes).map_err(|_| DecodeError)?;
	let grant = row.into_model();
	grant.valid().then_some(grant).ok_or(DecodeError)
}

#[cfg(test)]
mod tests {
	use super::*;
	use model::authorized_apps::{authorization_path, safe_policy_url};

	#[test]
	fn unknown_fields_stay_unknown_and_scopes_are_optional() {
		let page = decode_authorized_apps(
			br#"[
				{"id":"100","application":{"id":"200","name":"Synthetic","description":"Demo",
				 "is_verified":"yes","terms_of_service_url":"https://example.com/terms",
				 "privacy_policy_url":"https://example.com/privacy"},
				 "scopes":["identify","connections"],"created_at":"2026-01-02T03:04:05Z"}
			]"#,
		)
		.unwrap();
		let grant = &page.items[0];
		assert_eq!(grant.id, "100");
		assert_eq!(grant.application.name, "Synthetic");
		assert_eq!(grant.application.is_verified, None);
		assert_eq!(
			grant.scopes.as_deref(),
			Some(&["identify".to_owned(), "connections".to_owned()][..])
		);
		assert_eq!(grant.authorized_at.as_deref(), Some("2026-01-02T03:04:05Z"));
		assert_eq!(
			grant.application.terms_of_service_url.as_deref(),
			Some("https://example.com/terms")
		);
		assert!(grant.can_deauthorize());
		assert_eq!(
			authorization_path(&grant.id).as_deref(),
			Some("/oauth2/tokens/100")
		);
		assert_eq!(
			safe_policy_url(grant.application.privacy_policy_url.as_deref().unwrap()),
			Some("https://example.com/privacy")
		);

		let page = decode_authorized_apps(
			br#"[{"id":"101","application":{"id":"201","name":"No scopes"}}]"#,
		)
		.unwrap();
		assert_eq!(page.items[0].scopes, None);
		assert_eq!(page.items[0].authorized_at, None);
		assert!(!page.items[0].limited);
	}

	#[test]
	fn lists_are_capped_and_truncated_rows_lose_actions() {
		let rows: Vec<_> = (0..=MAX_AUTHORIZED_APPS)
			.map(|index| {
				format!(
					r#"{{"id":"{index}","application":{{"id":"{index}","name":"Synthetic"}},"scopes":["identify"]}}"#
				)
			})
			.collect();
		let page = decode_authorized_apps(format!("[{}]", rows.join(",")).as_bytes()).unwrap();
		assert_eq!(page.items.len(), MAX_AUTHORIZED_APPS);
		assert!(page.limited);
		assert!(page.items[0].can_deauthorize());
		assert_eq!(page.items.last().unwrap().id, "63");
		let long = "x".repeat(MAX_APPLICATION_NAME_CHARS + 1);
		let page = decode_authorized_apps(
			format!(
				r#"[{{"id":"100","application":{{"id":"200","name":"{long}"}},"scopes":["identify"]}}]"#
			)
			.as_bytes(),
		)
		.unwrap();
		assert!(page.items[0].limited);
		assert!(!page.items[0].can_deauthorize());
		assert!(page.limited);
	}

	#[test]
	fn malformed_scope_arrays_are_dropped_not_invented() {
		let page = decode_authorized_apps(
			br#"[{"id":"100","application":{"id":"200","name":"Synthetic"},"scopes":"identify"}]"#,
		)
		.unwrap();
		assert_eq!(page.items[0].scopes, None);
		assert!(page.items[0].limited);
		assert!(!page.items[0].can_deauthorize());
	}
}
