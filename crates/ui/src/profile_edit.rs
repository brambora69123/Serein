//! One bounded, session-only draft for the account's global profile.
use crate::{avatars::Avatars, design, icons};
use client_core::{Command, State};
use model::{Id, ProfileEdit, UserProfile};

#[derive(Clone, PartialEq, Eq)]
struct Draft {
	name: String,
	bio: String,
	pronouns: String,
	color: Option<u32>,
	/// `Some(Some(uri))` uploads a new picture, `Some(None)` removes the current one.
	avatar: Option<Option<String>>,
	/// The banner follows the same pending-dirty lifecycle, but is Nitro-gated.
	banner: Option<Option<String>>,
	avatar_decoration: Option<Id>,
	nameplate: Option<Id>,
	profile_effect: Option<Id>,
}
impl Draft {
	fn from_profile(profile: &UserProfile) -> Self {
		Self {
			name: profile.global_name.clone().unwrap_or_default(),
			bio: profile.bio.clone(),
			pronouns: profile.pronouns.clone(),
			color: profile.accent_color,
			avatar: None,
			banner: None,
			avatar_decoration: profile
				.user
				.cosmetics
				.as_deref()
				.and_then(|cosmetics| cosmetics.decoration.as_ref())
				.map(|decoration| decoration.sku),
			nameplate: profile
				.user
				.cosmetics
				.as_deref()
				.and_then(|cosmetics| cosmetics.nameplate.as_ref())
				.map(|nameplate| nameplate.sku),
			profile_effect: profile.effect_sku,
		}
	}
	fn changes(&self, profile: &UserProfile) -> ProfileEdit {
		let name = (!self.name.is_empty()).then(|| self.name.clone());
		ProfileEdit {
			global_name: (name != profile.global_name).then_some(name),
			bio: (self.bio != profile.bio).then(|| self.bio.clone()),
			pronouns: (self.pronouns != profile.pronouns).then(|| self.pronouns.clone()),
			accent_color: (self.color != profile.accent_color).then_some(self.color),
			avatar: match &self.avatar {
				// Removing a picture the account does not have is not a change.
				Some(None) if profile.user.avatar.is_none() => None,
				other => other.clone(),
			},
			banner: match &self.banner {
				Some(None) if profile.banner.is_none() => None,
				other => other.clone(),
			},
			avatar_decoration_sku_id: (self.avatar_decoration
				!= profile
					.user
					.cosmetics
					.as_deref()
					.and_then(|cosmetics| cosmetics.decoration.as_ref())
					.map(|decoration| decoration.sku))
			.then_some(self.avatar_decoration),
			nameplate_sku_id: (self.nameplate
				!= profile
					.user
					.cosmetics
					.as_deref()
					.and_then(|cosmetics| cosmetics.nameplate.as_ref())
					.map(|nameplate| nameplate.sku))
			.then_some(self.nameplate),
			profile_effect_id: (self.profile_effect != profile.effect_sku)
				.then_some(self.profile_effect),
		}
	}
	fn rebase(&mut self, before: &Self, fresh: &Self) {
		if self.name == before.name {
			self.name = fresh.name.clone();
		}
		if self.bio == before.bio {
			self.bio = fresh.bio.clone();
		}
		if self.pronouns == before.pronouns {
			self.pronouns = fresh.pronouns.clone();
		}
		if self.color == before.color {
			self.color = fresh.color;
		}
		if self.avatar_decoration == before.avatar_decoration {
			self.avatar_decoration = fresh.avatar_decoration;
		}
		if self.nameplate == before.nameplate {
			self.nameplate = fresh.nameplate;
		}
		if self.profile_effect == before.profile_effect {
			self.profile_effect = fresh.profile_effect;
		}
	}
}

