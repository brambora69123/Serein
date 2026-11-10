//! Current-account OAuth2 authorization list, detail and explicit deauthorization. The routes
//! are documented by Userdoccers, not Discord's public documentation, and normal-session
//! delivery remains unofficial/live-unverified:
//! <https://docs.discord.food/topics/oauth2>
//!
//! No access or refresh token is read, stored or sent. Deauthorization is one DELETE attempt
//! followed by one bounded list refetch; an ambiguous write is never retried here.
use crate::{DiscordApi, Failure};
use discord_protocol::authorized_apps::{
	MAX_AUTHORIZED_APP_WIRE, decode_authorized_app, decode_authorized_apps,
};
use model::authorized_apps::{AuthorizedAppGrant, AuthorizedAppsPage, authorization_path};
use reqwest::Method;

impl DiscordApi {
	pub(crate) async fn authorized_apps(&self) -> Result<AuthorizedAppsPage, Failure> {
		let bytes = self
			.request_limited(Method::GET, "/oauth2/tokens", None, MAX_AUTHORIZED_APP_WIRE)
			.await?;
		decode_authorized_apps(&bytes).map_err(|_| Failure::Protocol)
	}

	pub(crate) async fn authorized_app(&self, id: &str) -> Result<AuthorizedAppGrant, Failure> {
		let path = authorization_path(id).ok_or(Failure::Protocol)?;
		let bytes = self
			.request_limited(Method::GET, &path, None, MAX_AUTHORIZED_APP_WIRE)
			.await?;
		let grant = decode_authorized_app(&bytes).map_err(|_| Failure::Protocol)?;
		if grant.id != id {
			return Err(Failure::Protocol);
		}
		Ok(grant)
	}

