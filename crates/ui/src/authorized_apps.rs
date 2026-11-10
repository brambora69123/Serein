//! Settings > Authorized Apps: an explicit, bounded read of the account's OAuth2 grants.
//!
//! The list, detail and delete routes are documented by Userdoccers, not Discord's public
//! documentation, and normal-session delivery remains unofficial/live-unverified:
//! <https://docs.discord.food/topics/oauth2>
//!
//! Nothing here reads an access or refresh token, starts an authorization flow, intercepts a
//! callback, or reuses the Connections page. Deauthorization is per application, confirmed and
//! single-flight; an ambiguous write requires a list refresh before another attempt.
use crate::{MessagingUi, design, dialog, icons};
use client_core::{Command, State};
use egui::RichText;
use model::authorized_apps::{AuthorizedAppGrant, safe_policy_url};

#[derive(Default)]
pub(super) struct Ui {
	query: String,
	/// Authorization ID awaiting explicit confirmation.
	pub(super) confirm: Option<String>,
	/// Policy destination awaiting the shared external-link confirmation.
	opening: Option<String>,
}

impl MessagingUi {
	pub(super) fn authorized_apps_settings(
		&mut self,
		ui: &mut egui::Ui,
		state: &mut State,
		commands: &mut Vec<Command>,
	) {
		let p = design::palette(ui);
		ui.label(
			RichText::new(self.language.text("settings-authorized-apps-unofficial"))
				.size(13.0)
				.color(p.muted),
		);
		ui.add_space(8.0);
		let search = self.language.text("settings-authorized-apps-search");
		design::input(
			ui,
			egui::TextEdit::singleline(&mut self.settings.authorized_apps.query)
				.hint_text(search)
				.char_limit(128),
		);
		ui.add_space(8.0);
		let page = state
			.authorized_apps
			.view
			.as_ref()
			.and_then(|view| view.page.clone());
		let loading = state
			.authorized_apps
			.view
			.as_ref()
			.is_some_and(|view| view.loading);
		let error = state
			.authorized_apps
			.view
			.as_ref()
			.and_then(|view| view.error);
		if loading {
			ui.horizontal(|ui| {
				ui.spacing_mut().item_spacing.x = 8.0;
				ui.add(egui::Spinner::new().size(16.0));
				ui.label(
					RichText::new(self.language.text("settings-authorized-apps-loading"))
						.size(14.0)
						.color(p.muted),
				);
			});
			return;
		}
		if let Some(error) = error {
			design::notice(ui, design::Level::Error, error);
			if design::text_action(
				ui,
				&self.language.text("settings-authorized-apps-try-again"),
			)
			.clicked() && let Some(command) = state.request_authorized_apps()
			{
				commands.push(command);
			}
			return;
		}
		let Some(page) = page else {
			design::notice(
				ui,
				design::Level::Error,
				&self.language.text("settings-authorized-apps-unavailable"),
			);
			return;
		};
		if let Some(deauthorization) = state.authorized_apps.deauthorization.as_ref()
			&& deauthorization.confirmed > 0
			&& !deauthorization.saving
			&& deauthorization.error.is_none()
		{
			design::notice(
				ui,
				design::Level::Success,
				&self.language.text("settings-authorized-apps-deauthorized"),
			);
			ui.add_space(8.0);
		}
		if page.items.is_empty() {
			design::card(ui, |ui| {
				ui.spacing_mut().item_spacing.y = 4.0;
				ui.label(
					design::semibold(
						ui,
						self.language.text("settings-authorized-apps-empty"),
						16.0,
					)
					.color(p.text_strong),
				);
				ui.label(
					RichText::new(
						self.language
							.text("settings-authorized-apps-empty-description"),
					)
					.size(13.0)
					.color(p.muted),
				);
			});
			return;
		}
		if page.limited {
			design::notice(
				ui,
				design::Level::Warning,
				&self.language.text("settings-authorized-apps-limited"),
			);
			ui.add_space(8.0);
		}
		let query = self.settings.authorized_apps.query.to_lowercase();
		let mut shown = 0usize;
		for grant in &page.items {
			if !query.is_empty() && !grant_matches(grant, &query) {
				continue;
			}
			shown += 1;
			self.authorized_app_row(ui, state, commands, grant);
			ui.add_space(8.0);
		}
		if shown == 0 {
			design::card(ui, |ui| {
				ui.label(
					RichText::new(
						self.language
							.text("settings-authorized-apps-no-search-results"),
					)
					.size(13.0)
					.color(p.muted),
				);
			});
		}
		self.authorized_app_confirmation(ui, state, commands);
		crate::markdown::confirm_external_link(
			ui.ctx(),
			&mut self.settings.authorized_apps.opening,
			self.reading_preferences.confirm_external_links,
		);
	}