/// What the preview card paints inside the avatar circle.
enum Picture<'a> {
	/// The account's current picture, fetched like any other avatar.
	Remote,
	/// A locally chosen image that has not been saved yet.
	Pending(&'a egui::TextureHandle),
	/// A locally chosen image that was just saved; shown until the CDN copy is fetched.
	Saved(&'a egui::TextureHandle),
	/// Removal is pending, so the initial placeholder is shown.
	Removed,
}

enum BannerPreview<'a> {
	Remote,
	Pending(&'a egui::TextureHandle),
	Saved(&'a egui::TextureHandle),
	Removed,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum AvatarAction {
	Pick,
	BannerPick,
	Remove,
	BannerRemove,
	Undo,
	BannerUndo,
	ReloadCollectibles,
}

#[derive(Default)]
pub(super) struct Editor {
	generation: Option<u64>,
	draft: Option<Draft>,
	baseline: Option<Draft>,
	submitted: Option<u64>,
	saved: bool,
	preview_link: Option<String>,
	/// Set when the picture button is pressed; the desktop shell opens the native picker.
	pub avatar_request: Option<(u64, Id, u64)>,
	pub banner_request: Option<(u64, Id, u64)>,
	revision: u64,
	choosing: bool,
	choosing_banner: bool,
	pending_banner: Option<egui::TextureHandle>,
	saved_banner: Option<(Option<String>, egui::TextureHandle)>,
	pending_picture: Option<egui::TextureHandle>,
	/// Local copy of the last saved picture, keyed by the avatar hash the service returned.
	saved_picture: Option<(Option<String>, egui::TextureHandle)>,
}
impl Editor {
	/// Result of the native picker. Errors are returned so the caller can toast them.
	pub fn accept_avatar(
		&mut self,
		ctx: &egui::Context,
		request: (u64, Id, u64),
		result: Result<Option<(String, egui::ColorImage)>, &'static str>,
	) -> Result<(), &'static str> {
		if !self.choosing
			|| request
				!= (
					self.generation.unwrap_or_default(),
					request.1,
					self.revision,
				) {
			return Ok(());
		}
		self.choosing = false;
		match result {
			Ok(Some((uri, image))) => {
				if !model::valid_avatar_uri(&uri)
					|| image.resolution()[0] > 512
					|| image.resolution()[1] > 512
				{
					return Err("That picture is too large; choose a simpler image");
				}
				let Some(draft) = self.draft.as_mut() else {
					return Ok(());
				};
				draft.avatar = Some(Some(uri));
				self.pending_picture = Some(ctx.load_texture(
					"profile-avatar-preview",
					image,
					egui::TextureOptions::LINEAR,
				));
				self.saved = false;
				Ok(())
			}
			Ok(None) => Ok(()),
			Err(error) => Err(error),
		}
	}

	pub fn accept_banner(
		&mut self,
		ctx: &egui::Context,
		request: (u64, Id, u64),
		result: Result<Option<(String, egui::ColorImage)>, &'static str>,
	) -> Result<(), &'static str> {
		if !self.choosing_banner
			|| request
				!= (
					self.generation.unwrap_or_default(),
					request.1,
					self.revision,
				) {
			return Ok(());
		}
		self.choosing_banner = false;
		match result {
			Ok(Some((uri, image))) => {
				if !model::valid_banner_uri(&uri)
					|| image.resolution()[0] > 1024
					|| image.resolution()[1] > 1024
					|| image.resolution()[0] < 680
					|| image.resolution()[1] < 240
				{
					return Err("The banner must be at least 680 × 240 pixels");
				}
				let Some(draft) = self.draft.as_mut() else {
					return Ok(());
				};
				draft.banner = Some(Some(uri));
				self.pending_banner = Some(ctx.load_texture(
					"profile-banner-preview",
					image,
					egui::TextureOptions::LINEAR,
				));
				self.saved = false;
				Ok(())
			}
			Ok(None) => Ok(()),
			Err(error) => Err(error),
		}
	}

	fn picture_choice(&self) -> Option<model::ProfilePicture> {
		if self.choosing {
			Some(model::ProfilePicture::Avatar)
		} else if self.choosing_banner {
			Some(model::ProfilePicture::Banner)
		} else {
			None
		}
	}

	pub fn show(
		&mut self,
		ui: &mut egui::Ui,
		state: &mut State,
		avatars: &mut Avatars,
		commands: &mut Vec<Command>,
	) {
		if self.generation != Some(state.generation) {
			*self = Self {
				generation: Some(state.generation),
				..Self::default()
			};
		}
		if state.own_profile.data.is_none()
			&& !state.own_profile.loading
			&& state.own_profile.error.is_none()
			&& let Some(command) = state.load_own_profile()
		{
			commands.push(command);
		}
		if self.submitted == Some(state.own_profile.request) && !state.own_profile.saving {
			self.submitted = None;
			if state.own_profile.error.is_none()
				&& let Some(profile) = &state.own_profile.data
			{
				let uploaded = self
					.draft
					.as_ref()
					.is_some_and(|draft| matches!(draft.avatar, Some(Some(_))));
				self.saved_picture = match (uploaded, self.pending_picture.take()) {
					(true, Some(texture)) => Some((profile.user.avatar.clone(), texture)),
					_ => None,
				};
				let uploaded_banner = self
					.draft
					.as_ref()
					.is_some_and(|draft| matches!(draft.banner, Some(Some(_))));
				self.saved_banner = match (uploaded_banner, self.pending_banner.take()) {
					(true, Some(texture)) => Some((profile.banner.clone(), texture)),
					_ => None,
				};
				self.draft = Some(Draft::from_profile(profile));
				self.saved = true;
			}
		}
		let colors = design::palette(ui);
		if state.own_profile.loading {
			ui.horizontal(|ui| {
				ui.add(egui::Spinner::new().size(14.0));
				ui.label(
					egui::RichText::new(crate::i18n::translate(
						"profile-edit-show-loading-your-profile",
					))
					.size(13.0)
					.color(colors.muted),
				);
			});
		}
		if let Some(error) = state.own_profile.error {
			design::notice(ui, design::Level::Error, error);
			if !state.own_profile.loading
				&& !state.own_profile.saving
				&& design::text_action(
					ui,
					&crate::i18n::translate("profile-edit-show-reload-profile"),
				)
				.clicked() && let Some(command) = state.load_own_profile()
			{
				commands.push(command);
			}
		}
		let Some(profile) = state.own_profile.data.as_ref() else {
			return;
		};
		let fresh = Draft::from_profile(profile);
		if let (Some(draft), Some(before)) = (&mut self.draft, &self.baseline) {
			draft.rebase(before, &fresh);
		}
		self.baseline = Some(fresh);
		let picture_choice = self.picture_choice();
		let draft = self
			.draft
			.get_or_insert_with(|| Draft::from_profile(profile));
		let before = draft.clone();
		let editable = !state.own_profile.loading && !state.own_profile.saving;
		let picture = match &draft.avatar {
			Some(Some(_)) => self
				.pending_picture
				.as_ref()
				.map_or(Picture::Remote, Picture::Pending),
			Some(None) => Picture::Removed,
			None => match &self.saved_picture {
				Some((hash, texture)) if hash.is_some() && *hash == profile.user.avatar => {
					Picture::Saved(texture)
				}
				_ => Picture::Remote,
			},
		};
		let banner_picture = match &draft.banner {
			Some(Some(_)) => self
				.pending_banner
				.as_ref()
				.map_or(BannerPreview::Remote, BannerPreview::Pending),
			Some(None) => BannerPreview::Removed,
			None => match &self.saved_banner {
				Some((hash, texture)) if hash.is_some() && *hash == profile.banner => {
					BannerPreview::Saved(texture)
				}
				_ => BannerPreview::Remote,
			},
		};
		let mut action = None;
		let banner_entitled = matches!(state.premium_type, 1 | 2);
		let full_nitro = state.premium_type == 2;
		let width = ui.available_width();
		if width >= 620.0 {
			ui.horizontal_top(|ui| {
				ui.spacing_mut().item_spacing.x = 24.0;
				ui.allocate_ui_with_layout(
					egui::vec2(width - 324.0, 0.0),
					egui::Layout::top_down(egui::Align::Min),
					|ui| {
						ui.add_enabled_ui(editable, |ui| {
							action = form(
								ui,
								draft,
								profile,
								picture_choice,
								banner_entitled,
								state.own_profile.collectibles.as_ref(),
								full_nitro,
							);
						});
					},
				);
				ui.vertical(|ui| {
					ui.set_width(300.0);
					if let Some(picked) = preview(
						ui,
						draft,
						profile,
						state.own_profile.collectibles.as_ref(),
						avatars,
						(state.demo, &state.guilds),
						&mut self.preview_link,
						picture,
						banner_picture,
						editable && !(self.choosing || self.choosing_banner),
						editable && banner_entitled && !(self.choosing || self.choosing_banner),
						banner_entitled,
					) {
						action = Some(picked);
					}
				});
			});
		} else {
			ui.add_enabled_ui(editable, |ui| {
				action = form(
					ui,
					draft,
					profile,
					picture_choice,
					banner_entitled,
					state.own_profile.collectibles.as_ref(),
					full_nitro,
				);
			});
		}
		let mut reload_collectibles = false;
		match action {
			Some(AvatarAction::Pick) if editable && !(self.choosing || self.choosing_banner) => {
				self.revision = self.revision.wrapping_add(1);
				self.choosing = true;
				self.avatar_request = Some((state.generation, profile.user.id, self.revision));
			}
			Some(AvatarAction::Remove) => {
				draft.avatar = Some(None);
				self.pending_picture = None;
			}
			Some(AvatarAction::Undo) => {
				draft.avatar = None;
				self.pending_picture = None;
			}
			Some(AvatarAction::BannerPick)
				if editable && banner_entitled && !(self.choosing || self.choosing_banner) =>
			{
				self.revision = self.revision.wrapping_add(1);
				self.choosing_banner = true;
				self.banner_request = Some((state.generation, profile.user.id, self.revision));
			}
			Some(AvatarAction::BannerRemove) => {
				draft.banner = Some(None);
				self.pending_banner = None;
			}
			Some(AvatarAction::BannerUndo) => {
				draft.banner = None;
				self.pending_banner = None;
			}
			Some(AvatarAction::ReloadCollectibles) => reload_collectibles = true,
			_ => {}
		}
		if *draft != before {
			self.saved = false;
		}
		if width < 620.0
			&& let (Some(draft), Some(profile)) = (&self.draft, &state.own_profile.data)
		{
			ui.add_space(16.0);
			let picture = match &draft.avatar {
				Some(Some(_)) => self
					.pending_picture
					.as_ref()
					.map_or(Picture::Remote, Picture::Pending),
				Some(None) => Picture::Removed,
				None => Picture::Remote,
			};
			preview(
				ui,
				draft,
				profile,
				state.own_profile.collectibles.as_ref(),
				avatars,
				(state.demo, &state.guilds),
				&mut self.preview_link,
				picture,
				match &draft.banner {
					Some(Some(_)) => self
						.pending_banner
						.as_ref()
						.map_or(BannerPreview::Remote, BannerPreview::Pending),
					Some(None) => BannerPreview::Removed,
					None => BannerPreview::Remote,
				},
				false,
				false,
				banner_entitled,
			);
		}
		if reload_collectibles
			&& editable
			&& let Some(command) = state.load_own_profile()
		{
			commands.push(command);
		}
		crate::markdown::confirm_external_link(ui.ctx(), &mut self.preview_link, true);
		if !state.demo && !state.gateway_connected {
			design::hint(
				ui,
				&crate::i18n::translate("profile-edit-show-reconnect-to-save-your-profile"),
			);
		}
	}
}

fn form(
	ui: &mut egui::Ui,
	draft: &mut Draft,
	profile: &UserProfile,
	choosing: Option<model::ProfilePicture>,
	banner_entitled: bool,
	collectibles: Option<&model::OwnedCollectibles>,
	full_nitro: bool,
) -> Option<AvatarAction> {
	ui.spacing_mut().item_spacing.y = 6.0;
	let mut action = None;
	let has_picture = match &draft.avatar {
		Some(value) => value.is_some(),
		None => profile.user.avatar.is_some(),
	};
	design::row(
		ui,
		"profile-edit-form-profile-picture",
		Some(match &draft.avatar {
			Some(Some(_)) => "profile-edit-form-new-picture-chosen-save-to-upload-it",
			Some(None) => "profile-edit-form-your-picture-will-be-removed-when-you-save",
			None => "profile-edit-form-png-jpeg-gif-or-webp-up-to-8-mb-cropped",
		}),
		|ui| {
			ui.add_enabled_ui(choosing.is_none(), |ui| {
				if design::button(
					ui,
					&crate::i18n::translate("profile-edit-form-change"),
					design::ButtonKind::Outline,
				)
				.clicked()
				{
					action = Some(AvatarAction::Pick);
				}
			});
			if draft.avatar.is_some() {
				if design::text_action(ui, &crate::i18n::translate("profile-edit-form-undo"))
					.clicked()
				{
					action = Some(AvatarAction::Undo);
				}
			} else if has_picture
				&& design::text_action(ui, &crate::i18n::translate("profile-edit-form-remove"))
					.clicked()
			{
				action = Some(AvatarAction::Remove);
			}
			if matches!(choosing, Some(model::ProfilePicture::Avatar)) {
				ui.add(egui::Spinner::new().size(14.0));
			}
		},
	);
	ui.add_space(10.0);
	let has_banner = match &draft.banner {
		Some(value) => value.is_some(),
		None => profile.banner.is_some(),
	};
	design::row(
		ui,
		"profile-edit-form-profile-banner",
		Some(if banner_entitled {
			"profile-edit-form-banner-upload-hint"
		} else {
			"profile-edit-form-banner-requires-nitro"
		}),
		|ui| {
			ui.add_enabled_ui(banner_entitled && choosing.is_none(), |ui| {
				if design::button(
					ui,
					&crate::i18n::translate("profile-edit-form-change"),
					design::ButtonKind::Outline,
				)
				.clicked()
				{
					action = Some(AvatarAction::BannerPick);
				}
			});
			if draft.banner.is_some() {
				if design::text_action(ui, &crate::i18n::translate("profile-edit-form-undo"))
					.clicked()
				{
					action = Some(AvatarAction::BannerUndo);
				}
			} else if has_banner
				&& design::text_action(ui, &crate::i18n::translate("profile-edit-form-remove"))
					.clicked()
			{
				action = Some(AvatarAction::BannerRemove);
			}
			if matches!(choosing, Some(model::ProfilePicture::Banner)) {
				ui.add(egui::Spinner::new().size(14.0));
			}
		},
	);
	field(
		ui,
		"profile-edit-form-display-name",
		"profile-display-name",
		&mut draft.name,
		model::MAX_PROFILE_NAME_CHARS,
		false,
	);
	field(
		ui,
		"profile-edit-form-pronouns",
		"profile-pronouns",
		&mut draft.pronouns,
		model::MAX_PROFILE_PRONOUNS_CHARS,
		false,
	);
	ui.add_space(8.0);
	field(
		ui,
		"profile-edit-form-about-me",
		"profile-about-me",
		&mut draft.bio,
		model::MAX_PROFILE_BIO_CHARS,
		true,
	);
	ui.add_space(8.0);
	let mut enabled = draft.color.is_some();
	design::row(
		ui,
		"profile-edit-form-profile-color",
		Some("profile-edit-form-tints-your-banner-when-you-have-not-set-a-banner"),
		|ui| {
			if let Some(color) = &mut draft.color {
				let mut rgb = [(*color >> 16) as u8, (*color >> 8) as u8, *color as u8];
				if design::color_edit(ui, &mut rgb)
					.on_hover_text(crate::i18n::translate(
						"profile-edit-form-choose-profile-color",
					))
					.changed()
				{
					*color =
						(u32::from(rgb[0]) << 16) | (u32::from(rgb[1]) << 8) | u32::from(rgb[2]);
				}
				if design::text_action(ui, &crate::i18n::translate("profile-edit-form-use-default"))
					.clicked()
				{
					enabled = false;
				}
			} else if design::text_action(
				ui,
				&crate::i18n::translate("profile-edit-form-custom-color"),
			)
			.clicked()
			{
				enabled = true;
			}
		},
	);
	if enabled != draft.color.is_some() {
		// Start from the profile's own colors rather than the app accent.
		draft.color = enabled.then(|| {
			profile
				.accent_color
				.or(profile.theme_colors.map(|[top, _]| top))
				.unwrap_or(design::DEFAULT_PRIMARY_RGB)
		});
	}
	ui.add_space(12.0);
	collectible_groups(ui, draft, collectibles, full_nitro, &mut action);
	action
}

fn collectible_groups(
	ui: &mut egui::Ui,
	draft: &mut Draft,
	collectibles: Option<&model::OwnedCollectibles>,
	full_nitro: bool,
	action: &mut Option<AvatarAction>,
) {
	let Some(owned) = collectibles else {
		design::row(
			ui,
			"profile-edit-form-owned-cosmetics",
			Some("profile-edit-form-owned-cosmetics-unavailable"),
			|ui| {
				if design::text_action(
					ui,
					&crate::i18n::translate("profile-edit-form-reload-owned-cosmetics"),
				)
				.clicked()
				{
					*action = Some(AvatarAction::ReloadCollectibles);
				}
			},
		);
		return;
	};
	let decorations: Vec<_> = owned
		.decorations
		.iter()
		.map(|option| (option.item.sku, option.name.as_str(), option.requires_nitro))
		.collect();
	let nameplates: Vec<_> = owned
		.nameplates
		.iter()
		.map(|option| (option.item.sku, option.name.as_str(), option.requires_nitro))
		.collect();
	let effects: Vec<_> = owned
		.effects
		.iter()
		.map(|option| (option.item.sku, option.name.as_str(), option.requires_nitro))
		.collect();
	cosmetic_row(
		ui,
		"profile-edit-form-avatar-decoration",
		"profile-edit-form-owned-avatar-decoration",
		"profile-edit-form-no-owned-avatar-decorations",
		&mut draft.avatar_decoration,
		&decorations,
		full_nitro,
	);
	cosmetic_row(
		ui,
		"profile-edit-form-nameplate",
		"profile-edit-form-owned-nameplate",
		"profile-edit-form-no-owned-nameplates",
		&mut draft.nameplate,
		&nameplates,
		full_nitro,
	);
	cosmetic_row(
		ui,
		"profile-edit-form-profile-effect",
		"profile-edit-form-owned-profile-effect",
		"profile-edit-form-no-owned-profile-effects",
		&mut draft.profile_effect,
		&effects,
		full_nitro,
	);
	if owned.limited {
		design::hint(
			ui,
			&crate::i18n::translate("profile-edit-form-owned-cosmetics-partial"),
		);
	}
	// Display remains available, but no supported mutation field was found for frames.
	design::row(
		ui,
		"profile-edit-form-profile-frame",
		Some("profile-edit-form-profile-frame-unavailable"),
		|ui| {
			ui.add_enabled_ui(false, |ui| {
				design::button(
					ui,
					&crate::i18n::translate("profile-edit-form-change"),
					design::ButtonKind::Outline,
				);
			});
		},
	);
}

#[allow(clippy::too_many_arguments)]
fn cosmetic_row(
	ui: &mut egui::Ui,
	label: &'static str,
	hint: &'static str,
	empty: &'static str,
	selected: &mut Option<Id>,
	options: &[(Id, &str, bool)],
	full_nitro: bool,
) {
	let unknown = selected.is_some_and(|sku| !options.iter().any(|option| option.0 == sku));
	design::row(ui, label, Some(hint), |ui| {
		if options.is_empty() && selected.is_none() {
			ui.label(crate::i18n::translate(empty));
			return;
		}
		let selected_name = selected
			.and_then(|sku| {
				options
					.iter()
					.find(|option| option.0 == sku)
					.map(|option| option.1.to_owned())
			})
			.or_else(|| selected.map(|sku| format!("SKU {sku}")))
			.unwrap_or_else(|| crate::i18n::translate("profile-edit-form-none"));
		egui::ComboBox::from_id_salt(label)
			.selected_text(selected_name)
			.width(210.0)
			.show_ui(ui, |ui| {
				ui.selectable_value(
					selected,
					None,
					crate::i18n::translate("profile-edit-form-none"),
				);
				for (sku, name, requires_nitro) in options {
					ui.add_enabled_ui(!requires_nitro || full_nitro, |ui| {
						let name = if *requires_nitro {
							format!(
								"{} · {}",
								name,
								crate::i18n::translate("profile-edit-form-requires-full-nitro")
							)
						} else {
							(*name).to_owned()
						};
						ui.selectable_value(selected, Some(*sku), name);
					});
				}
			});
	});
	if unknown {
		design::hint(
			ui,
			&crate::i18n::translate("profile-edit-form-equipped-cosmetic-not-owned"),
		);
	}
}

fn field(
	ui: &mut egui::Ui,
	label: &str,
	id: &str,
	value: &mut String,
	limit: usize,
	multiline: bool,
) {
	let colors = design::palette(ui);
	let label = crate::i18n::translate_if_key(label);
	let label_id = ui
		.horizontal(|ui| {
			let label = ui.label(design::eyebrow(ui, &label, colors.muted));
			if multiline {
				ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
					ui.label(
						egui::RichText::new(format!("{} / {limit}", value.chars().count()))
							.size(11.0)
							.color(colors.muted),
					);
				});
			}
			label.id
		})
		.inner;
	let edit = if multiline {
		egui::TextEdit::multiline(value).desired_rows(3)
	} else {
		egui::TextEdit::singleline(value).align(egui::Align2::LEFT_CENTER)
	};
	let edit = if id == "profile-display-name" {
		edit.hint_text(crate::i18n::translate(
			"profile-edit-form-leave-blank-to-use-your-username",
		))
	} else {
		edit
	};
	design::input(
		ui,
		edit.id(egui::Id::unique(id))
			.char_limit(limit)
			.font(egui::FontId::proportional(15.0))
			.text_color(colors.text),
	)
	.labelled_by(label_id);
}

