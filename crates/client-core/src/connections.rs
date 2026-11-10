//! Explicitly opened, session-only connection list and visibility writes. No persistence, no
//! reuse of profile connection summaries, and no shared state with the Spotify playback poller.
use crate::{
	Command, State,
	auth::{AuthState, Failure},
};
use model::connections::{Connection, ConnectionMetadata, ConnectionVisibility, ConnectionsPage};

pub struct View {
	pub request: u64,
	pub generation: u64,
	pub loading: bool,
	pub error: Option<&'static str>,
	pub page: Option<ConnectionsPage>,
}
impl View {
	pub fn limited(&self) -> bool {
		self.page.as_ref().is_some_and(|page| page.limited)
	}
}

/// One visibility write. The visible list is not changed optimistically; a failed or
/// unconfirmed response therefore leaves the previous setting in place.
pub struct Mutation {
	pub request: u64,
	pub generation: u64,
	pub kind: String,
	pub id: String,
	pub target: ConnectionVisibility,
	pub saving: bool,
	pub error: Option<&'static str>,
	/// Increments after a confirmed readback.
	pub confirmed: u64,
}

#[derive(Default)]
pub struct Connections {
	pub view: Option<View>,
	pub request: u64,
	pub mutation: Option<Mutation>,
}
impl Connections {
	fn pending(&self) -> bool {
		self.view.as_ref().is_some_and(|view| view.loading)
			|| self
				.mutation
				.as_ref()
				.is_some_and(|mutation| mutation.saving)
	}
	pub fn mutation_saving(&self, kind: &str, id: &str) -> bool {
		self.mutation
			.as_ref()
			.is_some_and(|mutation| mutation.saving && mutation.kind == kind && mutation.id == id)
	}
	pub fn mutation_error(&self, kind: &str, id: &str) -> Option<&'static str> {
		self.mutation.as_ref().and_then(|mutation| {
			(mutation.kind == kind && mutation.id == id)
				.then_some(mutation.error)
				.flatten()
		})
	}
	pub fn mutation_confirmed(&self, kind: &str, id: &str) -> bool {
		self.mutation.as_ref().is_some_and(|mutation| {
			mutation.confirmed > 0
				&& !mutation.saving
				&& mutation.error.is_none()
				&& mutation.kind == kind
				&& mutation.id == id
		})
	}
}

impl State {
	/// Explicitly open the page. A repeated open refreshes the list, while a request already
	/// in flight (or a visibility write) is left alone.
	pub fn request_connections(&mut self) -> Option<Command> {
		if self.connections.pending() {
			return None;
		}
		self.connections.mutation = None;
		self.connections.request = self.connections.request.wrapping_add(1);
		let request = self.connections.request;
		let allowed = self.demo
			|| (self.auth == AuthState::Authenticated
				&& self.gateway_connected
				&& self.user.is_some());
		self.connections.view = Some(View {
			request,
			generation: self.generation,
			loading: allowed,
			error: (!allowed)
				.then_some("Connections are unavailable while disconnected or signed out"),
			page: None,
		});
		if self.demo {
			self.connections.view.as_mut().unwrap().page = Some(demo_page());
			self.connections.view.as_mut().unwrap().loading = false;
			return None;
		}
		allowed.then_some(Command::Connections { request })
	}

	pub(crate) fn apply_connections(
		&mut self,
		request: u64,
		result: Result<ConnectionsPage, Failure>,
	) {
		if let Err(failure) = &result
			&& failure.ends_session()
			&& *failure != Failure::Capacity
		{
			self.fail(*failure);
			return;
		}
		let Some(view) = self.connections.view.as_mut().filter(|view| {
			view.request == request && view.loading && view.generation == self.generation
		}) else {
			return;
		};
		view.loading = false;
		match result {
			Ok(page) if page.valid() => {
				view.error = None;
				view.page = Some(page);
			}
			Ok(_) | Err(Failure::Protocol | Failure::ProtocolAt(_)) => {
				view.error = Some("Connected accounts were rejected or incompatible");
			}
			Err(Failure::RateLimited) => {
				view.error = Some("Connected accounts are rate limited; wait a moment and retry");
			}
			Err(Failure::Forbidden) => {
				view.error = Some("Connected accounts are unavailable for this account");
			}
			Err(Failure::Capacity) => {
				view.error = Some("Connected accounts exceeded the safe size limit");
			}
			Err(_) => view.error = Some("Connected accounts could not be loaded; retry"),
		}
	}

