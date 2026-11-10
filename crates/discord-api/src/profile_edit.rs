use crate::{DiscordApi, Failure};
use discord_protocol::{UserDto, collectibles, profile};
use model::{Id, OwnProfileData, OwnedCollectibles, ProfileEdit, UserProfile};
use reqwest::Method;

impl DiscordApi {
	/// Read-only optional enrichment in the existing cancellable profile worker.
	pub(super) async fn load_profile_board_games(
		&self,
		profile: &mut UserProfile,
	) -> Result<(), Failure> {
		let Some(board) = &profile.board else {
			return Ok(());
		};
		let mut ids: Vec<Id> = board
			.iter()
			.flat_map(|widget| widget.games.iter().map(|game| game.id))
			.collect();
		ids.sort_unstable();
		ids.dedup();
		for batch in ids.chunks(25) {
			let path = board_games_path(batch);
			let bytes = match self
				.request_limited(Method::GET, &path, None, profile::MAX_PROFILE_WIRE)
				.await
			{
				Ok(bytes) => bytes,
				Err(failure) if failure.ends_session() => return Err(failure),
				// Optional metadata failures must not hide the rest of the profile.
				Err(_) => break,
			};
			let mut enriched = profile.clone();
			if profile::apply_board_games(&mut enriched, &bytes, batch).is_ok() {
				*profile = enriched;
			}
		}
		Ok(())
	}

	/// Optional owned options for this session. Losing the optional catalog never hides the
	/// ordinary profile, but an expired or challenged session remains authoritative.
	async fn load_owned_collectibles(&self) -> Result<Option<OwnedCollectibles>, Failure> {
		match self
			.request_limited(
				Method::GET,
				"/users/@me/collectibles-purchases",
				None,
				collectibles::MAX_PURCHASES_WIRE,
			)
			.await
		{
			Ok(bytes) => Ok(collectibles::decode_purchases(&bytes).ok()),
			Err(failure)
				if failure.ends_session()
					&& !matches!(failure, Failure::Capacity | Failure::CapacityAt(_)) =>
			{
				Err(failure)
			}
			Err(_) => Ok(None),
		}
	}

	/// Account fields and the profile effect have separate routes and no cross-route
	/// atomicity. They are sent in that order, once each, then every requested field is read back.
	pub(super) async fn edit_profile(
		&self,
		user: Id,
		changes: Option<ProfileEdit>,
	) -> Result<Box<OwnProfileData>, Failure> {
		if user.0 == 0 {
			return Err(Failure::Protocol);
		}
		if let Some(changes) = &changes {
			let body = profile::encode_edit(changes).map_err(|_| Failure::Protocol)?;
			if body.as_object().is_some_and(|fields| !fields.is_empty()) {
				// No write retry. Account responses may contain tokens; ignore those fields
				// and clear the owned response buffer rather than retaining a replacement token.
				let bytes = zeroize::Zeroizing::new(
					self.request_limited(
						Method::PATCH,
						"/users/@me",
						Some(body),
						profile::MAX_PROFILE_WIRE,
					)
					.await?,
				);
				let saved: UserDto = discord_protocol::decode(&bytes).map_err(|_| {
					Failure::ProtocolAt(
						"Profile save could not be confirmed; reload before retrying",
					)
				})?;
				if saved.id != user || saved.bot {
					return Err(Failure::Protocol);
				}
			}
			if let Some(body) =
				profile::encode_profile_effect(changes).map_err(|_| Failure::Protocol)?
			{
				let bytes = self
					.request_limited(
						Method::PATCH,
						"/users/@me/profile",
						Some(body),
						profile::MAX_PROFILE_WIRE,
					)
					.await?;
				let saved: serde_json::Value = discord_protocol::decode(&bytes).map_err(|_| {
					Failure::ProtocolAt(
						"Profile effect save could not be confirmed; reload before retrying",
					)
				})?;
				if !saved.is_object() {
					return Err(Failure::ProtocolAt(
						"Profile effect save could not be confirmed; reload before retrying",
					));
				}
			}
		}
		let path = format!(
			"/users/{user}/profile?with_mutual_guilds=false&with_mutual_friends=false&with_mutual_friends_count=false"
		);
		let bytes = self
			.request_limited(Method::GET, &path, None, profile::MAX_PROFILE_WIRE)
			.await?;
		let saved = profile::decode_profile(&bytes, None, false).map_err(|_| Failure::Protocol)?;
		if saved.user.id != user || saved.limited || saved.guild.is_some() {
			return Err(Failure::ProtocolAt(
				"Full account profile is unavailable; reload before editing",
			));
		}
		if let Some(changes) = changes
			&& (changes
				.global_name
				.as_ref()
				.is_some_and(|name| *name != saved.global_name)
				|| changes.bio.as_ref().is_some_and(|bio| *bio != saved.bio)
				|| changes
					.pronouns
					.as_ref()
					.is_some_and(|pronouns| *pronouns != saved.pronouns)
				|| changes
					.accent_color
					.is_some_and(|color| color != saved.accent_color)
				|| changes
					.avatar
					.as_ref()
					.is_some_and(|avatar| avatar.is_some() != saved.user.avatar.is_some())
				|| changes
					.banner
					.as_ref()
					.is_some_and(|banner| banner.is_some() != saved.banner.is_some())
				|| changes.avatar_decoration_sku_id.is_some_and(|sku| {
					sku != saved
						.user
						.cosmetics
						.as_deref()
						.and_then(|cosmetics| cosmetics.decoration.as_ref())
						.map(|decoration| decoration.sku)
				}) || changes.nameplate_sku_id.is_some_and(|sku| {
				sku != saved
					.user
					.cosmetics
					.as_deref()
					.and_then(|cosmetics| cosmetics.nameplate.as_ref())
					.map(|nameplate| nameplate.sku)
			}) || changes
				.profile_effect_id
				.is_some_and(|sku| sku != saved.effect_sku))
		{
			return Err(Failure::ProtocolAt(
				"Profile changes were not confirmed; reload before retrying",
			));
		}
		let collectibles = self.load_owned_collectibles().await?;
		Ok(Box::new(OwnProfileData {
			profile: saved,
			collectibles,
		}))
	}
}

