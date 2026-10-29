//! Published-release selector. A choice changes the release used by future checks, not a running install.
use artcraft_launcher_core::catalog::{CraftApp, LAUNCHER, LAUNCHER_ID};
use artcraft_launcher_core::protocol::VersionTrack;
use artcraft_launcher_core::release::{Release, is_nightly};
use egui::{RichText, Ui};

use crate::app::LauncherUi;
use crate::theme::Tokens;
use crate::widgets::{self, Kind};

impl LauncherUi {
    /// The per-app selector stays in its dialog. The launcher's selector is drawn in Settings.
    pub(crate) fn version_dialog(&mut self, ui: &mut Ui, id: &str) -> bool {
        let Some(app) = CraftApp::by_id(id) else { return true };
        let t = Tokens::get(ui.ctx());
        ui.set_width(480.0f32.min((ui.ctx().content_rect().width() - 80.0).max(240.0)));
        ui.heading(format!("{} · {}", app.name, tl!("Choose version")));
        ui.add_space(8.0);
        let label = ui.label(tl!("Version"));
        let selected = self.version_combo(ui, id, app, 440.0, label.id);
        ui.add_space(10.0);
        ui.label(
            RichText::new(tl!("After choosing a version, Install version appears in place of Open. Your files may not open in older versions."))
                .color(t.muted),
        );
        ui.label(RichText::new(tl!("Enable automatic installs in Settings to follow new nightly builds.")).color(t.muted));
        ui.add_space(12.0);
        selected || widgets::button(ui, tl!("Close"), Kind::Secondary, 100.0).clicked()
    }

    pub(crate) fn version_combo(&mut self, ui: &mut Ui, id: &str, app: &CraftApp, width: f32, label: egui::Id) -> bool {
        let t = Tokens::get(ui.ctx());
        let choice =
            if id == LAUNCHER_ID { self.settings.launcher_version.clone() } else { self.settings.versions.get(id).cloned().unwrap_or_default() };
        let list = self.version_lists.get(id).cloned();
        let mut picked = None;
        let summary = match &choice {
            VersionTrack::Stable => tl!("Latest release").to_string(),
            VersionTrack::Nightly => tl!("Nightly").to_string(),
            VersionTrack::Pinned(tag) => tag.clone(),
        };
        craft_ui::ComboBox::from_id_salt(("version", id))
            .selected_text(summary)
            .width(width)
            .height(390.0)
            .show_ui(ui, |ui| {
                // The combo already scrolls its entire menu. Keep every option, including
                // Nightly and its availability note, in that one scroll region.
                ui.spacing_mut().item_spacing.y = 10.0;
                let option_size = egui::vec2(ui.available_width(), 42.0);
                if ui.add(egui::Button::selectable(choice == VersionTrack::Stable, tl!("Latest release")).min_size(option_size)).clicked() {
                    picked = Some(VersionTrack::Stable);
                }
                let nightly = list
                    .as_ref()
                    .and_then(|r| r.as_ref().ok())
                    .is_some_and(|rs| rs.iter().any(|r| is_nightly(r) && self.has_version_download(app, r)));
                if ui.add_enabled(nightly, egui::Button::selectable(choice == VersionTrack::Nightly, tl!("Nightly")).min_size(option_size)).clicked()
                {
                    picked = Some(VersionTrack::Nightly);
                }
                if !nightly {
                    ui.label(RichText::new(tl!("No nightly build published yet")).color(t.muted));
                }
                ui.separator();
                match &list {
                    None => {
                        ui.spinner();
                        ui.label(tl!("Checking…"));
                    }
                    Some(Err(why)) => {
                        ui.label(RichText::new(why).color(t.bad));
                    }
                    Some(Ok(releases)) => {
                        if releases.is_empty() {
                            ui.label(tl!("No release published yet"));
                        }
                        for r in releases {
                            if !self.has_version_download(app, r) {
                                continue;
                            }
                            let tag = &r.tag;
                            if ui.add(egui::Button::selectable(choice == VersionTrack::Pinned(tag.clone()), tag).min_size(option_size)).clicked() {
                                picked = Some(VersionTrack::Pinned(tag.clone()));
                            }
                        }
                    }
                }
            })
            .response
            .labelled_by(label);
        if let Some(track) = picked {
            if id == LAUNCHER_ID {
                self.settings.launcher_version = track;
            } else if track == VersionTrack::Stable {
                self.settings.versions.remove(id);
            } else {
                self.settings.versions.insert(id.to_string(), track);
            }
            // Refresh the checked release before the new main action can install it.
            self.configure();
            self.check_now_from_ui();
            return true;
        }
        false
    }

    /// Launcher downgrades are manual. Only offer the published release selected above.
    pub(crate) fn launcher_version_notes(&self, ui: &mut Ui) {
        let t = Tokens::get(ui.ctx());
        if let VersionTrack::Pinned(tag) = &self.settings.launcher_version {
            if let Some(release) =
                self.version_lists.get(LAUNCHER_ID).and_then(|r| r.as_ref().ok()).and_then(|releases| releases.iter().find(|r| &r.tag == tag))
                && self.has_version_download(&LAUNCHER, release)
                && ui.link(tl!("Download selected version")).clicked()
            {
                let url = self
                    .platform
                    .and_then(|p| crate::views::launcher_download(release, p, crate::settings::is_flatpak_sandbox()))
                    .unwrap_or_else(|| release.html_url.clone());
                widgets::open_external_link(ui.ctx(), &url);
            }
            ui.label(RichText::new(tl!("Older launcher versions require a manual reinstall.")).color(t.muted));
        }
        if self.settings.launcher_version == VersionTrack::Nightly {
            ui.label(RichText::new(tl!("Enable automatic installs in Settings to follow new nightly builds.")).color(t.muted));
        }
    }

    fn has_version_download(&self, app: &CraftApp, r: &Release) -> bool {
        let Some(platform) = self.platform else { return false };
        let stems: Vec<&str> = app.stems().collect();
        artcraft_launcher_core::platform::pick_asset(r, &stems, platform, platform.download_kinds())
            .is_some_and(|(asset, _)| asset.sha256.is_some() || r.sums_asset().is_some())
    }
}