impl Editor {
	/// Shares the visible error state with the settings page's footer reservation.
	pub(super) fn invalid_draft(&self, state: &State) -> bool {
		self.draft
			.as_ref()
			.zip(state.own_profile.data.as_ref())
			.is_some_and(|(draft, profile)| {
				let changes = draft.changes(profile);
				changes != ProfileEdit::default() && !changes.valid()
			})
	}

	/// The actions stay below the scrollable profile fields, including when new editors are added.
	pub(super) fn save_controls(
		&mut self,
		ui: &mut egui::Ui,
		state: &mut State,
		commands: &mut Vec<Command>,
	) {
		let Some(profile) = state.own_profile.data.as_ref() else {
			return;
		};
		let Some(draft) = self.draft.as_ref() else {
			return;
		};
		let changes = draft.changes(profile);
		let changed = changes != ProfileEdit::default();
		let editable = !state.own_profile.loading && !state.own_profile.saving;
		let colors = design::palette(ui);
		if !changed && !changes.valid() {
			return;
		}
		if !changes.valid() {
			design::notice(
				ui,
				design::Level::Error,
				&crate::i18n::translate(
					"profile-edit-show-check-character-limits-and-remove-control-characters-a-display-name",
				),
			);
		}
		egui::Frame::new()
			.fill(colors.base)
			.corner_radius(8)
			.inner_margin(egui::Margin::symmetric(12, 8))
			.show(ui, |ui| {
				ui.set_width(ui.available_width());
				ui.horizontal(|ui| {
					ui.spacing_mut().item_spacing.x = 12.0;
					if state.own_profile.saving {
						ui.add(egui::Spinner::new().size(14.0));
						ui.label(
							egui::RichText::new(crate::i18n::translate(
								"profile-edit-show-saving-profile",
							))
							.size(13.0)
							.color(colors.muted),
						);
					} else if self.saved {
						ui.label(
							egui::RichText::new(crate::i18n::translate_if_key(if state.demo {
								"profile-edit-show-saved-in-preview"
							} else {
								"profile-edit-show-profile-saved"
							}))
							.size(13.0)
							.color(colors.positive),
						);
					} else if changed {
						ui.label(
							egui::RichText::new(crate::i18n::translate(
								"profile-edit-show-you-have-unsaved-changes",
							))
							.size(13.0)
							.color(colors.text),
						);
					}
					ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
						ui.add_enabled_ui(
							changed && changes.valid() && state.can_save_own_profile(),
							|ui| {
								if design::button(
									ui,
									&crate::i18n::translate("profile-edit-show-save-changes"),
									design::ButtonKind::Primary,
								)
								.clicked() && let Some(command) =
									state.save_own_profile(changes.clone())
								{
									self.submitted = Some(state.own_profile.request);
									self.saved = false;
									commands.push(command);
								}
							},
						);
						ui.add_enabled_ui(changed && editable, |ui| {
							if design::button(
								ui,
								&crate::i18n::translate("profile-edit-show-cancel"),
								design::ButtonKind::Neutral,
							)
							.clicked()
							{
								self.draft =
									state.own_profile.data.as_ref().map(Draft::from_profile);
								self.pending_picture = None;
								self.pending_banner = None;
								self.saved = false;
							}
						});
					});
				});
			});
	}
}

