//! Explicitly opened, session-only OAuth2 authorization list, detail and deauthorization.
//! No access or refresh token is retained, and no state is shared with Connections or Spotify.
use crate::{
	Command, State,
	auth::{AuthState, Failure},
};
use model::authorized_apps::{
	AuthorizedAppGrant, AuthorizedApplication, AuthorizedAppsPage, authorization_path,
};

pub struct View {
	pub request: u64,
	pub generation: u64,
	pub loading: bool,
	pub error: Option<&'static str>,
	pub page: Option<AuthorizedAppsPage>,
}
impl View {
	pub fn limited(&self) -> bool {
		self.page.as_ref().is_some_and(|page| page.limited)
	}
}

/// One bounded detail read. The list row is not changed until a matching response arrives.
pub struct DetailView {
	pub request: u64,
	pub generation: u64,
	pub id: String,
	pub loading: bool,
	pub error: Option<&'static str>,
	pub grant: Option<AuthorizedAppGrant>,
}

/// One explicit deauthorization. The row is not removed optimistically; a failed or
/// unconfirmed response leaves the previous list in place.
pub struct Deauthorization {
	pub request: u64,
	pub generation: u64,
	pub id: String,
	pub application_name: String,
	pub saving: bool,
	pub error: Option<&'static str>,
	/// The write may have reached the service. A list refresh is required before another try.
	pub ambiguous: bool,
	/// Increments after a confirmed refetch that no longer lists the application.
	pub confirmed: u64,
}

#[derive(Default)]
pub struct AuthorizedApps {
	pub view: Option<View>,
	pub request: u64,
	pub detail: Option<DetailView>,
	pub deauthorization: Option<Deauthorization>,
}
impl AuthorizedApps {
	/// One shared pipeline; UI can disable actions that the reducer would otherwise reject.
	pub fn pending(&self) -> bool {
		self.view.as_ref().is_some_and(|view| view.loading)
			|| self.detail.as_ref().is_some_and(|detail| detail.loading)
			|| self
				.deauthorization
				.as_ref()
				.is_some_and(|deauthorization| deauthorization.saving)
	}
	pub fn detail_loading(&self, id: &str) -> bool {
		self.detail
			.as_ref()
			.is_some_and(|detail| detail.loading && detail.id == id)
	}
	pub fn detail_error(&self, id: &str) -> Option<&'static str> {
		self.detail
			.as_ref()
			.and_then(|detail| (detail.id == id).then_some(detail.error).flatten())
	}
	pub fn detail_grant(&self, id: &str) -> Option<&AuthorizedAppGrant> {
		self.detail
			.as_ref()
			.and_then(|detail| (detail.id == id).then_some(detail.grant.as_ref()).flatten())
	}
	pub fn deauthorization_saving(&self, id: &str) -> bool {
		self.deauthorization
			.as_ref()
			.is_some_and(|deauthorization| deauthorization.saving && deauthorization.id == id)
	}
	pub fn deauthorization_error(&self, id: &str) -> Option<&'static str> {
		self.deauthorization.as_ref().and_then(|deauthorization| {
			(deauthorization.id == id)
				.then_some(deauthorization.error)
				.flatten()
		})
	}
	pub fn deauthorization_ambiguous(&self, id: &str) -> bool {
		self.deauthorization
			.as_ref()
			.is_some_and(|deauthorization| deauthorization.id == id && deauthorization.ambiguous)
	}
	pub fn deauthorization_confirmed(&self, id: &str) -> bool {
		self.deauthorization
			.as_ref()
			.is_some_and(|deauthorization| {
				deauthorization.confirmed > 0
					&& !deauthorization.saving
					&& deauthorization.error.is_none()
					&& deauthorization.id == id
			})
	}
}