	/// Exactly one DELETE attempt. A 204 is followed by one bounded list refetch; success is
	/// reported only when the refreshed list no longer contains the requested authorization.
	/// A failed or unconfirmed refetch is ambiguous, never a success and never retried.
	pub(crate) async fn deauthorize_authorized_app(
		&self,
		id: &str,
	) -> Result<AuthorizedAppsPage, Failure> {
		let path = authorization_path(id).ok_or(Failure::Protocol)?;
		let bytes = self
			.request_limited(Method::DELETE, &path, None, MAX_AUTHORIZED_APP_WIRE)
			.await?;
		if !bytes.is_empty() {
			return Err(Failure::Protocol);
		}
		let page = match self.authorized_apps().await {
			Ok(page) => page,
			Err(failure) if failure.ends_session() => return Err(failure),
			Err(_) => return Err(Failure::Ambiguous),
		};
		if page.items.iter().any(|grant| grant.id == id) {
			return Err(Failure::ProtocolAt(
				"Deauthorization was not confirmed; the application is still listed",
			));
		}
		Ok(page)
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use client_core::{Command, Event, auth::SessionSecret};
	use std::{sync::Arc, time::Duration};
	use tokio::{
		io::{AsyncReadExt, AsyncWriteExt},
		net::TcpListener,
	};

	async fn api(listener: &TcpListener) -> DiscordApi {
		let mut api = DiscordApi::new(Arc::new(
			SessionSecret::from_owner_input("SYNTHETIC_AUTHORIZED_APPS_TOKEN".into()).unwrap(),
		))
		.unwrap();
		api.base = format!("http://{}", listener.local_addr().unwrap());
		api
	}

	async fn serve(listener: &TcpListener, expected: &str, status: &str, body: &str) -> String {
		let (mut socket, _) = listener.accept().await.unwrap();
		let mut request = Vec::new();
		loop {
			let mut buffer = [0; 1024];
			let n = socket.read(&mut buffer).await.unwrap();
			assert!(n > 0);
			request.extend_from_slice(&buffer[..n]);
			assert!(request.len() < 8192);
			if request.windows(4).any(|window| window == b"\r\n\r\n") {
				break;
			}
		}
		let text = std::str::from_utf8(&request).unwrap();
		assert!(text.starts_with(expected), "{text}");
		assert!(text.contains("SYNTHETIC_AUTHORIZED_APPS_TOKEN"));
		socket
			.write_all(
				format!(
					"HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
					body.len()
				)
				.as_bytes(),
			)
			.await
			.unwrap();
		text.into()
	}

	const GRANT: &str = r#"{"id":"100","application":{"id":"200","name":"Synthetic","description":"Demo","is_verified":"yes","terms_of_service_url":"https://example.com/terms","privacy_policy_url":"https://example.com/privacy"},"scopes":["identify","connections"],"created_at":"2026-01-02T03:04:05Z"}"#;

	#[tokio::test]
	async fn list_preserves_unknown_state_and_bounds() {
		tokio::time::timeout(Duration::from_secs(10), async {
			let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
			let api = api(&listener).await;
			let server = {
				let listener = listener;
				tokio::spawn(async move {
					serve(
						&listener,
						"GET /oauth2/tokens HTTP/1.1\r\n",
						"200 OK",
						&format!("[{GRANT}]"),
					)
					.await;
				})
			};
			let Event::AuthorizedApps {
				request: 4,
				result: Ok(page),
			} = api.execute(Command::AuthorizedApps { request: 4 }).await
			else {
				panic!("authorized apps list")
			};
			let grant = &page.items[0];
			assert_eq!(grant.id, "100");
			assert_eq!(grant.application.name, "Synthetic");
			assert_eq!(grant.application.is_verified, None);
			assert_eq!(
				grant.scopes.as_deref(),
				Some(&["identify".to_owned(), "connections".to_owned()][..])
			);
			assert_eq!(grant.authorized_at.as_deref(), Some("2026-01-02T03:04:05Z"));
			assert!(grant.can_deauthorize());
			server.await.unwrap();
		})
		.await
		.unwrap();
	}

	#[tokio::test]
	async fn detail_is_one_bounded_read_and_must_match_the_requested_id() {
		tokio::time::timeout(Duration::from_secs(10), async {
			let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
			let api = api(&listener).await;
			let server = {
				let listener = listener;
				tokio::spawn(async move {
					serve(
						&listener,
						"GET /oauth2/tokens/100 HTTP/1.1\r\n",
						"200 OK",
						GRANT,
					)
					.await;
				})
			};
			let Event::AuthorizedAppDetail {
				request: 5,
				result: Ok(grant),
			} = api.execute(Command::AuthorizedAppDetail {
				id: "100".into(),
				request: 5,
			})
			.await
			else {
				panic!("authorized app detail")
			};
			assert_eq!(grant.id, "100");
			assert_eq!(grant.application.name, "Synthetic");
			server.await.unwrap();
		})
		.await
		.unwrap();
	}

	#[tokio::test]
	async fn deauthorization_is_one_delete_then_one_bounded_refetch() {
		tokio::time::timeout(Duration::from_secs(10), async {
			let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
			let api = api(&listener).await;
			let server = {
				let listener = listener;
				tokio::spawn(async move {
					let delete = serve(
						&listener,
						"DELETE /oauth2/tokens/100 HTTP/1.1\r\n",
						"204 No Content",
						"",
					)
					.await;
					assert!(delete.contains("SYNTHETIC_AUTHORIZED_APPS_TOKEN"));
					let get = serve(
						&listener,
						"GET /oauth2/tokens HTTP/1.1\r\n",
						"200 OK",
						r#"[{"id":"101","application":{"id":"201","name":"Other"},"scopes":["identify"]}]"#,
					)
					.await;
					assert!(get.contains("SYNTHETIC_AUTHORIZED_APPS_TOKEN"));
				})
			};
			let Event::AuthorizedAppDeauthorized {
				request: 6,
				result: Ok(page),
			} = api.execute(Command::DeauthorizeAuthorizedApp {
				id: "100".into(),
				request: 6,
			})
			.await
			else {
				panic!("deauthorization")
			};
			assert!(page.items.iter().all(|grant| grant.id != "100"));
			server.await.unwrap();
		})
		.await
		.unwrap();
	}

	#[tokio::test]
	async fn deauthorization_is_not_success_when_the_refetch_still_lists_the_app() {
		tokio::time::timeout(Duration::from_secs(10), async {
			let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
			let api = api(&listener).await;
			let server = {
				let listener = listener;
				tokio::spawn(async move {
					serve(
						&listener,
						"DELETE /oauth2/tokens/100 HTTP/1.1\r\n",
						"204 No Content",
						"",
					)
					.await;
					serve(
						&listener,
						"GET /oauth2/tokens HTTP/1.1\r\n",
						"200 OK",
						&format!("[{GRANT}]"),
					)
					.await;
				})
			};
			let Event::AuthorizedAppDeauthorized {
				request: 7,
				result: Err(Failure::ProtocolAt(_)),
			} = api.execute(Command::DeauthorizeAuthorizedApp {
				id: "100".into(),
				request: 7,
			})
			.await
			else {
				panic!("unconfirmed deauthorization")
			};
			server.await.unwrap();
		})
		.await
		.unwrap();
	}

	#[tokio::test]
	async fn ambiguous_delete_is_not_retried_and_invalid_ids_never_reach_the_network() {
		tokio::time::timeout(Duration::from_secs(10), async {
			let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
			let api = api(&listener).await;
			let server = {
				let listener = listener;
				tokio::spawn(async move {
					serve(
						&listener,
						"DELETE /oauth2/tokens/100 HTTP/1.1\r\n",
						"500 Internal Server Error",
						"",
					)
					.await;
				})
			};
			let Event::AuthorizedAppDeauthorized {
				request: 8,
				result: Err(Failure::Ambiguous),
			} = api.execute(Command::DeauthorizeAuthorizedApp {
				id: "100".into(),
				request: 8,
			})
			.await
			else {
				panic!("ambiguous delete")
			};
			server.await.unwrap();
			assert!(api.deauthorize_authorized_app("../other").await.is_err());
		})
		.await
		.unwrap();
	}
}
