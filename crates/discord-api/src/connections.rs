//! Current-account connection list and visibility write. The list route is documented for
//! OAuth2 applications; normal-session delivery and the PATCH route are documented by
//! Userdoccers, not Discord's public documentation, and remain live-unverified.
//!
//! This module deliberately shares no client, token or state with `spotify::Playback`; opening
//! Settings > Connections performs only the one bounded list read described here.
use crate::{DiscordApi, Failure};
use discord_protocol::connections::{MAX_CONNECTION_WIRE, decode_connection, decode_connections};
use model::connections::{Connection, ConnectionVisibility, ConnectionsPage, connection_path};
use reqwest::Method;

impl DiscordApi {
	pub(crate) async fn connections(&self) -> Result<ConnectionsPage, Failure> {
		let bytes = self
			.request_limited(
				Method::GET,
				"/users/@me/connections",
				None,
				MAX_CONNECTION_WIRE,
			)
			.await?;
		decode_connections(&bytes).map_err(|_| Failure::Protocol)
	}

	/// Exactly one PATCH attempt. The decoded response must name the requested row and report
	/// the requested visibility; no retry or follow-up read is issued.
	pub(crate) async fn set_connection_visibility(
		&self,
		kind: &str,
		id: &str,
		visible: bool,
	) -> Result<Connection, Failure> {
		let path = connection_path(kind, id).ok_or(Failure::Protocol)?;
		let body = serde_json::json!({ "visibility": if visible { 1 } else { 0 } });
		let bytes = self
			.request_limited(Method::PATCH, &path, Some(body), MAX_CONNECTION_WIRE)
			.await?;
		let connection = decode_connection(&bytes).map_err(|_| Failure::Protocol)?;
		let expected = ConnectionVisibility::from_raw(Some(if visible { 1 } else { 0 }));
		if connection.kind != kind || connection.id != id || connection.visibility != expected {
			return Err(Failure::Protocol);
		}
		Ok(connection)
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
			SessionSecret::from_owner_input("SYNTHETIC_CONNECTIONS_TOKEN".into()).unwrap(),
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
		assert!(text.contains("SYNTHETIC_CONNECTIONS_TOKEN"));
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

	#[tokio::test]
	async fn list_preserves_unknown_state_and_never_reads_spotify() {
		tokio::time::timeout(Duration::from_secs(10), async {
			let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
			let api = api(&listener).await;
			let server = {
				let listener = listener;
				tokio::spawn(async move {
					serve(
						&listener,
						"GET /users/@me/connections HTTP/1.1\r\n",
						"200 OK",
						r#"[{"type":"reddit","id":"run&hide","name":"synthetic","verified":true,"visibility":7,"show_activity":true,"metadata_visibility":0,"metadata":{"total_karma":"20223"}}]"#,
					)
					.await;
				})
			};
			let Event::Connections {
				request: 4,
				result: Ok(page),
			} = api.execute(Command::Connections { request: 4 }).await
			else {
				panic!("connection list")
			};
			let connection = &page.items[0];
			assert_eq!(connection.id, "run&hide");
			assert_eq!(connection.verified, Some(true));
			assert_eq!(connection.revoked, None);
			assert_eq!(connection.visibility, ConnectionVisibility::Unknown);
			assert_eq!(connection.metadata_visibility, ConnectionVisibility::OnlyYou);
			assert_eq!(connection.show_activity, Some(true));
			assert_eq!(connection.friend_sync, None);
			assert_eq!(connection.two_way_link, None);
			assert_eq!(connection.metadata[0].key, "total_karma");
			server.await.unwrap();
		})
		.await
		.unwrap();
	}

	#[tokio::test]
	async fn visibility_patch_is_one_encoded_attempt_and_requires_matching_readback() {
		tokio::time::timeout(Duration::from_secs(10), async {
			let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
			let api = api(&listener).await;
			let server = {
				let listener = listener;
				tokio::spawn(async move {
					let request = serve(
						&listener,
						"PATCH /users/@me/connections/reddit/run%26hide HTTP/1.1\r\n",
						"200 OK",
						r#"{"type":"reddit","id":"run&hide","name":"synthetic","verified":true,"visibility":0,"show_activity":true,"metadata_visibility":0}"#,
					)
					.await;
					assert!(request.contains(r#""visibility":0"#), "{request}");
				})
			};
			let Event::ConnectionVisibility {
				request: 9,
				result: Ok(connection),
			} = api
				.execute(Command::SetConnectionVisibility {
					kind: "reddit".into(),
					id: "run&hide".into(),
					visible: false,
					request: 9,
				})
				.await
			else {
				panic!("visibility readback")
			};
			assert_eq!(connection.visibility, ConnectionVisibility::OnlyYou);
			assert!(
				api.set_connection_visibility("../other", "id", false)
					.await
					.is_err()
			);
			server.await.unwrap();
		})
		.await
		.unwrap();
	}
}
