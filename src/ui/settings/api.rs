// Speech to Text - Local API Server Settings Page
// Copyright (C) 2026 Christos A. Daggas
// SPDX-License-Identifier: MIT

//! Enable and configure the opt-in local HTTP API server (off by default). The
//! server binds 127.0.0.1 only; other local apps POST audio for transcription
//! and optional translation. A bearer token (stored in the system keyring,
//! never in the config) gates access and can be copied or regenerated here.

use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk4 as gtk;
use gtk4::glib;
use gtk4::prelude::*;
use libadwaita as adw;
use std::cell::RefCell;

use crate::application::tokio_runtime;
use crate::config::AppConfig;
use crate::i18n::gettext;

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct ApiPage {
        pub enable_switch: RefCell<Option<adw::SwitchRow>>,
        pub port_spin: RefCell<Option<adw::SpinRow>>,
        pub url_row: RefCell<Option<adw::ActionRow>>,
        pub token_switch: RefCell<Option<adw::SwitchRow>>,
        pub token_row: RefCell<Option<adw::ActionRow>>,
        pub token_label: RefCell<Option<gtk::Label>>,
        pub status_label: RefCell<Option<gtk::Label>>,
        pub current_token: RefCell<Option<String>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for ApiPage {
        const NAME: &'static str = "SttApiPage";
        type Type = super::ApiPage;
        type ParentType = adw::PreferencesPage;
    }

    impl ObjectImpl for ApiPage {
        fn constructed(&self) {
            self.parent_constructed();
            self.obj().setup_ui();
        }
    }

    impl WidgetImpl for ApiPage {}
    impl adw::subclass::prelude::PreferencesPageImpl for ApiPage {}
}

glib::wrapper! {
    pub struct ApiPage(ObjectSubclass<imp::ApiPage>)
        @extends gtk::Widget, adw::PreferencesPage;
}

impl Default for ApiPage {
    fn default() -> Self {
        Self::new()
    }
}

impl ApiPage {
    pub fn new() -> Self {
        glib::Object::builder()
            .property("title", "API")
            .property("icon-name", "network-server-symbolic")
            .build()
    }

    /// The running Application instance, for start/stop of the server.
    fn app() -> Option<crate::application::Application> {
        gtk::gio::Application::default()
            .and_then(|a| a.downcast::<crate::application::Application>().ok())
    }