impl State {
	/// Explicitly open the page. A repeated open refreshes the list, while a request already in
	/// flight (or a deauthorization write) is left alone.
	pub fn request_authorized_apps(&mut self) -> Option<Command> {
		if self.authorized_apps.pending() {
			return None;
		}
		self.authorized_apps.detail = None;
		self.authorized_apps.request = self.authorized_apps.request.wrapping_add(1);
		let request = self.authorized_apps.request;
		let allowed = self.demo
			|| (self.auth == AuthState::Authenticated
				&& self.gateway_connected
				&& self.user.is_some());
		self.authorized_apps.view = Some(View {
			request,
			generation: self.generation,
			loading: allowed,
			error: (!allowed)
				.then_some("Authorized apps are unavailable while disconnected or signed out"),
			page: None,
		});
		if self.demo {
			let view = self.authorized_apps.view.as_mut().unwrap();
			view.page = Some(demo_page());
			view.loading = false;
			self.authorized_apps.deauthorization = None;
			return None;
		}
		allowed.then_some(Command::AuthorizedApps { request })
	}

	pub(crate) fn apply_authorized_apps(
		&mut self,
		request: u64,
		result: Result<AuthorizedAppsPage, Failure>,
	) {
		if let Err(failure) = &result
			&& failure.ends_session()
			&& *failure != Failure::Capacity
		{
			self.fail(*failure);
			return;
		}
		let Some(view) = self.authorized_apps.view.as_mut().filter(|view| {
			view.request == request && view.loading && view.generation == self.generation
		}) else {
			return;
		};
		view.loading = false;
		let mut clear_deauthorization = false;
		let mut clear_detail = false;
		match result {
			Ok(page) if page.valid() => {
				view.error = None;
				view.page = Some(page);
				clear_deauthorization = true;
				clear_detail = true;
			}
			Ok(_) | Err(Failure::Protocol | Failure::ProtocolAt(_)) => {
				view.error = Some("Authorized apps were rejected or incompatible");
			}
			Err(Failure::RateLimited) => {
				view.error = Some("Authorized apps are rate limited; wait a moment and retry");
			}
			Err(Failure::Forbidden) => {
				view.error = Some("Authorized apps are unavailable for this account");
			}
			Err(Failure::Capacity) => {
				view.error = Some("Authorized apps exceeded the safe size limit");
			}
			Err(_) => view.error = Some("Authorized apps could not be loaded; retry"),
		}
		if clear_deauthorization {
			self.authorized_apps.deauthorization = None;
		}
		if clear_detail {
			let keep = self.authorized_apps.detail.as_ref().is_some_and(|detail| {
				self.authorized_apps
					.view
					.as_ref()
					.and_then(|view| view.page.as_ref())
					.is_some_and(|page| page.items.iter().any(|grant| grant.id == detail.id))
			});
			if !keep {
				self.authorized_apps.detail = None;
			}
		}
	}

	/// One detail read at a time. The requested row must already be in the bounded list.
	pub fn request_authorized_app_detail(&mut self, id: &str) -> Option<Command> {
		if self.authorized_apps.pending() {
			return None;
		}
		let grant = self
			.authorized_apps
			.view
			.as_ref()?
			.page
			.as_ref()?
			.items
			.iter()
			.find(|grant| grant.id == id)?
			.clone();
		let allowed = self.demo
			|| (self.auth == AuthState::Authenticated
				&& self.gateway_connected
				&& self.user.is_some());
		self.authorized_apps.request = self.authorized_apps.request.wrapping_add(1);
		let request = self.authorized_apps.request;
		self.authorized_apps.detail = Some(DetailView {
			request,
			generation: self.generation,
			id: id.to_owned(),
			loading: allowed,
			error: (!allowed)
				.then_some("Application details are unavailable while disconnected or signed out"),
			grant: None,
		});
		if self.demo {
			let detail = self.authorized_apps.detail.as_mut().unwrap();
			detail.grant = Some(grant);
			detail.loading = false;
			return None;
		}
		allowed.then_some(Command::AuthorizedAppDetail {
			id: id.to_owned(),
			request,
		})
	}