	fn authorized_app_row(
		&mut self,
		ui: &mut egui::Ui,
		state: &mut State,
		commands: &mut Vec<Command>,
		grant: &AuthorizedAppGrant,
	) {
		let p = design::palette(ui);
		let id = grant.id.clone();
		let pending = state.authorized_apps.pending();
		let saving = state.authorized_apps.deauthorization_saving(&id);
		let ambiguous = state.authorized_apps.deauthorization_ambiguous(&id);
		design::card(ui, |ui| {
			ui.spacing_mut().item_spacing.y = 6.0;
			ui.horizontal_top(|ui| {
				ui.vertical(|ui| {
					ui.spacing_mut().item_spacing.y = 2.0;
					ui.add(
						egui::Label::new(
							design::semibold(ui, &grant.application.name, 16.0)
								.color(p.text_strong),
						)
						.truncate(),
					);
					ui.add(
						egui::Label::new(
							RichText::new(&grant.application.id)
								.monospace()
								.size(12.0)
								.color(p.muted),
						)
						.truncate(),
					);
				});
				ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
					self.authorized_app_verification_chip(ui, grant.application.is_verified);
				});
			});
			ui.add_space(4.0);
			self.authorized_app_fact(
				ui,
				&self.language.text("settings-authorized-apps-scopes"),
				scopes_text(&self.language, grant),
			);
			self.authorized_app_fact(
				ui,
				&self.language.text("settings-authorized-apps-authorized"),
				grant
					.authorized_at
					.clone()
					.unwrap_or_else(|| self.language.text("settings-authorized-apps-unknown")),
			);
			if grant.limited {
				design::notice(
					ui,
					design::Level::Warning,
					&self
						.language
						.text("settings-authorized-apps-field-unavailable"),
				);
			}
			ui.horizontal(|ui| {
				ui.spacing_mut().item_spacing.x = 8.0;
				ui.add_enabled_ui(!pending, |ui| {
					if design::text_action(
						ui,
						&self.language.text("settings-authorized-apps-details"),
					)
					.clicked() && let Some(command) = state.request_authorized_app_detail(&id)
					{
						commands.push(command);
					}
				});
				ui.add_enabled_ui(!pending && !ambiguous && grant.can_deauthorize(), |ui| {
					if design::button(
						ui,
						&self.language.text("settings-authorized-apps-deauthorize"),
						design::ButtonKind::Danger,
					)
					.clicked()
					{
						self.settings.authorized_apps.confirm = Some(id.clone());
					}
				});
			});
			if !grant.can_deauthorize() {
				ui.label(
					RichText::new(
						self.language
							.text("settings-authorized-apps-deauthorize-unavailable"),
					)
					.size(12.0)
					.color(p.muted),
				);
			}
			if saving {
				ui.horizontal(|ui| {
					ui.add(egui::Spinner::new().size(12.0));
					ui.label(
						RichText::new(self.language.text("settings-authorized-apps-deauthorizing"))
							.size(12.0)
							.color(p.muted),
					);
				});
			}
			if let Some(error) = state.authorized_apps.deauthorization_error(&id) {
				design::notice(ui, design::Level::Error, error);
				if ambiguous
					&& design::text_action(
						ui,
						&self.language.text("settings-authorized-apps-refresh"),
					)
					.clicked() && let Some(command) = state.request_authorized_apps()
				{
					commands.push(command);
				}
			} else if state.authorized_apps.deauthorization_confirmed(&id) {
				ui.label(
					RichText::new(self.language.text("settings-authorized-apps-deauthorized"))
						.size(12.0)
						.color(p.positive),
				);
			}
			self.authorized_app_detail(ui, state, commands, grant);
		});
	}

	fn authorized_app_detail(
		&mut self,
		ui: &mut egui::Ui,
		state: &mut State,
		commands: &mut Vec<Command>,
		grant: &AuthorizedAppGrant,
	) {
		let id = grant.id.as_str();
		if state.authorized_apps.detail_loading(id) {
			ui.horizontal(|ui| {
				ui.add(egui::Spinner::new().size(12.0));
				ui.label(
					RichText::new(
						self.language
							.text("settings-authorized-apps-details-loading"),
					)
					.size(12.0)
					.color(design::palette(ui).muted),
				);
			});
			return;
		}
		if let Some(error) = state.authorized_apps.detail_error(id) {
			design::notice(ui, design::Level::Error, error);
			if design::text_action(
				ui,
				&self.language.text("settings-authorized-apps-try-again"),
			)
			.clicked() && let Some(command) = state.request_authorized_app_detail(id)
			{
				commands.push(command);
			}
			return;
		}
		let Some(detail) = state.authorized_apps.detail_grant(id) else {
			return;
		};
		let p = design::palette(ui);
		ui.add_space(4.0);
		design::card_divider(ui);
		ui.add_space(4.0);
		if let Some(description) = &detail.application.description {
			ui.add(egui::Label::new(RichText::new(description).size(13.0).color(p.text)).wrap());
		}
		self.authorized_app_fact(
			ui,
			&self
				.language
				.text("settings-authorized-apps-authorization-id"),
			detail.id.clone(),
		);
		self.authorized_app_fact(
			ui,
			&self.language.text("settings-authorized-apps-scopes"),
			scopes_text(&self.language, detail),
		);
		self.authorized_app_fact(
			ui,
			&self.language.text("settings-authorized-apps-authorized"),
			detail
				.authorized_at
				.clone()
				.unwrap_or_else(|| self.language.text("settings-authorized-apps-unknown")),
		);
		self.authorized_app_policy_links(ui, detail);
	}

	fn authorized_app_policy_links(&mut self, ui: &mut egui::Ui, grant: &AuthorizedAppGrant) {
		let p = design::palette(ui);
		let mut shown = false;
		ui.horizontal_wrapped(|ui| {
			ui.spacing_mut().item_spacing.x = 12.0;
			if let Some(url) = grant
				.application
				.terms_of_service_url
				.as_deref()
				.and_then(safe_policy_url)
			{
				if ui
					.link(self.language.text("settings-authorized-apps-terms"))
					.clicked()
				{
					self.settings.authorized_apps.opening = Some(url.to_owned());
				}
				shown = true;
			}
			if let Some(url) = grant
				.application
				.privacy_policy_url
				.as_deref()
				.and_then(safe_policy_url)
			{
				if ui
					.link(self.language.text("settings-authorized-apps-privacy"))
					.clicked()
				{
					self.settings.authorized_apps.opening = Some(url.to_owned());
				}
				shown = true;
			}
		});
		if !shown {
			ui.label(
				RichText::new(
					self.language
						.text("settings-authorized-apps-no-policy-links"),
				)
				.size(12.0)
				.color(p.muted),
			);
		}
	}

	fn authorized_app_fact(&self, ui: &mut egui::Ui, label: &str, value: String) {
		let p = design::palette(ui);
		ui.horizontal(|ui| {
			ui.spacing_mut().item_spacing.x = 6.0;
			ui.label(RichText::new(label).size(12.0).color(p.muted));
			ui.add(egui::Label::new(RichText::new(value).size(12.0).color(p.text)).truncate());
		});
	}

	fn authorized_app_verification_chip(&self, ui: &mut egui::Ui, verified: Option<bool>) {
		let p = design::palette(ui);
		let (text, color) = match verified {
			Some(true) => (
				self.language.text("settings-authorized-apps-verified"),
				p.positive,
			),
			Some(false) => (
				self.language.text("settings-authorized-apps-not-verified"),
				p.muted,
			),
			None => (
				self.language
					.text("settings-authorized-apps-verification-unknown"),
				p.muted,
			),
		};
		ui.horizontal(|ui| {
			ui.spacing_mut().item_spacing.x = 4.0;
			icons::inline(ui, icons::Icon::Check, 14.0, color);
			ui.label(RichText::new(text).size(12.0).color(color));
		});
	}

	fn authorized_app_confirmation(
		&mut self,
		ui: &mut egui::Ui,
		state: &mut State,
		commands: &mut Vec<Command>,
	) {
		let Some(id) = self.settings.authorized_apps.confirm.clone() else {
			return;
		};
		let Some(grant) = state
			.authorized_apps
			.view
			.as_ref()
			.and_then(|view| view.page.as_ref())
			.and_then(|page| page.items.iter().find(|grant| grant.id == id))
			.cloned()
		else {
			self.settings.authorized_apps.confirm = None;
			return;
		};
		let pending = state.authorized_apps.pending();
		let ambiguous = state.authorized_apps.deauthorization_ambiguous(&id);
		let mut confirmation = dialog::Confirm::new(
			("deauthorize-authorized-app", &id),
			self.language.text("settings-authorized-apps-confirm-title"),
			format!(
				"{} {}",
				self.language
					.text("settings-authorized-apps-confirm-message"),
				grant.application.name
			),
		)
		.danger()
		.confirm_label(self.language.text("settings-authorized-apps-deauthorize"))
		.cancel_label(self.language.text("settings-authorized-apps-cancel"))
		.enabled(!pending && !ambiguous);
		if ambiguous {
			confirmation = confirmation.note(
				dialog::Level::Warning,
				self.language
					.text("settings-authorized-apps-refresh-before-retry"),
			);
		}
		match confirmation.show(ui.ctx()) {
			Some(dialog::Choice::Confirmed) => {
				self.settings.authorized_apps.confirm = None;
				if let Some(command) = state.deauthorize_authorized_app(&id) {
					commands.push(command);
				}
			}
			Some(dialog::Choice::Cancelled) => self.settings.authorized_apps.confirm = None,
			None => {}
		}
	}
}

