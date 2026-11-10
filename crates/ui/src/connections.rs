//! Settings > Connections: an explicit, bounded read of the account's linked services.
//!
//! The list route is documented for OAuth2 applications; normal-session delivery and the
//! visibility PATCH are documented by Userdoccers and remain unofficial/live-unverified:
//! <https://github.com/discord-userdoccers/discord-userdoccers/blob/master/pages/resources/connected-accounts.mdx>
//!
//! Nothing here reuses profile connection summaries, polls, unlinks, changes friend sync, or
//! starts a new authorization flow. Spotify playback has its own independent reader.
use crate::{MessagingUi, design, icons};
use client_core::{Command, State};
use egui::RichText;
use model::connections::{Connection, ConnectionVisibility};

impl MessagingUi {
	pub(super) fn connections_settings(
		&mut self,
		ui: &mut egui::Ui,
		state: &mut State,
		commands: &mut Vec<Command>,
	) {
		let p = design::palette(ui);
		ui.label(
			RichText::new(self.language.text("settings-connections-unofficial"))
				.size(13.0)
				.color(p.muted),
		);
		ui.add_space(4.0);
		if design::text_action(
			ui,
			&self.language.text("settings-connections-authorized-apps"),
		)
		.clicked()
		{
			self.open_authorized_apps_settings();
		}
		ui.add_space(8.0);
		let page = state
			.connections
			.view
			.as_ref()
			.and_then(|view| view.page.clone());
		let loading = state
			.connections
			.view
			.as_ref()
			.is_some_and(|view| view.loading);
		let error = state.connections.view.as_ref().and_then(|view| view.error);
		if loading {
			ui.horizontal(|ui| {
				ui.spacing_mut().item_spacing.x = 8.0;
				ui.add(egui::Spinner::new().size(16.0));
				ui.label(
					RichText::new(self.language.text("settings-connections-loading"))
						.size(14.0)
						.color(p.muted),
				);
			});
			return;
		}
		if let Some(error) = error {
			design::notice(ui, design::Level::Error, error);
			if design::text_action(ui, &self.language.text("settings-connections-try-again"))
				.clicked() && let Some(command) = state.request_connections()
			{
				commands.push(command);
			}
			return;
		}
		let Some(page) = page else {
			design::notice(
				ui,
				design::Level::Error,
				&self.language.text("settings-connections-unavailable"),
			);
			return;
		};
		if page.items.is_empty() {
			design::card(ui, |ui| {
				ui.spacing_mut().item_spacing.y = 4.0;
				ui.label(
					design::semibold(ui, self.language.text("settings-connections-empty"), 16.0)
						.color(p.text_strong),
				);
				ui.label(
					RichText::new(self.language.text("settings-connections-empty-description"))
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
				&self.language.text("settings-connections-limited"),
			);
			ui.add_space(8.0);
		}
		for connection in &page.items {
			self.connection_row(ui, state, commands, connection);
			ui.add_space(8.0);
		}
	}

	fn connection_row(
		&mut self,
		ui: &mut egui::Ui,
		state: &mut State,
		commands: &mut Vec<Command>,
		connection: &Connection,
	) {
		let p = design::palette(ui);
		let kind = connection.kind.clone();
		let id = connection.id.clone();
		design::card(ui, |ui| {
			ui.spacing_mut().item_spacing.y = 6.0;
			ui.horizontal_top(|ui| {
				ui.vertical(|ui| {
					ui.spacing_mut().item_spacing.y = 2.0;
					ui.add(
						egui::Label::new(
							design::semibold(ui, service_name(&connection.kind), 16.0)
								.color(p.text_strong),
						)
						.truncate(),
					);
					ui.add(
						egui::Label::new(RichText::new(&connection.name).size(14.0).color(p.text))
							.truncate(),
					);
					ui.add(
						egui::Label::new(
							RichText::new(self.language.text("settings-connections-account-id"))
								.size(12.0)
								.color(p.muted),
						)
						.truncate(),
					);
					ui.add(
						egui::Label::new(
							RichText::new(&connection.id)
								.monospace()
								.size(12.0)
								.color(p.muted),
						)
						.truncate(),
					);
				});
				ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
					self.verification_chip(ui, connection.verified);
				});
			});
			ui.add_space(4.0);
			self.connection_fact(
				ui,
				&self.language.text("settings-connections-activity"),
				optional_flag(&self.language, connection.show_activity),
			);
			self.connection_fact(
				ui,
				&self.language.text("settings-connections-friend-sync"),
				optional_flag(&self.language, connection.friend_sync),
			);
			self.connection_fact(
				ui,
				&self.language.text("settings-connections-two-way-link"),
				optional_flag(&self.language, connection.two_way_link),
			);
			self.connection_fact(
				ui,
				&self
					.language
					.text("settings-connections-metadata-visibility"),
				visibility_text(&self.language, connection.metadata_visibility),
			);
			if connection.revoked == Some(true) {
				design::notice(
					ui,
					design::Level::Warning,
					&self.language.text("settings-connections-revoked"),
				);
			}
			if connection.limited {
				design::notice(
					ui,
					design::Level::Warning,
					&self.language.text("settings-connections-field-unavailable"),
				);
			}
			let saving = state.connections.mutation_saving(&kind, &id);
			if connection.can_change_visibility() {
				let mut visible = connection.visibility == ConnectionVisibility::Everyone;
				let response = ui.add_enabled_ui(!saving, |ui| {
					design::switch(
						ui,
						&self.language.text("settings-connections-show-on-profile"),
						Some(&visibility_text(&self.language, connection.visibility)),
						&mut visible,
					)
				});
				if response.inner.changed()
					&& let Some(command) = state.set_connection_visibility(&kind, &id, visible)
				{
					commands.push(command);
				}
			} else {
				ui.label(
					RichText::new(
						self.language
							.text("settings-connections-visibility-unavailable"),
					)
					.size(12.0)
					.color(p.muted),
				);
			}
			if saving {
				ui.horizontal(|ui| {
					ui.add(egui::Spinner::new().size(12.0));
					ui.label(
						RichText::new(self.language.text("settings-connections-saving"))
							.size(12.0)
							.color(p.muted),
					);
				});
			}
			if let Some(error) = state.connections.mutation_error(&kind, &id) {
				design::notice(ui, design::Level::Error, error);
			} else if state.connections.mutation_confirmed(&kind, &id) {
				ui.label(
					RichText::new(self.language.text("settings-connections-saved"))
						.size(12.0)
						.color(p.positive),
				);
			}
		});
	}