	/// One visibility write at a time. The previous setting stays visible until a matching
	/// readback arrives.
	pub fn set_connection_visibility(
		&mut self,
		kind: &str,
		id: &str,
		visible: bool,
	) -> Option<Command> {
		if self.connections.pending()
			|| self.auth != AuthState::Authenticated && !self.demo
			|| !self.gateway_connected && !self.demo
		{
			return None;
		}
		let target = if visible {
			ConnectionVisibility::Everyone
		} else {
			ConnectionVisibility::OnlyYou
		};
		let connection = self
			.connections
			.view
			.as_ref()?
			.page
			.as_ref()?
			.items
			.iter()
			.find(|connection| connection.kind == kind && connection.id == id)?
			.clone();
		if !connection.can_change_visibility() || connection.visibility == target {
			return None;
		}
		model::connections::connection_path(kind, id)?;
		self.connections.request = self.connections.request.wrapping_add(1);
		let request = self.connections.request;
		self.connections.mutation = Some(Mutation {
			request,
			generation: self.generation,
			kind: kind.to_owned(),
			id: id.to_owned(),
			target,
			saving: true,
			error: None,
			confirmed: 0,
		});
		if self.demo {
			let mut connection = connection;
			connection.visibility = target;
			self.apply_connection_visibility(request, Ok(connection));
			return None;
		}
		Some(Command::SetConnectionVisibility {
			kind: kind.to_owned(),
			id: id.to_owned(),
			visible,
			request,
		})
	}

	pub(crate) fn apply_connection_visibility(
		&mut self,
		request: u64,
		result: Result<Connection, Failure>,
	) {
		if let Err(failure) = &result
			&& failure.ends_session()
			&& *failure != Failure::Capacity
		{
			self.fail(*failure);
			return;
		}
		let (kind, id, target) = {
			let Some(mutation) = self.connections.mutation.as_mut().filter(|mutation| {
				mutation.request == request
					&& mutation.saving
					&& mutation.generation == self.generation
			}) else {
				return;
			};
			mutation.saving = false;
			(mutation.kind.clone(), mutation.id.clone(), mutation.target)
		};
		let error = match result {
			Ok(connection)
				if connection.kind == kind
					&& connection.id == id
					&& connection.visibility == target =>
			{
				if let Some(row) = self
					.connections
					.view
					.as_mut()
					.and_then(|view| view.page.as_mut())
					.and_then(|page| {
						page.items
							.iter_mut()
							.find(|row| row.kind == kind && row.id == id)
					}) {
					row.visibility = connection.visibility;
					row.limited |= connection.limited;
				}
				let mutation = self.connections.mutation.as_mut().unwrap();
				mutation.confirmed = mutation.confirmed.wrapping_add(1);
				if let Some(view) = self.connections.view.as_mut() {
					view.error = None;
				}
				None
			}
			Ok(_) | Err(Failure::Protocol | Failure::ProtocolAt(_)) => {
				Some("Visibility change was not confirmed; the previous setting was kept")
			}
			Err(Failure::RateLimited) => {
				Some("Visibility change was rate limited; the previous setting was kept")
			}
			Err(Failure::Forbidden) => Some(
				"Visibility cannot be changed for this connection; the previous setting was kept",
			),
			Err(Failure::Capacity) => Some(
				"Visibility response exceeded the safe size limit; the previous setting was kept",
			),
			Err(_) => {
				Some("Visibility change could not be confirmed; the previous setting was kept")
			}
		};
		if let Some(mutation) = self.connections.mutation.as_mut() {
			mutation.error = error;
		}
	}