// The client serializes array query parameters as repeated keys, not CSV.
fn board_games_path(ids: &[Id]) -> String {
	let query = ids
		.iter()
		.map(|id| format!("game_ids={id}"))
		.collect::<Vec<_>>()
		.join("&");
	format!("/games?{query}")
}

#[cfg(debug_assertions)]
pub fn debug_profile_board_request_check() {
	assert_eq!(
		board_games_path(&[Id(2), Id(3)]),
		"/games?game_ids=2&game_ids=3"
	);
	assert_eq!(board_games_path(&[Id(2)]), "/games?game_ids=2");
}

#[cfg(test)]
mod tests {
	use super::*;
	use client_core::{Command, Event, auth::SessionSecret};
	use serde_json::{Value, json};
	use std::{sync::Arc, time::Duration};
	use tokio::{
		io::{AsyncReadExt, AsyncWriteExt},
		net::TcpListener,
	};

	const GET: &str = "GET /users/1/profile?with_mutual_guilds=false&with_mutual_friends=false&with_mutual_friends_count=false";
	const PATCH: &str = "PATCH /users/@me";
	const PATCH_PROFILE: &str = "PATCH /users/@me/profile";
	const PURCHASES: &str = "GET /users/@me/collectibles-purchases";
	const USER: &str =
		r#"{"id":"1","username":"synthetic","global_name":null,"avatar":null,"discriminator":"0"}"#;
	const PROFILE: &str = r#"{"user":{"id":"1","username":"synthetic","global_name":null,"avatar":null,"discriminator":"0"},"user_profile":{"bio":"hello","pronouns":"","accent_color":null},"mutual_guilds":null,"mutual_friends":null}"#;

	async fn respond(
		listener: &TcpListener,
		route: &str,
		body: Option<Value>,
		status: u16,
		response: &str,
	) {
		let (mut socket, _) = listener.accept().await.unwrap();
		let mut bytes = Vec::new();
		loop {
			let mut chunk = [0; 1024];
			let count = socket.read(&mut chunk).await.unwrap();
			assert!(count > 0);
			bytes.extend_from_slice(&chunk[..count]);
			assert!(bytes.len() <= 8192);
			if let Some(end) = bytes.windows(4).position(|v| v == b"\r\n\r\n") {
				let headers = std::str::from_utf8(&bytes[..end]).unwrap();
				let length = headers
					.lines()
					.find_map(|line| {
						line.to_ascii_lowercase()
							.strip_prefix("content-length: ")
							.and_then(|length| length.parse::<usize>().ok())
					})
					.unwrap_or_default();
				if bytes.len() < end + 4 + length {
					continue;
				}
				assert!(headers.starts_with(&format!("{route} HTTP/1.1\r\n")));
				assert_eq!(
					if length == 0 {
						None
					} else {
						Some(
							serde_json::from_slice::<Value>(&bytes[end + 4..end + 4 + length])
								.unwrap(),
						)
					},
					body
				);
				break;
			}
		}
		socket.write_all(format!("HTTP/1.1 {status} Test\r\nContent-Length: {}\r\nRetry-After: 0.001\r\nConnection: close\r\n\r\n{response}", response.len()).as_bytes()).await.unwrap();
	}