	fn connection_fact(&self, ui: &mut egui::Ui, label: &str, value: String) {
		let p = design::palette(ui);
		ui.horizontal(|ui| {
			ui.spacing_mut().item_spacing.x = 6.0;
			ui.label(RichText::new(label).size(12.0).color(p.muted));
			ui.label(RichText::new(value).size(12.0).color(p.text));
		});
	}

	fn verification_chip(&self, ui: &mut egui::Ui, verified: Option<bool>) {
		let p = design::palette(ui);
		let (text, color) = match verified {
			Some(true) => (
				self.language.text("settings-connections-verified"),
				p.positive,
			),
			Some(false) => (
				self.language.text("settings-connections-not-verified"),
				p.muted,
			),
			None => (
				self.language
					.text("settings-connections-verification-unknown"),
				p.muted,
			),
		};
		ui.horizontal(|ui| {
			ui.spacing_mut().item_spacing.x = 4.0;
			icons::inline(ui, icons::Icon::Check, 14.0, color);
			ui.label(RichText::new(text).size(12.0).color(color));
		});
	}
}

fn service_name(kind: &str) -> String {
	match kind {
		"amazon-music" => "Amazon Music",
		"battlenet" => "Battle.net",
		"bluesky" => "Bluesky",
		"bungie" => "Bungie.net",
		"contacts" => "Contact Sync",
		"crunchyroll" => "Crunchyroll",
		"domain" => "Domain",
		"ebay" => "eBay",
		"epicgames" => "Epic Games",
		"facebook" => "Facebook",
		"github" => "GitHub",
		"instagram" => "Instagram",
		"leagueoflegends" => "League of Legends",
		"mastodon" => "Mastodon",
		"paypal" => "PayPal",
		"playstation" | "playstation-stg" => "PlayStation Network",
		"reddit" => "Reddit",
		"roblox" => "Roblox",
		"riotgames" => "Riot Games",
		"samsung" => "Samsung Galaxy",
		"soundcloud" => "SoundCloud",
		"spotify" => "Spotify",
		"skype" => "Skype",
		"steam" => "Steam",
		"tiktok" => "TikTok",
		"twitch" => "Twitch",
		"twitter" => "X (Twitter)",
		"xbox" => "Xbox",
		"youtube" => "YouTube",
		_ => kind,
	}
	.to_owned()
}

fn optional_flag(language: &crate::i18n::Language, value: Option<bool>) -> String {
	language.text(match value {
		Some(true) => "settings-connections-on",
		Some(false) => "settings-connections-off",
		None => "settings-connections-unknown",
	})
}

fn visibility_text(language: &crate::i18n::Language, visibility: ConnectionVisibility) -> String {
	language.text(match visibility {
		ConnectionVisibility::Everyone => "settings-connections-everyone",
		ConnectionVisibility::OnlyYou => "settings-connections-only-you",
		ConnectionVisibility::Unknown => "settings-connections-unknown",
	})
}