/// A preview target focuses its existing editor, without opening an unsaved parallel draft.
fn focus_field(
	ui: &mut egui::Ui,
	response: egui::Response,
	field: &'static str,
	label: &'static str,
) {
	let response = ui
		.interact(response.rect, response.id, egui::Sense::click())
		.on_hover_cursor(egui::CursorIcon::Text)
		.on_hover_text(crate::i18n::translate(label));
	if response.clicked() {
		ui.memory_mut(|memory| memory.request_focus(egui::Id::unique(field)));
	}
}

#[allow(clippy::too_many_arguments)]
fn preview(
	ui: &mut egui::Ui,
	draft: &Draft,
	profile: &UserProfile,
	collectibles: Option<&model::OwnedCollectibles>,
	avatars: &mut Avatars,
	media: (bool, &[model::Guild]),
	opening: &mut Option<String>,
	picture: Picture<'_>,
	banner_preview: BannerPreview<'_>,
	clickable: bool,
	banner_clickable: bool,
	banner_entitled: bool,
) -> Option<AvatarAction> {
	let (demo, guilds) = media;
	let colors = design::palette(ui);
	let mut preview_profile = profile.clone();
	if let Some(owned) = collectibles {
		let current = profile
			.user
			.cosmetics
			.as_deref()
			.and_then(|cosmetics| cosmetics.decoration.as_ref())
			.cloned();
		let current_sku = current.as_ref().map(|item| item.sku);
		let cosmetics = preview_profile.user.cosmetics.get_or_insert_default();
		cosmetics.decoration = match draft.avatar_decoration {
			Some(sku) => owned.decoration(sku).map_or_else(
				|| (Some(sku) == current_sku).then_some(current.clone()).flatten(),
				|option| Some(option.item.clone()),
			),
			None => None,
		};
		let current = profile
			.user
			.cosmetics
			.as_deref()
			.and_then(|cosmetics| cosmetics.nameplate.as_ref())
			.cloned();
		let current_sku = current.as_ref().map(|item| item.sku);
		cosmetics.nameplate = match draft.nameplate {
			Some(sku) => owned.nameplate(sku).map_or_else(
				|| (Some(sku) == current_sku).then_some(current.clone()).flatten(),
				|option| Some(option.item.clone()),
			),
			None => None,
		};
		preview_profile.effect_sku = draft.profile_effect;
		preview_profile.effect = match draft.profile_effect {
			Some(sku) => owned.effect(sku).map_or_else(
				|| {
					(Some(sku) == profile.effect_sku)
						.then(|| profile.effect.clone())
						.flatten()
				},
				|option| Some(Box::new(option.item.clone())),
			),
			None => None,
		};
	}
	let mut action = None;
	ui.label(design::eyebrow(
		ui,
		crate::i18n::translate("profile-edit-preview-preview"),
		colors.muted,
	));
	ui.add_space(4.0);
	// Paint with the profile's own colors, as other people see the card, not the app theme.
	let theme = crate::profiles::Theme::new(&colors, profile.theme_colors);
	let rgb = |c: u32| egui::Color32::from_rgb((c >> 16) as u8, (c >> 8) as u8, c as u8);
	ui.scope(|ui| {
		let background = ui.painter().add(egui::Shape::Noop);
		let frame_back = ui.painter().add(egui::Shape::Noop);
		ui.visuals_mut().override_text_color = Some(theme.text);
		ui.visuals_mut().hyperlink_color = theme.link;
		ui.set_width(ui.available_width());
		ui.spacing_mut().item_spacing.y = 0.0;
		{
			let (banner, _) = ui
				.allocate_exact_size(egui::vec2(ui.available_width(), 90.0), egui::Sense::hover());
			let corner = egui::CornerRadius {
				nw: 12,
				ne: 12,
				sw: 0,
				se: 0,
			};
			match banner_preview {
				BannerPreview::Remote if profile.banner.is_some() => {
					avatars.paint_banner(ui, profile, banner, corner, demo);
				}
				BannerPreview::Pending(texture) | BannerPreview::Saved(texture) => {
					let source = texture.resolution_vec2();
					let scale = (banner.width() / source.x).max(banner.height() / source.y);
					let uv = egui::Rect::from_center_size(
						egui::pos2(0.5, 0.5),
						banner.size() / (source * scale),
					);
					egui::Image::new(texture)
						.uv(uv)
						.corner_radius(corner)
						.paint_at(ui, banner);
				}
				_ => {
					let color = profile
						.theme_colors
						.map(crate::profiles::Theme::banner)
						.or_else(|| draft.color.map(rgb))
						.unwrap_or(colors.raised);
					ui.painter().rect_filled(banner, corner, color);
				}
			}
			let banner_hit = ui.interact(
				banner,
				ui.scope_id().with("change-banner"),
				egui::Sense::click(),
			);
			if banner_clickable {
				let banner_hit = banner_hit
					.on_hover_text(crate::i18n::translate("profile-edit-preview-change-banner"));
				if banner_hit.clicked() {
					action = Some(AvatarAction::BannerPick);
				}
			} else {
				banner_hit.on_hover_text(crate::i18n::translate(if banner_entitled {
					"profile-edit-preview-use-banner-editor"
				} else {
					"profile-edit-form-banner-requires-nitro"
				}));
			}
			let avatar = egui::Rect::from_min_size(
				banner.left_bottom() + egui::vec2(16.0, -40.0),
				egui::Vec2::splat(80.0),
			);
			ui.painter()
				.circle_filled(avatar.center(), 46.0, theme.card);
			let remote_picture = matches!(picture, Picture::Remote);
			let response =
				ui.scope_builder(
					egui::UiBuilder::new().max_rect(avatar),
					|ui| match picture {
						Picture::Pending(texture) | Picture::Saved(texture) => ui.add(
							egui::Image::new(texture)
								.fit_to_exact_size(egui::Vec2::splat(80.0))
								.corner_radius(40.0),
						),
						Picture::Removed => design::avatar(ui, &preview_profile.user.name, 80.0),
						Picture::Remote => avatars.with_avatar_animation(true, |avatars| {
							avatars.show_plain(ui, &preview_profile.user, 80.0, demo)
						}),
					},
				);
			if !remote_picture {
				avatars.paint_decoration(ui, &preview_profile.user, 80.0, avatar, demo);
			}
			if clickable {
				let hit = ui.interact(
					avatar,
					ui.scope_id().with("change-avatar"),
					egui::Sense::click(),
				);
				let hit = hit.on_hover_text(crate::i18n::translate(
					"profile-edit-preview-change-profile-picture",
				));
				if hit.hovered() || hit.has_focus() {
					ui.painter().circle_filled(
						avatar.center(),
						40.0,
						egui::Color32::from_black_alpha(120),
					);
					icons::paint(
						ui.painter(),
						icons::Icon::Pencil,
						egui::Rect::from_center_size(avatar.center(), egui::Vec2::splat(22.0)),
						egui::Color32::WHITE,
					);
				}
				if hit.clicked() {
					action = Some(AvatarAction::Pick);
				}
			}
			drop(response);
			ui.add_space((avatar.bottom() + 12.0 - ui.cursor().top()).max(0.0));
			egui::Frame::new()
				.inner_margin(egui::Margin {
					left: 12,
					right: 12,
					top: 0,
					bottom: 12,
				})
				.show(ui, |ui| {
					egui::Frame::new()
						.fill(theme.panel)
						.corner_radius(8)
						.inner_margin(12)
						.show(ui, |ui| {
							ui.set_width(ui.available_width());
							ui.set_min_height(138.0);
							ui.spacing_mut().item_spacing.y = 4.0;
							let name = if draft.name.is_empty() {
								&profile.username
							} else {
								&draft.name
							};
							let (name_row, _) = ui.allocate_exact_size(
								egui::vec2(ui.available_width(), 30.0),
								egui::Sense::hover(),
							);
							avatars.paint_nameplate(
								ui,
								&preview_profile.user,
								name_row,
								6,
								false,
								false,
								demo,
							);
							let name = ui
								.scope_builder(egui::UiBuilder::new().max_rect(name_row), |ui| {
									ui.add(
										egui::Label::new(
											design::semibold(ui, name, 20.0).color(theme.text),
										)
										.truncate(),
									)
								})
								.inner;
							focus_field(
								ui,
								name,
								"profile-display-name",
								"profile-edit-preview-edit-display-name",
							);
							ui.label(
								egui::RichText::new(&profile.username)
									.size(13.0)
									.color(theme.text),
							);
							if !draft.pronouns.is_empty() {
								let pronouns = ui.label(
									egui::RichText::new(&draft.pronouns)
										.size(12.0)
										.color(theme.muted),
								);
								focus_field(
									ui,
									pronouns,
									"profile-pronouns",
									"profile-edit-preview-edit-pronouns",
								);
							}
							if !draft.bio.is_empty() {
								ui.add_space(8.0);
								ui.separator();
								ui.add_space(8.0);
								let about = ui.label(design::eyebrow(
									ui,
									crate::i18n::translate("profile-edit-preview-about-me"),
									theme.text,
								));
								focus_field(
									ui,
									about,
									"profile-about-me",
									"profile-edit-preview-edit-about-me",
								);
								let mut mentions = crate::profiles::ProfileSession::default();
								crate::markdown::Formatted::parse(&draft.bio).show_with_images(
									ui,
									opening,
									&[],
									None,
									&mut mentions,
									(avatars, demo, guilds),
									crate::design::MessageCardSurface::Opaque,
								);
							}
						});
				});
		}
		let rect = ui.min_rect();
		ui.painter().set(background, theme.background(rect));
		// The editor lives inside the Settings scroll area, not a floating profile pop-out.
		// Reuse the docked clipping path so frame overflow cannot paint adjacent settings.
		let settings_clip = ui.clip_rect();
		crate::profiles::paint_collectibles(
			ui,
			avatars,
			&preview_profile,
			rect,
			frame_back,
			(false, Some(settings_clip)),
			demo,
		);
	});
	action
}