	pub(crate) fn apply_authorized_app_detail(
		&mut self,
		request: u64,
		result: Result<AuthorizedAppGrant, Failure>,
	) {
		if let Err(failure) = &result
			&& failure.ends_session()
			&& *failure != Failure::Capacity
		{
			self.fail(*failure);
			return;
		}
		let Some(detail) = self.authorized_apps.detail.as_mut().filter(|detail| {
			detail.request == request && detail.loading && detail.generation == self.generation
		}) else {
			return;
		};
		detail.loading = false;
		let id = detail.id.clone();
		let mut updated = None;
		let error = match result {
			Ok(grant) if grant.valid() && grant.id == id => {
				updated = Some(grant);
				None
			}
			Ok(_) | Err(Failure::Protocol | Failure::ProtocolAt(_)) => {
				Some("Application details were rejected or incompatible")
			}
			Err(Failure::RateLimited) => {
				Some("Application details are rate limited; wait a moment and retry")
			}
			Err(Failure::Forbidden) => Some("Application details are unavailable for this account"),
			Err(Failure::Capacity) => Some("Application details exceeded the safe size limit"),
			Err(_) => Some("Application details could not be loaded; retry"),
		};
		if let Some(grant) = updated {
			if let Some(detail) = self.authorized_apps.detail.as_mut() {
				detail.error = None;
				detail.grant = Some(grant.clone());
			}
			if let Some(row) = self
				.authorized_apps
				.view
				.as_mut()
				.and_then(|view| view.page.as_mut())
				.and_then(|page| page.items.iter_mut().find(|row| row.id == grant.id))
			{
				*row = grant;
			}
		} else if let Some(detail) = self.authorized_apps.detail.as_mut() {
			detail.error = error;
		}
	}

	/// One deauthorization at a time. The previous list stays visible until a matching refetch
	/// confirms the application is gone. An ambiguous write requires a list refresh first.
	pub fn deauthorize_authorized_app(&mut self, id: &str) -> Option<Command> {
		if self.authorized_apps.pending() {
			return None;
		}
		if self.authorized_apps.deauthorization_ambiguous(id) {
			return None;
		}
		let grant = self
			.authorized_apps
			.view
			.as_ref()?
			.page
			.as_ref()?
			.items
			.iter()
			.find(|grant| grant.id == id)?
			.clone();
		if !grant.can_deauthorize() {
			return None;
		}
		authorization_path(id)?;
		let allowed = self.demo
			|| (self.auth == AuthState::Authenticated
				&& self.gateway_connected
				&& self.user.is_some());
		if !allowed {
			return None;
		}
		self.authorized_apps.request = self.authorized_apps.request.wrapping_add(1);
		let request = self.authorized_apps.request;
		self.authorized_apps.deauthorization = Some(Deauthorization {
			request,
			generation: self.generation,
			id: id.to_owned(),
			application_name: grant.application.name.clone(),
			saving: true,
			error: None,
			ambiguous: false,
			confirmed: 0,
		});
		if self.demo {
			if let Some(page) = self
				.authorized_apps
				.view
				.as_mut()
				.and_then(|view| view.page.as_mut())
			{
				page.items.retain(|grant| grant.id != id);
			}
			self.authorized_apps.detail = None;
			let deauthorization = self.authorized_apps.deauthorization.as_mut().unwrap();
			deauthorization.saving = false;
			deauthorization.confirmed = 1;
			return None;
		}
		Some(Command::DeauthorizeAuthorizedApp {
			id: id.to_owned(),
			request,
		})
	}