fn grant_matches(grant: &AuthorizedAppGrant, query: &str) -> bool {
	grant.application.name.to_lowercase().contains(query)
		|| grant.application.id.contains(query)
		|| grant.id.contains(query)
		|| grant.scopes.as_ref().is_some_and(|scopes| {
			scopes
				.iter()
				.any(|scope| scope.to_lowercase().contains(query))
		})
}

fn scopes_text(language: &crate::i18n::Language, grant: &AuthorizedAppGrant) -> String {
	match &grant.scopes {
		Some(scopes) if scopes.is_empty() => language.text("settings-authorized-apps-no-scopes"),
		Some(scopes) => scopes.join(", "),
		None => language.text("settings-authorized-apps-scopes-unknown"),
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn text_rect(shape: &egui::Shape, label: &str) -> Option<egui::Rect> {
		match shape {
			egui::Shape::Text(text) => {
				(text.galley.job.text == label).then(|| text.visual_bounding_rect())
			}
			egui::Shape::Vec(shapes) => shapes.iter().find_map(|shape| text_rect(shape, label)),
			_ => None,
		}
	}

	#[test]
	fn policy_links_request_confirmation_without_opening_egui_urls() {
		let ctx = egui::Context::default();
		let mut view = MessagingUi::default();
		let mut state = State {
			demo: true,
			..Default::default()
		};
		assert!(state.request_authorized_apps().is_none());
		let id = state
			.authorized_apps
			.view
			.as_ref()
			.unwrap()
			.page
			.as_ref()
			.unwrap()
			.items[0]
			.id
			.clone();
		assert!(state.request_authorized_app_detail(&id).is_none());
		let mut render = |events| {
			let mut commands = Vec::new();
			let output = ctx.run_ui(
				egui::RawInput {
					screen_rect: Some(egui::Rect::from_min_size(
						egui::Pos2::ZERO,
						egui::vec2(720.0, 1000.0),
					)),
					events,
					focused: true,
					..Default::default()
				},
				|ui| view.authorized_apps_settings(ui, &mut state, &mut commands),
			);
			assert!(commands.is_empty());
			output
		};
		let warmup = render(vec![]);
		warmup.drop_without_applying_deltas();
		let output = render(vec![]);
		let point = output
			.shapes
			.iter()
			.find_map(|shape| text_rect(&shape.shape, "Terms of Service"))
			.expect("policy link")
			.center();
		output.drop_without_applying_deltas();
		for pressed in [true, false] {
			let output = render(vec![
				egui::Event::PointerMoved(point),
				egui::Event::PointerButton {
					pos: point,
					button: egui::PointerButton::Primary,
					pressed,
					modifiers: egui::Modifiers::NONE,
				},
			]);
			assert!(
				output
					.platform_output
					.commands
					.iter()
					.all(|command| !matches!(command, egui::OutputCommand::OpenUrl(_)))
			);
			output.drop_without_applying_deltas();
		}
		assert_eq!(
			view.settings.authorized_apps.opening.as_deref(),
			Some("https://example.com/terms")
		);
	}
}