    fn setup_ui(&self) {
        let imp = self.imp();
        let config = AppConfig::load();

        // ── Server ──────────────────────────────────────────────────
        let group = adw::PreferencesGroup::new();
        group.set_title(gettext("Local API Server").as_str());
        group.set_description(Some(&gettext(
            "Let other apps on this computer send audio for transcription and translation. The server listens on 127.0.0.1 only (never the network).",
        )));

        let enable_switch = adw::SwitchRow::builder()
            .title(gettext("Enable API server").as_str())
            .subtitle(gettext("Off by default. Starts/stops immediately when toggled.").as_str())
            .build();
        enable_switch.set_active(config.api_server_enabled);
        group.add(&enable_switch);

        let port_adj = gtk::Adjustment::new(
            config.api_server_port as f64,
            1024.0,
            65535.0,
            1.0,
            10.0,
            0.0,
        );
        let port_spin = adw::SpinRow::new(Some(&port_adj), 1.0, 0);
        port_spin.set_title(&gettext("Port"));
        group.add(&port_spin);

        let url_row = adw::ActionRow::builder()
            .title(gettext("Address").as_str())
            .subtitle(format!("http://127.0.0.1:{}", config.api_server_port))
            .build();
        url_row.add_css_class("property");
        group.add(&url_row);

        self.add(&group);

        // ── Authentication ──────────────────────────────────────────
        let auth_group = adw::PreferencesGroup::new();
        auth_group.set_title(gettext("Authentication").as_str());
        auth_group.set_description(Some(&gettext(
            "Requests must send an Authorization: Bearer header with this token. It is stored in your system keyring.",
        )));

        let token_switch = adw::SwitchRow::builder()
            .title(gettext("Require token").as_str())
            .subtitle(
                gettext("Strongly recommended — other local processes can reach the server.")
                    .as_str(),
            )
            .build();
        token_switch.set_active(config.api_token_enabled);
        auth_group.add(&token_switch);

        let token_row = adw::ActionRow::builder()
            .title(gettext("Token").as_str())
            .build();
        let token_label =
            gtk::Label::new(Some(&gettext("Hidden — enable the server to create it")));
        token_label.add_css_class("dim-label");
        token_label.add_css_class("monospace");
        token_label.set_selectable(false);
        token_label.set_wrap(true);
        token_label.set_max_width_chars(40);
        token_label.set_xalign(1.0);
        let copy_btn = gtk::Button::from_icon_name("edit-copy-symbolic");
        copy_btn.set_tooltip_text(Some(&gettext("Copy token")));
        copy_btn.add_css_class("flat");
        copy_btn.set_valign(gtk::Align::Center);
        let regen_btn = gtk::Button::from_icon_name("view-refresh-symbolic");
        regen_btn.set_tooltip_text(Some(&gettext("Regenerate token")));
        regen_btn.add_css_class("flat");
        regen_btn.set_valign(gtk::Align::Center);
        token_row.add_suffix(&token_label);
        token_row.add_suffix(&copy_btn);
        token_row.add_suffix(&regen_btn);
        auth_group.add(&token_row);

        self.add(&auth_group);

        // ── Status ──────────────────────────────────────────────────
        let status_group = adw::PreferencesGroup::new();
        let status_row = adw::ActionRow::builder()
            .title(gettext("Status").as_str())
            .build();
        let status_label = gtk::Label::new(None);
        status_label.add_css_class("dim-label");
        status_label.set_xalign(1.0);
        status_row.add_suffix(&status_label);
        status_group.add(&status_row);
        self.add(&status_group);

        // Stash widgets.
        *imp.enable_switch.borrow_mut() = Some(enable_switch.clone());
        *imp.port_spin.borrow_mut() = Some(port_spin.clone());
        *imp.url_row.borrow_mut() = Some(url_row.clone());
        *imp.token_switch.borrow_mut() = Some(token_switch.clone());
        *imp.token_row.borrow_mut() = Some(token_row);
        *imp.token_label.borrow_mut() = Some(token_label);
        *imp.status_label.borrow_mut() = Some(status_label);

        // ── Wiring ──────────────────────────────────────────────────

        // Enable / disable: persist, then start or stop the server live.
        let page = self.clone();
        enable_switch.connect_active_notify(move |s| {
            let active = s.is_active();
            let mut c = AppConfig::load();
            c.api_server_enabled = active;
            c.save();
            if let Some(app) = Self::app() {
                if active {
                    app.start_api_server();
                } else {
                    app.stop_api_server();
                }
            }
            page.refresh_status();
            page.refresh_token_display();
        });

        // Port change: persist; restart if running so the new port takes effect.
        let page = self.clone();
        port_adj.connect_value_changed(move |adj| {
            let port = adj.value() as u16;
            let mut c = AppConfig::load();
            if c.api_server_port == port {
                return;
            }
            c.api_server_port = port;
            let was_enabled = c.api_server_enabled;
            c.save();
            if let Some(row) = page.imp().url_row.borrow().as_ref() {
                row.set_subtitle(&format!("http://127.0.0.1:{port}"));
            }
            if was_enabled {
                if let Some(app) = Self::app() {
                    app.restart_api_server();
                }
            }
            page.refresh_status();
        });

        // Require-token toggle: persist; restart if running to apply the change.
        let page = self.clone();
        token_switch.connect_active_notify(move |s| {
            let mut c = AppConfig::load();
            c.api_token_enabled = s.is_active();
            let was_enabled = c.api_server_enabled;
            c.save();
            if was_enabled {
                if let Some(app) = Self::app() {
                    app.restart_api_server();
                }
            }
            page.refresh_token_display();
        });

        // Copy the token to the clipboard.
        let page = self.clone();
        copy_btn.connect_clicked(move |_| {
            let token = page.imp().current_token.borrow().clone();
            if let (Some(token), Some(display)) = (token, gtk::gdk::Display::default()) {
                display.clipboard().set_text(&token);
            }
        });

        // Regenerate the token (stores a fresh one, restarts the server if on).
        let page = self.clone();
        regen_btn.connect_clicked(move |_| {
            let page = page.clone();
            let (tx, rx) = async_channel::bounded::<Result<String, String>>(1);
            tokio_runtime().spawn(async move {
                let token = crate::api::generate_token();
                let result = crate::secrets::store_api_token(&token)
                    .await
                    .map(|_| token)
                    .map_err(|error| crate::error::redact_secrets(&error.to_string()));
                let _ = tx.send(result).await;
            });
            glib::spawn_future_local(async move {
                if let Ok(result) = rx.recv().await {
                    match result {
                        Ok(token) => {
                            *page.imp().current_token.borrow_mut() = Some(token);
                            if let Some(label) = page.imp().token_label.borrow().as_ref() {
                                label.set_text("••••••••••••");
                                label.remove_css_class("dim-label");
                            }
                            if AppConfig::load().api_server_enabled {
                                if let Some(app) = Self::app() {
                                    app.restart_api_server();
                                }
                            }
                        }
                        Err(error) => {
                            if let Some(label) = page.imp().token_label.borrow().as_ref() {
                                label.set_text(&gettext("Keyring error — token unchanged"));
                                label.add_css_class("dim-label");
                            }
                            tracing::warn!("Could not regenerate API token: {error}");
                        }
                    }
                }
            });
        });

        self.refresh_status();
        self.refresh_token_display();
    }

