//! Floating "Plugins" window — the pool of installed Rhai scripts — plus one
//! small window per plugin that declares controls. See `plugin.rs` for the
//! script format itself.

use eframe::egui;

use crate::app::App;
use crate::plugin::{self, Control};

/// Reveal the plugins folder in the OS file manager.
fn open_plugins_folder() {
    let path = std::env::current_dir()
        .unwrap_or_default()
        .join(crate::paths::data_path(plugin::PLUGINS_DIR));
    let _ = std::fs::create_dir_all(&path);
    #[cfg(target_os = "windows")]
    let _ = std::process::Command::new("explorer").arg(&path).spawn();
    #[cfg(target_os = "macos")]
    let _ = std::process::Command::new("open").arg(&path).spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    let _ = std::process::Command::new("xdg-open").arg(&path).spawn();
}

impl App {
    /// Re-scan the folder for new/changed scripts, keeping enabled flags.
    pub(crate) fn reload_plugins(&mut self) {
        plugin::save_state(&self.plugins);
        self.plugins = plugin::load_all(&self.plugin_engine);
        self.plugin_theme_dirty = true;
        self.log.push(format!("Plugins: {} loaded", self.plugins.len()));
    }

    pub(crate) fn plugins_window(&mut self, ctx: &egui::Context) {
        if !self.show_plugins {
            return;
        }
        let screen = ctx.screen_rect();
        let mut open = self.show_plugins;
        let mut popped = self.popped_out.contains("plugins");

        // Dropping .rhai files anywhere while this window is open installs them.
        let dropped: Vec<std::path::PathBuf> = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .filter_map(|f| f.path.clone())
                .filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("rhai")))
                .collect()
        });
        if !dropped.is_empty() {
            let _ = std::fs::create_dir_all(crate::paths::data_path(plugin::PLUGINS_DIR));
            for src in &dropped {
                if let Some(name) = src.file_name() {
                    let dst = crate::paths::data_path(plugin::PLUGINS_DIR).join(name);
                    match std::fs::copy(src, &dst) {
                        Ok(_) => self.log.push(format!("Plugins: installed {}", name.to_string_lossy())),
                        Err(e) => self.log.push(format!("Plugins: install failed — {e}")),
                    }
                }
            }
            self.reload_plugins();
        }

        // Deferred row actions so the list borrow stays clean.
        enum Act {
            Toggle(usize),
            Reload(usize),
            AskRemove(usize),
            Remove(usize),
            CancelRemove,
        }
        let mut act: Option<Act> = None;

        let mut zoom_level = self.zoom.plugins;
        super::floating_panel(
            ctx,
            "plugins",
            "Plugins",
            &mut open,
            &mut popped,
            Some(&mut zoom_level),
            [460.0, 420.0],
            [screen.left() + 260.0, 160.0],
            |ui| {
                ui.horizontal(|ui| {
                    if ui.button("Open folder").on_hover_text("Reveal the plugins folder — drop .rhai files here").clicked() {
                        open_plugins_folder();
                    }
                    if ui.button("Reload all").on_hover_text("Re-scan the folder and recompile every script").clicked() {
                        act = Some(Act::Reload(usize::MAX));
                    }
                    if ui.button("Get plugins").on_hover_text(plugin::SITE_PLUGINS_URL).clicked() {
                        ctx.open_url(egui::OpenUrl::new_tab(plugin::SITE_PLUGINS_URL));
                    }
                });
                ui.small("Install by dropping a .rhai file on this window, or copy it into the folder and press Reload all.");
                ui.separator();

                if self.plugins.is_empty() {
                    ui.weak("No plugins installed. A plugin is a single .rhai script that can paint its own layer, open a window of controls, or tint the theme.");
                    return;
                }

                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                    for (i, p) in self.plugins.iter_mut().enumerate() {
                        ui.horizontal(|ui| {
                            let mut on = p.enabled;
                            if ui
                                .checkbox(&mut on, egui::RichText::new(&p.meta.name).strong())
                                .on_hover_text(&p.file_name())
                                .changed()
                            {
                                act = Some(Act::Toggle(i));
                            }
                            ui.weak(format!("v{}", p.meta.version));
                            if !p.meta.author.is_empty() {
                                ui.weak(format!("by {}", p.meta.author));
                            }
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if self.plugin_confirm_remove == Some(i) {
                                    if ui.button("Cancel").clicked() {
                                        act = Some(Act::CancelRemove);
                                    }
                                    if ui
                                        .button(egui::RichText::new("Delete file").color(ui.visuals().error_fg_color))
                                        .clicked()
                                    {
                                        act = Some(Act::Remove(i));
                                    }
                                } else {
                                    if ui.small_button("Remove").on_hover_text("Delete the script file").clicked() {
                                        act = Some(Act::AskRemove(i));
                                    }
                                    if ui.small_button("Reload").on_hover_text("Recompile this script from disk").clicked() {
                                        act = Some(Act::Reload(i));
                                    }
                                    if !p.controls.is_empty() {
                                        ui.toggle_value(&mut p.show_window, "Window")
                                            .on_hover_text("Show this plugin's own control window");
                                    }
                                }
                            });
                        });
                        if !p.meta.description.is_empty() {
                            ui.weak(&p.meta.description);
                        }
                        let mut tags: Vec<&str> = Vec::new();
                        if p.has_frame {
                            tags.push("layer");
                        }
                        if !p.controls.is_empty() {
                            tags.push("window");
                        }
                        if !p.theme.is_empty() {
                            tags.push("theme");
                        }
                        if !tags.is_empty() {
                            ui.small(tags.join(" · "));
                        }
                        if let Some(e) = &p.error {
                            ui.colored_label(ui.visuals().error_fg_color, e);
                        }
                        ui.separator();
                    }
                });
            },
        );
        self.zoom.plugins = zoom_level;

        match act {
            Some(Act::Toggle(i)) => {
                if let Some(p) = self.plugins.get_mut(i) {
                    p.enabled = !p.enabled;
                    let state = if p.enabled { "enabled" } else { "disabled" };
                    self.log.push(format!("Plugin '{}' {state}", p.meta.name));
                }
                plugin::save_state(&self.plugins);
                self.plugin_theme_dirty = true;
            }
            Some(Act::Reload(i)) => {
                if i == usize::MAX {
                    self.reload_plugins();
                } else if let Some(p) = self.plugins.get_mut(i) {
                    plugin::reload(&self.plugin_engine, p);
                    self.plugin_theme_dirty = true;
                    self.log.push(format!("Plugin '{}' reloaded", p.meta.name));
                }
            }
            Some(Act::AskRemove(i)) => self.plugin_confirm_remove = Some(i),
            Some(Act::CancelRemove) => self.plugin_confirm_remove = None,
            Some(Act::Remove(i)) => {
                if i < self.plugins.len() {
                    let p = self.plugins.remove(i);
                    match std::fs::remove_file(&p.file) {
                        Ok(_) => self.log.push(format!("Plugin '{}' removed", p.meta.name)),
                        Err(e) => self.log.push(format!("Plugin remove failed — {e}")),
                    }
                }
                self.plugin_confirm_remove = None;
                plugin::save_state(&self.plugins);
                self.plugin_theme_dirty = true;
            }
            None => {}
        }

        if popped {
            self.popped_out.insert("plugins");
        } else {
            self.popped_out.remove("plugins");
        }
        self.show_plugins = open;
    }

    /// One small window per enabled plugin that declared controls.
    pub(crate) fn plugin_panels(&mut self, ctx: &egui::Context) {
        let t = self.plugin_epoch.elapsed().as_secs_f64();
        let mut presses: Vec<(usize, String)> = Vec::new();
        for (i, p) in self.plugins.iter_mut().enumerate() {
            if !p.enabled || p.controls.is_empty() || !p.show_window {
                continue;
            }
            let mut open = true;
            egui::Window::new(&p.meta.name)
                .id(egui::Id::new(("plugin-window", p.file_name())))
                .open(&mut open)
                .default_width(280.0)
                .resizable(true)
                .show(ctx, |ui| {
                    for c in &p.controls {
                        match c {
                            Control::Label { text } => {
                                ui.weak(text);
                            }
                            Control::Slider { id, label, min, max, .. } => {
                                let mut v = p
                                    .state
                                    .get(id.as_str())
                                    .and_then(|d| d.as_float().ok())
                                    .unwrap_or(*min as f64)
                                    as f32;
                                if ui
                                    .add(egui::Slider::new(&mut v, *min..=*max).text(label))
                                    .changed()
                                {
                                    p.state.insert(id.as_str().into(), rhai::Dynamic::from_float(v as f64));
                                }
                            }
                            Control::Checkbox { id, label, .. } => {
                                let mut v = p
                                    .state
                                    .get(id.as_str())
                                    .and_then(|d| d.as_bool().ok())
                                    .unwrap_or(false);
                                if ui.checkbox(&mut v, label).changed() {
                                    p.state.insert(id.as_str().into(), rhai::Dynamic::from_bool(v));
                                }
                            }
                            Control::Button { id, label } => {
                                if ui.button(label).clicked() {
                                    presses.push((i, id.clone()));
                                }
                            }
                        }
                    }
                    if let Some(e) = &p.error {
                        ui.colored_label(ui.visuals().error_fg_color, e);
                    }
                });
            if !open {
                p.show_window = false;
            }
        }
        for (i, id) in presses {
            if let Some(p) = self.plugins.get_mut(i) {
                p.press(&self.plugin_engine, &id, t);
            }
        }
    }
}