	#[tokio::test]
	async fn profile_edits_send_only_changes_confirm_readback_and_do_not_retry() {
		tokio::time::timeout(Duration::from_secs(10), async {
			let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
			let mut api = DiscordApi::new(Arc::new(
				SessionSecret::from_owner_input("SYNTHETIC_PROFILE_EDIT_TOKEN".into()).unwrap(),
			))
			.unwrap();
			api.base = format!("http://{}", listener.local_addr().unwrap());
			let edit = ProfileEdit {
				global_name: Some(None),
				bio: Some("hello".into()),
				pronouns: Some(String::new()),
				accent_color: Some(None),
				avatar: None,
				banner: Some(None),
				..Default::default()
			};
			let server = async {
				respond(
					&listener,
					PATCH,
					Some(
						json!({"global_name":null,"bio":"hello","pronouns":"","accent_color":null,"banner":null}),
					),
					200,
					USER,
				)
				.await;
				respond(&listener, GET, None, 200, PROFILE).await;
				respond(&listener, PURCHASES, None, 200, "[]").await;
			};
			let (event, ()) = tokio::join!(
				api.execute(Command::EditProfile {
					user: Id(1),
					request: 42,
					changes: Some(edit)
				}),
				server
			);
			let Event::ProfileEdited {
				user,
				request,
				result,
			} = event
			else {
				panic!("wrong event")
			};
			assert_eq!((user, request), (Id(1), 42));
			assert_eq!(result.unwrap().profile.bio, "hello");
			let set = ProfileEdit {
				global_name: Some(Some("New name".into())),
				bio: Some(String::new()),
				pronouns: Some("they/them".into()),
				accent_color: Some(Some(0x123456)),
				avatar: None,
				banner: None,
				..Default::default()
			};
			let server = async {
				respond(&listener, PATCH, Some(json!({"global_name":"New name","bio":"","pronouns":"they/them","accent_color":0x123456})), 200, USER).await;
				let saved = json!({"user":{"id":"1","username":"synthetic","global_name":"New name"},"user_profile":{"bio":"","pronouns":"they/them","accent_color":0x123456}}).to_string();
				respond(&listener, GET, None, 200, &saved).await;
				respond(&listener, PURCHASES, None, 200, "[]").await;
			};
			let (result, ()) = tokio::join!(api.edit_profile(Id(1), Some(set)), server);
			assert_eq!(result.unwrap().profile.user.name, "New name");

			let only_bio = ProfileEdit {
				bio: Some("hello".into()),
				..Default::default()
			};
			for changes in [None, Some(ProfileEdit::default()), Some(only_bio.clone())] {
				let server = async {
					if changes.as_ref().is_some_and(|edit| edit.bio.is_some()) {
						respond(&listener, PATCH, Some(json!({"bio":"hello"})), 200, USER).await;
					}
					respond(&listener, GET, None, 200, PROFILE).await;
					respond(&listener, PURCHASES, None, 200, "[]").await;
				};
				let (result, ()) = tokio::join!(api.edit_profile(Id(1), changes.clone()), server);
				assert_eq!(result.unwrap().profile.bio, "hello");
			}

			for (status, body, expected) in [
				(401, "{}", Failure::Expired),
				(403, "{}", Failure::Forbidden),
				(429, r#"{"retry_after":0.001}"#, Failure::RateLimited),
				(500, "{}", Failure::Ambiguous),
				(200, r#"{"id":"2","username":"other"}"#, Failure::Protocol),
			] {
				let mut api = DiscordApi::new(Arc::new(
					SessionSecret::from_owner_input("SYNTHETIC_PROFILE_EDIT_TOKEN".into()).unwrap(),
				))
				.unwrap();
				api.base = format!("http://{}", listener.local_addr().unwrap());
				let (result, ()) = tokio::join!(
					api.edit_profile(Id(1), Some(only_bio.clone())),
					respond(&listener, PATCH, Some(json!({"bio":"hello"})), status, body)
				);
				assert_eq!(result.err(), Some(expected));
			}
			let server = async {
				respond(&listener, PATCH, Some(json!({"bio":"hello"})), 200, USER).await;
				respond(
					&listener,
					GET,
					None,
					200,
					&PROFILE.replace("hello", "old value"),
				)
				.await;
			};
			let (result, ()) = tokio::join!(api.edit_profile(Id(1), Some(only_bio)), server);
			assert!(matches!(result, Err(Failure::ProtocolAt(_))));
			for body in [format!(r#"{{"user":{USER}}}"#), PROFILE.replace(r#""id":"1""#, r#""id":"2""#)] {
				let (result, ()) = tokio::join!(api.edit_profile(Id(1), None), respond(&listener, GET, None, 200, &body));
				assert!(matches!(result, Err(Failure::ProtocolAt(_))));
			}

			let too_long = ProfileEdit {
				bio: Some("x".repeat(191)),
				..Default::default()
			};
			assert_eq!(
				api.edit_profile(Id(1), Some(too_long)).await.err(),
				Some(Failure::Protocol)
			);
			assert_eq!(
				api.edit_profile(Id(0), None).await.err(),
				Some(Failure::Protocol)
			);
			assert!(
				tokio::time::timeout(Duration::from_millis(10), listener.accept())
					.await
					.is_err(),
				"invalid edits and retry attempts must not reach the transport"
			);
		})
		.await
		.unwrap();
	}

	#[tokio::test]
	async fn owned_cosmetics_use_ordered_routes_and_exact_readback() {
		tokio::time::timeout(Duration::from_secs(10), async {
			let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
			let mut api = DiscordApi::new(Arc::new(
				SessionSecret::from_owner_input("SYNTHETIC_PROFILE_EDIT_TOKEN".into()).unwrap(),
			))
			.unwrap();
			api.base = format!("http://{}", listener.local_addr().unwrap());
			let changes = ProfileEdit {
				avatar_decoration_sku_id: Some(Some(Id(11))),
				nameplate_sku_id: Some(Some(Id(12))),
				profile_effect_id: Some(Some(Id(14))),
				..Default::default()
			};
			let saved = json!({
				"user":{"id":"1","username":"synthetic",
					"avatar_decoration_data":{"asset":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","sku_id":"11","expires_at":null},
					"collectibles":{"nameplate":{"asset":"nameplates/synthetic/","sku_id":"12","label":"Plate","palette":"violet","expires_at":null}}},
				"user_profile":{"collectibles":[{"sku_id":"14","type":1}]}
			});
			let purchases = json!([
				{"sku_id":"11","name":"Ring","purchase_type":1,"items":[
					{"type":0,"sku_id":"11","asset":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}]},
				{"sku_id":"12","name":"Plate","purchase_type":6,"items":[
					{"type":2,"sku_id":"12","asset":"nameplates/synthetic/","label":"Plate","palette":"violet"}]},
				{"sku_id":"14","name":"Sparks","purchase_type":1,"items":[
					{"type":1,"sku_id":"14","title":"Sparks","reducedMotionSrc":format!("https://cdn.discordapp.com/assets/content/{}","a".repeat(64)),"effects":[]}]}
			]);
			let server = async {
				respond(
					&listener,
					PATCH,
					Some(json!({"avatar_decoration_sku_id":"11","nameplate_sku_id":"12"})),
					200,
					USER,
				)
				.await;
				respond(
					&listener,
					PATCH_PROFILE,
					Some(json!({"profile_effect_id":"14"})),
					200,
					"{}",
				)
				.await;
				respond(&listener, GET, None, 200, &saved.to_string()).await;
				respond(&listener, PURCHASES, None, 200, &purchases.to_string()).await;
			};
			let (result, ()) = tokio::join!(api.edit_profile(Id(1), Some(changes)), server);
			let result = result.unwrap();
			assert_eq!(result.profile.effect_sku, Some(Id(14)));
			assert_eq!(result.collectibles.unwrap().effects.len(), 1);

			// An effect-only clear never emits an empty account PATCH. An unavailable optional
			// catalog disables new SKU choices without turning the profile read into a failure.
			let server = async {
				respond(
					&listener,
					PATCH_PROFILE,
					Some(json!({"profile_effect_id":null})),
					200,
					"{}",
				)
				.await;
				respond(&listener, GET, None, 200, PROFILE).await;
				respond(&listener, PURCHASES, None, 403, "{}").await;
			};
			let (result, ()) = tokio::join!(
				api.edit_profile(
					Id(1),
					Some(ProfileEdit {
						profile_effect_id: Some(None),
						..Default::default()
					})
				),
				server
			);
			assert!(result.unwrap().collectibles.is_none());

			// A profile effect ignored by the service must not be reported as saved, and the
			// inventory read/deauthorization still follows the successful profile read.
			let server = async {
				respond(
					&listener,
					PATCH_PROFILE,
					Some(json!({"profile_effect_id":"14"})),
					200,
					"{}",
				)
				.await;
				respond(&listener, GET, None, 200, PROFILE).await;
			};
			let (result, ()) = tokio::join!(
				api.edit_profile(
					Id(1),
					Some(ProfileEdit {
						profile_effect_id: Some(Some(Id(14))),
						..Default::default()
					})
				),
				server
			);
			assert!(matches!(result, Err(Failure::ProtocolAt(_))));
			let (result, ()) = tokio::join!(api.edit_profile(Id(1), None), async {
				respond(&listener, GET, None, 200, PROFILE).await;
				respond(&listener, PURCHASES, None, 401, "{}").await;
			});
			assert_eq!(result.err(), Some(Failure::Expired));
		})
		.await
		.unwrap();
	}
}