	pub(crate) fn apply_authorized_app_deauthorized(
		&mut self,
		request: u64,
		result: Result<AuthorizedAppsPage, Failure>,
	) {
		if let Err(failure) = &result
			&& failure.ends_session()
			&& *failure != Failure::Capacity
		{
			self.fail(*failure);
			return;
		}
		let Some(deauthorization) =
			self.authorized_apps
				.deauthorization
				.as_mut()
				.filter(|deauthorization| {
					deauthorization.request == request
						&& deauthorization.saving
						&& deauthorization.generation == self.generation
				})
		else {
			return;
		};
		deauthorization.saving = false;
		let id = deauthorization.id.clone();
		let mut confirmed = None;
		let (error, ambiguous) = match result {
			Ok(page) if page.valid() && page.items.iter().all(|grant| grant.id != id) => {
				confirmed = Some(page);
				(None, false)
			}
			Ok(_) | Err(Failure::Protocol | Failure::ProtocolAt(_)) => (
				Some("Deauthorization was not confirmed; refresh the list before retrying"),
				true,
			),
			Err(Failure::Ambiguous) => (
				Some("Deauthorization outcome is unknown; refresh the list before retrying"),
				true,
			),
			Err(Failure::RateLimited) => (
				Some("Deauthorization was rate limited; wait a moment and retry"),
				false,
			),
			Err(Failure::Forbidden) => (
				Some("Deauthorization is not permitted for this application"),
				false,
			),
			Err(Failure::Capacity) => (
				Some(
					"Deauthorization response exceeded the safe size limit; refresh the list before retrying",
				),
				true,
			),
			Err(_) => (
				Some("Deauthorization could not be confirmed; refresh the list before retrying"),
				true,
			),
		};
		if let Some(page) = confirmed {
			if let Some(view) = self.authorized_apps.view.as_mut() {
				view.error = None;
				view.page = Some(page);
			}
			if self
				.authorized_apps
				.detail
				.as_ref()
				.is_some_and(|detail| detail.id == id)
			{
				self.authorized_apps.detail = None;
			}
			let deauthorization = self.authorized_apps.deauthorization.as_mut().unwrap();
			deauthorization.error = None;
			deauthorization.ambiguous = false;
			deauthorization.confirmed = deauthorization.confirmed.wrapping_add(1);
		} else if let Some(deauthorization) = self.authorized_apps.deauthorization.as_mut() {
			deauthorization.error = error;
			deauthorization.ambiguous = ambiguous;
		}
	}

	/// Interrupting a session discards the visible list/detail as well as stale request IDs. A
	/// reconnecting user must never confirm a destructive action against an earlier snapshot.
	pub(crate) fn interrupt_authorized_apps(&mut self) {
		if let Some(view) = self.authorized_apps.view.as_mut() {
			view.loading = false;
			view.request = view.request.wrapping_add(1);
			view.page = None;
			view.error = Some("Authorized apps were interrupted; reconnect and reload");
		}
		self.authorized_apps.detail = None;
		if let Some(deauthorization) = self.authorized_apps.deauthorization.as_mut()
			&& deauthorization.saving
		{
			deauthorization.saving = false;
			deauthorization.request = deauthorization.request.wrapping_add(1);
			deauthorization.error =
				Some("Deauthorization was interrupted; refresh the list before retrying");
			deauthorization.ambiguous = true;
		}
	}
}