    /// Update the Status row from the current config + running state.
    fn refresh_status(&self) {
        let config = AppConfig::load();
        let running = Self::app().map(|a| a.api_server_running()).unwrap_or(false);
        let text = if config.api_server_enabled {
            if running {
                format!(
                    "{} http://127.0.0.1:{}",
                    gettext("Listening on"),
                    config.api_server_port
                )
            } else {
                gettext("Starting…")
            }
        } else {
            gettext("Stopped")
        };
        if let Some(label) = self.imp().status_label.borrow().as_ref() {
            label.set_text(&text);
        }
    }

    /// Load the stored token from the keyring (off the GTK thread) and show it,
    /// or a placeholder when token auth is off / none exists yet.
    fn refresh_token_display(&self) {
        let config = AppConfig::load();
        *self.imp().current_token.borrow_mut() = None;
        if let Some(label) = self.imp().token_label.borrow().as_ref() {
            label.add_css_class("dim-label");
            if !config.api_token_enabled {
                label.set_text(&gettext("Token not required"));
                return;
            }
            label.set_text(&gettext("Loading…"));
        }
        let page = self.clone();
        let (tx, rx) = async_channel::bounded::<Option<String>>(1);
        tokio_runtime().spawn(async move {
            let _ = tx.send(crate::secrets::load_api_token().await).await;
        });
        glib::spawn_future_local(async move {
            let token = rx.recv().await.ok().flatten();
            if let Some(label) = page.imp().token_label.borrow().as_ref() {
                match token {
                    Some(t) if !t.is_empty() => {
                        *page.imp().current_token.borrow_mut() = Some(t);
                        label.set_text("••••••••••••");
                        label.remove_css_class("dim-label");
                    }
                    _ => {
                        label.set_text(&gettext("Enable the server to create a token"));
                    }
                }
            }
        });
    }
}