#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn draft_only_sends_changed_fields_and_can_clear_values() {
		let user = model::User {
			id: model::Id(1),
			name: "Synthetic".into(),
			avatar: None,
			webhook: false,
			kind: Default::default(),
			discriminator: 0,
			primary_guild: None,
			cosmetics: None,
		};
		let mut profile = crate::profiles::synthetic(&user, None);
		profile.global_name = Some("Before".into());
		profile.accent_color = Some(0x123456);
		let mut draft = Draft::from_profile(&profile);
		assert!(draft.changes(&profile) == ProfileEdit::default());
		draft.name.clear();
		draft.bio.clear();
		draft.color = None;
		draft.avatar_decoration = Some(Id(9101));
		draft.nameplate = Some(Id(9102));
		draft.profile_effect = None;
		let changes = draft.changes(&profile);
		assert_eq!(changes.global_name, Some(None));
		assert_eq!(changes.bio, Some(String::new()));
		assert_eq!(changes.accent_color, Some(None));
		assert_eq!(changes.pronouns, None);
		assert_eq!(changes.avatar_decoration_sku_id, Some(Some(Id(9101))));
		assert_eq!(changes.nameplate_sku_id, Some(Some(Id(9102))));
		assert_eq!(changes.profile_effect_id, Some(None));
		assert!(changes.valid());
		// A reload retains edited values but adopts remote changes to untouched fields.
		let before = Draft::from_profile(&profile);
		profile.pronouns = "she/her".into();
		draft.rebase(&before, &Draft::from_profile(&profile));
		assert_eq!(draft.pronouns, "she/her");
		assert!(draft.name.is_empty() && draft.bio.is_empty());
		assert_eq!(draft.changes(&profile).pronouns, None);
	}
	#[test]
	fn editor_saves_once_preserves_failed_draft_and_cancels_in_both_themes() {
		fn frame(
			ctx: &egui::Context,
			editor: &mut Editor,
			state: &mut State,
			avatars: &mut Avatars,
			width: f32,
			events: Vec<egui::Event>,
		) -> (Vec<Command>, Vec<(String, egui::Rect)>) {
			let mut commands = vec![];
			let mut output = ctx.run_ui(
				egui::RawInput {
					screen_rect: Some(egui::Rect::from_min_size(
						egui::Pos2::ZERO,
						egui::vec2(width, 1800.0),
					)),
					events,
					..Default::default()
				},
				|ui| {
					editor.show(ui, state, avatars, &mut commands);
					editor.save_controls(ui, state, &mut commands);
				},
			);
			output.textures_delta.clear();
			fn text(shape: &egui::Shape, labels: &mut Vec<(String, egui::Rect)>) {
				match shape {
					egui::Shape::Text(text) => labels.push((
						text.galley.job.text.clone(),
						text.galley.rect.translate(text.pos.to_vec2()),
					)),
					egui::Shape::Vec(shapes) => {
						for shape in shapes {
							text(shape, labels);
						}
					}
					_ => {}
				}
			}
			let mut labels = vec![];
			for shape in output.shapes {
				text(&shape.shape, &mut labels);
			}
			(commands, labels)
		}
		for theme in [egui::ThemePreference::Dark, egui::ThemePreference::Light] {
			for width in [320.0, 720.0] {
				let ctx = egui::Context::default();
				ctx.set_theme(theme);
				let mut editor = Editor::default();
				let mut avatars = Avatars::default();
				let mut state = test_support::demo_state();
				let (commands, _) =
					frame(&ctx, &mut editor, &mut state, &mut avatars, width, vec![]);
				assert_eq!(commands.len(), 1);
				let Command::EditProfile {
					user,
					request,
					changes: None,
				} = commands.into_iter().next().unwrap()
				else {
					panic!("load expected")
				};
				state.apply(client_core::Envelope {
					generation: state.generation,
					event: client_core::Event::ProfileEdited {
						user,
						request,
						result: Ok(Box::new(model::OwnProfileData {
							profile: crate::profiles::synthetic(state.user.as_ref().unwrap(), None),
							collectibles: Some(crate::synthetic_owned_collectibles()),
						})),
					},
				});
				frame(&ctx, &mut editor, &mut state, &mut avatars, width, vec![]);
				ctx.memory_mut(|memory| {
					memory.request_focus(egui::Id::unique("profile-display-name"))
				});
				frame(
					&ctx,
					&mut editor,
					&mut state,
					&mut avatars,
					width,
					vec![egui::Event::Text(" Edited".into())],
				);
				let draft = editor.draft.as_ref().unwrap().clone();
				assert!(draft.name.contains("Edited"));
				let (_, labels) = frame(&ctx, &mut editor, &mut state, &mut avatars, width, vec![]);
				let save = labels
					.iter()
					.find(|(text, _)| text == "Save changes")
					.unwrap()
					.1;
				assert!(save.left() >= 0.0 && save.right() <= width);
				let mut sent = vec![];
				for pressed in [true, false] {
					let pos = save.center();
					let (commands, _) = frame(
						&ctx,
						&mut editor,
						&mut state,
						&mut avatars,
						width,
						vec![
							egui::Event::PointerMoved(pos),
							egui::Event::PointerButton {
								pos,
								button: egui::PointerButton::Primary,
								pressed,
								modifiers: egui::Modifiers::NONE,
							},
						],
					);
					sent.extend(commands);
				}
				assert_eq!(sent.len(), 1);
				assert!(
					frame(&ctx, &mut editor, &mut state, &mut avatars, width, vec![])
						.0
						.is_empty()
				);
				let Command::EditProfile {
					user,
					request,
					changes: Some(changes),
				} = sent.pop().unwrap()
				else {
					panic!("save expected")
				};
				assert_eq!(changes.global_name, Some(Some(draft.name.clone())));
				state.apply(client_core::Envelope {
					generation: state.generation,
					event: client_core::Event::ProfileEdited {
						user,
						request,
						result: Err(client_core::auth::Failure::Ambiguous),
					},
				});
				let (_, labels) = frame(&ctx, &mut editor, &mut state, &mut avatars, width, vec![]);
				assert!(editor.draft.as_ref() == Some(&draft));
				assert!(!state.can_save_own_profile());
				let cancel = labels
					.iter()
					.find(|(text, _)| text == "Cancel")
					.unwrap()
					.1
					.center();
				for pressed in [true, false] {
					assert!(
						frame(
							&ctx,
							&mut editor,
							&mut state,
							&mut avatars,
							width,
							vec![
								egui::Event::PointerMoved(cancel),
								egui::Event::PointerButton {
									pos: cancel,
									button: egui::PointerButton::Primary,
									pressed,
									modifiers: egui::Modifiers::NONE
								}
							]
						)
						.0
						.is_empty()
					);
				}
				assert!(
					editor.draft.as_ref()
						== Some(&Draft::from_profile(
							state.own_profile.data.as_ref().unwrap()
						))
				);
			}
		}
	}
}