/// Synthetic rows for the offline preview only. Never evidence of a live authorization.
fn demo_page() -> AuthorizedAppsPage {
	AuthorizedAppsPage {
		items: vec![
			AuthorizedAppGrant {
				id: "100000000000000001".into(),
				application: AuthorizedApplication {
					id: "200000000000000001".into(),
					name: "Synthetic Music".into(),
					description: Some(
						"Synthetic demo application; no service is contacted.".into(),
					),
					icon: None,
					is_verified: Some(true),
					terms_of_service_url: Some("https://example.com/terms".into()),
					privacy_policy_url: Some("https://example.com/privacy".into()),
				},
				scopes: Some(vec!["identify".into(), "connections".into()]),
				authorized_at: Some("2026-01-02T03:04:05Z".into()),
				limited: false,
			},
			AuthorizedAppGrant {
				id: "100000000000000002".into(),
				application: AuthorizedApplication {
					id: "200000000000000002".into(),
					name: "Synthetic Tools".into(),
					description: None,
					icon: None,
					is_verified: None,
					terms_of_service_url: None,
					privacy_policy_url: None,
				},
				scopes: None,
				authorized_at: None,
				limited: false,
			},
		],
		limited: false,
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::{Envelope, Event};
	use model::authorized_apps::MAX_AUTHORIZED_APPS;

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
	fn grant(name: &str) -> AuthorizedAppGrant {
		AuthorizedAppGrant {
			id: "100".into(),
			application: AuthorizedApplication {
				id: "200".into(),
				name: name.into(),
				description: Some("Synthetic".into()),
				icon: None,
				is_verified: Some(true),
				terms_of_service_url: Some("https://example.com/terms".into()),
				privacy_policy_url: Some("https://example.com/privacy".into()),
			},
			scopes: Some(vec!["identify".into()]),
			authorized_at: Some("2026-01-02T03:04:05Z".into()),
			limited: false,
		}
	}
	fn page(items: Vec<AuthorizedAppGrant>) -> AuthorizedAppsPage {
		AuthorizedAppsPage {
			items,
			limited: false,
		}
	}

	#[test]
	fn explicit_open_is_single_flight_and_stale_generations_are_ignored() {
		let mut state = state();
		let Some(Command::AuthorizedApps { request }) = state.request_authorized_apps() else {
			panic!("first open requests the list");
		};
		assert!(
			state.request_authorized_apps().is_none(),
			"one request in flight"
		);
		state.apply_authorized_apps(request.wrapping_add(1), Ok(page(vec![grant("Synthetic")])));
		assert!(state.authorized_apps.view.as_ref().unwrap().loading);
		let next = state.generation.wrapping_add(1);
		state.generation = next;
		state.apply_authorized_apps(request, Ok(page(vec![grant("Synthetic")])));
		assert!(
			state
				.authorized_apps
				.view
				.as_ref()
				.is_some_and(|view| view.loading && view.page.is_none()),
			"an old generation cannot publish state"
		);
	}

	#[test]
	fn empty_error_and_demo_states_keep_loading_and_explicit_failures_distinct() {
		let mut state = state();
		let Some(Command::AuthorizedApps { request }) = state.request_authorized_apps() else {
			panic!()
		};
		state.apply_authorized_apps(request, Ok(page(vec![])));
		let view = state.authorized_apps.view.as_ref().unwrap();
		assert!(!view.loading && view.error.is_none());
		assert_eq!(view.page.as_ref().unwrap().items.len(), 0);
		let Some(Command::AuthorizedApps { request }) = state.request_authorized_apps() else {
			panic!("refresh")
		};
		state.apply_authorized_apps(request, Err(Failure::Forbidden));
		assert!(state.authorized_apps.view.as_ref().unwrap().error.is_some());
		state.gateway_connected = false;
		assert!(
			state.request_authorized_apps().is_none(),
			"offline is explicit"
		);
		assert!(state.authorized_apps.view.as_ref().unwrap().error.is_some());
		state.demo = true;
		assert!(state.request_authorized_apps().is_none());
		let view = state.authorized_apps.view.as_ref().unwrap();
		assert!(!view.loading && view.page.as_ref().unwrap().items.len() <= MAX_AUTHORIZED_APPS);
	}

	#[test]
	fn detail_updates_the_row_and_stale_results_are_ignored() {
		let mut state = state();
		let Some(Command::AuthorizedApps { request }) = state.request_authorized_apps() else {
			panic!()
		};
		state.apply_authorized_apps(request, Ok(page(vec![grant("Synthetic")])));
		let Some(Command::AuthorizedAppDetail { request, .. }) =
			state.request_authorized_app_detail("100")
		else {
			panic!("detail command")
		};
		assert!(
			state.request_authorized_app_detail("100").is_none(),
			"one detail read in flight"
		);
		let mut updated = grant("Synthetic Updated");
		updated.scopes = Some(vec!["identify".into(), "connections".into()]);
		state.apply_authorized_app_detail(request, Ok(updated));
		assert_eq!(
			state
				.authorized_apps
				.view
				.as_ref()
				.unwrap()
				.page
				.as_ref()
				.unwrap()
				.items[0]
				.application
				.name,
			"Synthetic Updated"
		);
		assert_eq!(
			state
				.authorized_apps
				.detail_grant("100")
				.unwrap()
				.scopes
				.as_ref()
				.unwrap()
				.len(),
			2
		);
		let Some(Command::AuthorizedAppDetail { request, .. }) =
			state.request_authorized_app_detail("100")
		else {
			panic!("refresh detail")
		};
		let next = state.generation.wrapping_add(1);
		state.generation = next;
		state.apply_authorized_app_detail(request, Ok(grant("Stale")));
		assert_eq!(
			state
				.authorized_apps
				.view
				.as_ref()
				.unwrap()
				.page
				.as_ref()
				.unwrap()
				.items[0]
				.application
				.name,
			"Synthetic Updated",
			"an old generation cannot publish detail"
		);
	}

	#[test]
	fn deauthorization_requires_confirmation_and_ambiguous_writes_are_not_retried() {
		let mut state = state();
		let Some(Command::AuthorizedApps { request }) = state.request_authorized_apps() else {
			panic!()
		};
		state.apply_authorized_apps(request, Ok(page(vec![grant("Synthetic")])));
		let Some(Command::DeauthorizeAuthorizedApp { request, .. }) =
			state.deauthorize_authorized_app("100")
		else {
			panic!("deauthorization command")
		};
		assert!(
			state
				.authorized_apps
				.view
				.as_ref()
				.unwrap()
				.page
				.as_ref()
				.unwrap()
				.items
				.iter()
				.any(|grant| grant.id == "100"),
			"no optimistic removal"
		);
		state.apply_authorized_app_deauthorized(request, Err(Failure::Ambiguous));
		assert!(
			state
				.authorized_apps
				.view
				.as_ref()
				.unwrap()
				.page
				.as_ref()
				.unwrap()
				.items
				.iter()
				.any(|grant| grant.id == "100")
		);
		assert!(state.authorized_apps.deauthorization_ambiguous("100"));
		assert!(
			state.deauthorize_authorized_app("100").is_none(),
			"an ambiguous write is not retried"
		);
		let Some(Command::AuthorizedApps { request }) = state.request_authorized_apps() else {
			panic!("refresh after an ambiguous write")
		};
		state.apply_authorized_apps(request, Ok(page(vec![])));
		assert!(!state.authorized_apps.deauthorization_ambiguous("100"));
		assert!(
			state
				.authorized_apps
				.view
				.as_ref()
				.unwrap()
				.page
				.as_ref()
				.unwrap()
				.items
				.is_empty()
		);
	}

	#[test]
	fn confirmed_deauthorization_removes_the_row_and_clears_detail() {
		let mut state = state();
		let Some(Command::AuthorizedApps { request }) = state.request_authorized_apps() else {
			panic!()
		};
		state.apply_authorized_apps(request, Ok(page(vec![grant("Synthetic")])));
		let Some(Command::AuthorizedAppDetail { request, .. }) =
			state.request_authorized_app_detail("100")
		else {
			panic!()
		};
		state.apply_authorized_app_detail(request, Ok(grant("Synthetic")));
		let Some(Command::DeauthorizeAuthorizedApp { request, .. }) =
			state.deauthorize_authorized_app("100")
		else {
			panic!()
		};
		state.apply_authorized_app_deauthorized(request, Ok(page(vec![])));
		assert!(
			state
				.authorized_apps
				.view
				.as_ref()
				.unwrap()
				.page
				.as_ref()
				.unwrap()
				.items
				.is_empty()
		);
		assert!(state.authorized_apps.detail.is_none());
		assert!(state.authorized_apps.deauthorization_confirmed("100"));
	}

	#[test]
	fn session_interruption_invalidates_pending_list_detail_and_write() {
		let mut connected = state();
		let mut state = state();
		let Some(Command::AuthorizedApps { request }) = state.request_authorized_apps() else {
			panic!()
		};
		state.apply(Envelope {
			generation: state.generation,
			event: Event::Failure(Failure::Expired),
		});
		assert!(state.authorized_apps.view.as_ref().unwrap().error.is_some());
		state.apply_authorized_apps(request, Ok(page(vec![grant("Synthetic")])));
		assert!(state.authorized_apps.view.as_ref().unwrap().page.is_none());

		let Some(Command::AuthorizedApps { request }) = connected.request_authorized_apps() else {
			panic!()
		};
		connected.apply_authorized_apps(request, Ok(page(vec![grant("Synthetic")])));
		connected.apply(Envelope {
			generation: connected.generation,
			event: Event::Disconnected,
		});
		assert!(
			connected
				.authorized_apps
				.view
				.as_ref()
				.unwrap()
				.page
				.is_none()
		);
		assert!(connected.deauthorize_authorized_app("100").is_none());
	}
}