	/// A lost session invalidates both the in-flight list and any unconfirmed write.
	pub(crate) fn interrupt_connections(&mut self) {
		if let Some(view) = self.connections.view.as_mut()
			&& view.loading
		{
			view.loading = false;
			view.request = view.request.wrapping_add(1);
			view.error = Some("Connections loading was interrupted; reconnect and reload");
		}
		if let Some(mutation) = self.connections.mutation.as_mut()
			&& mutation.saving
		{
			mutation.saving = false;
			mutation.request = mutation.request.wrapping_add(1);
			mutation.error = Some(
				"Visibility change was interrupted; reconnect and check the connection before retrying",
			);
		}
	}
}

/// Synthetic rows for the offline preview only. Never evidence of a live connection.
fn demo_page() -> ConnectionsPage {
	ConnectionsPage {
		items: vec![
			Connection {
				kind: "spotify".into(),
				id: "synthetic-spotify".into(),
				name: "Synthetic Listener".into(),
				verified: Some(true),
				revoked: Some(false),
				visibility: ConnectionVisibility::Everyone,
				metadata_visibility: ConnectionVisibility::Everyone,
				show_activity: Some(true),
				friend_sync: Some(false),
				two_way_link: Some(true),
				metadata: vec![ConnectionMetadata {
					key: "country".into(),
					value: "Synthetic".into(),
				}],
				limited: false,
			},
			Connection {
				kind: "github".into(),
				id: "synthetic-github".into(),
				name: "synthetic-maintainer".into(),
				verified: Some(true),
				revoked: Some(false),
				visibility: ConnectionVisibility::OnlyYou,
				metadata_visibility: ConnectionVisibility::OnlyYou,
				show_activity: Some(false),
				friend_sync: Some(false),
				two_way_link: Some(false),
				metadata: vec![],
				limited: false,
			},
			Connection {
				kind: "steam".into(),
				id: "synthetic-steam".into(),
				name: "Synthetic Player".into(),
				verified: Some(false),
				revoked: None,
				visibility: ConnectionVisibility::Unknown,
				metadata_visibility: ConnectionVisibility::Unknown,
				show_activity: None,
				friend_sync: None,
				two_way_link: None,
				metadata: vec![],
				limited: true,
			},
		],
		limited: false,
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::{Envelope, Event};
	use model::connections::MAX_CONNECTIONS;

	fn state() -> State {
		State {
			auth: AuthState::Authenticated,
			gateway_connected: true,
			user: Some(model::User {
				id: model::Id(1),
				name: "Synthetic".into(),
				avatar: None,
				webhook: false,
				kind: Default::default(),
				discriminator: 0,
				primary_guild: None,
				cosmetics: None,
			}),
			..State::default()
		}
	}
	fn connection(visibility: ConnectionVisibility) -> Connection {
		Connection {
			kind: "github".into(),
			id: "octocat".into(),
			name: "Synthetic Octocat".into(),
			verified: Some(true),
			revoked: Some(false),
			visibility,
			metadata_visibility: ConnectionVisibility::Everyone,
			show_activity: Some(true),
			friend_sync: Some(true),
			two_way_link: Some(false),
			metadata: vec![],
			limited: false,
		}
	}

	#[test]
	fn explicit_open_is_single_flight_and_stale_generations_are_ignored() {
		let mut state = state();
		let Some(Command::Connections { request }) = state.request_connections() else {
			panic!("first open requests the list");
		};
		assert!(
			state.request_connections().is_none(),
			"one request in flight"
		);
		state.apply_connections(
			request.wrapping_add(1),
			Ok(ConnectionsPage {
				items: vec![connection(ConnectionVisibility::Everyone)],
				limited: false,
			}),
		);
		assert!(state.connections.view.as_ref().unwrap().loading);
		let next = state.generation.wrapping_add(1);
		state.generation = next;
		state.apply_connections(
			request,
			Ok(ConnectionsPage {
				items: vec![connection(ConnectionVisibility::Everyone)],
				limited: false,
			}),
		);
		assert!(
			state
				.connections
				.view
				.as_ref()
				.is_some_and(|view| view.loading && view.page.is_none()),
			"an old generation cannot publish state"
		);
	}
	#[test]
	fn empty_error_and_demo_states_keep_loading_and_explicit_failures_distinct() {
		let mut state = state();
		let Some(Command::Connections { request }) = state.request_connections() else {
			panic!()
		};
		state.apply_connections(
			request,
			Ok(ConnectionsPage {
				items: vec![],
				limited: false,
			}),
		);
		let view = state.connections.view.as_ref().unwrap();
		assert!(!view.loading && view.error.is_none());
		assert_eq!(view.page.as_ref().unwrap().items.len(), 0);
		let Some(Command::Connections { request }) = state.request_connections() else {
			panic!("refresh")
		};
		state.apply_connections(request, Err(Failure::Forbidden));
		assert!(state.connections.view.as_ref().unwrap().error.is_some());
		state.gateway_connected = false;
		assert!(state.request_connections().is_none(), "offline is explicit");
		assert!(state.connections.view.as_ref().unwrap().error.is_some());
		state.demo = true;
		assert!(state.request_connections().is_none());
		let view = state.connections.view.as_ref().unwrap();
		assert!(!view.loading && view.page.as_ref().unwrap().items.len() <= MAX_CONNECTIONS);
	}
	#[test]
	fn visibility_write_without_readback_keeps_the_previous_setting() {
		let mut state = state();
		let Some(Command::Connections { request }) = state.request_connections() else {
			panic!()
		};
		state.apply_connections(
			request,
			Ok(ConnectionsPage {
				items: vec![connection(ConnectionVisibility::Everyone)],
				limited: false,
			}),
		);
		let Some(Command::SetConnectionVisibility {
			visible: false,
			request,
			..
		}) = state.set_connection_visibility("github", "octocat", false)
		else {
			panic!("visibility command")
		};
		assert!(
			state
				.connections
				.view
				.as_ref()
				.unwrap()
				.page
				.as_ref()
				.unwrap()
				.items[0]
				.visibility == ConnectionVisibility::Everyone,
			"no optimistic change"
		);
		state.apply_connection_visibility(request, Err(Failure::Network));
		assert!(
			state
				.connections
				.view
				.as_ref()
				.unwrap()
				.page
				.as_ref()
				.unwrap()
				.items[0]
				.visibility == ConnectionVisibility::Everyone
		);
		assert!(
			state
				.connections
				.mutation_error("github", "octocat")
				.is_some()
		);
		let Some(Command::SetConnectionVisibility { request, .. }) =
			state.set_connection_visibility("github", "octocat", false)
		else {
			panic!("retry after a confirmed rollback")
		};
		state.apply_connection_visibility(request, Ok(connection(ConnectionVisibility::OnlyYou)));
		assert!(
			state
				.connections
				.view
				.as_ref()
				.unwrap()
				.page
				.as_ref()
				.unwrap()
				.items[0]
				.visibility == ConnectionVisibility::OnlyYou
		);
		assert!(state.connections.mutation_confirmed("github", "octocat"));
	}
	#[test]
	fn session_interruption_invalidates_pending_list_and_write() {
		let mut state = state();
		let Some(Command::Connections { request }) = state.request_connections() else {
			panic!()
		};
		state.apply(Envelope {
			generation: state.generation,
			event: Event::Failure(Failure::Expired),
		});
		assert!(state.connections.view.as_ref().unwrap().error.is_some());
		state.apply_connections(
			request,
			Ok(ConnectionsPage {
				items: vec![connection(ConnectionVisibility::Everyone)],
				limited: false,
			}),
		);
		assert!(state.connections.view.as_ref().unwrap().page.is_none());
	}
}
