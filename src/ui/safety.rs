//! The show-safety controls: the Backups and Export / import sections of
//! the Configurations window, drop-a-show-file-to-import, and the confirm
//! dialogs that guard a restore or import.

use std::path::{Path, PathBuf};

use eframe::egui;

use super::theme;
use crate::app::App;
use crate::backup;

impl App {
    /// The Backups and Export / import sections, drawn inside the
    /// Configurations window.
    pub(crate) fn safety_sections(&mut self, ui: &mut egui::Ui) {
        ui.add_space(8.0);
        theme::section(ui, "Backups");
        theme::hint(
            ui,
            "The whole show is backed up every minute in which something changed, \
             and when you quit. The newest 20 are kept.",
        );
        ui.horizontal(|ui| {
            if ui
                .button("Back up now")
                .on_hover_text("Write the show to the backups folder right away")
                .clicked()
            {
                match backup::write("manual", &self.snapshot_configuration()) {
                    Ok(path) => {
                        self.log.push(format!("Backed up to {}", path.display()));
                        self.autosave.mark(self.undo.hash(), path);
                    }
                    Err(e) => self.log.push(format!("Backup failed: {e}")),
                }
            }
            if ui
                .button("Show folder")
                .on_hover_text("Open the backups folder in your file manager")
                .clicked()
            {
                let _ = std::fs::create_dir_all(backup::BACKUPS_DIR);
                backup::reveal(Path::new(backup::BACKUPS_DIR));
            }
            if let Some((path, at)) = &self.autosave.last_written {
                let name = path.file_stem().map(|s| s.to_string_lossy().into_owned());
                ui.weak(format!(
                    "last: {} · {}",
                    name.unwrap_or_default(),
                    crate::undo::ago(at.elapsed())
                ));
            }
        });
        let entries = backup::list();
        if entries.is_empty() {
            ui.weak("No backups yet.");
        }
        egui::ScrollArea::vertical()
            .max_height(170.0)
            .id_salt("backups_list")
            .show(ui, |ui| {
                for e in &entries {
                    ui.horizontal(|ui| {
                        if ui
                            .small_button("Restore")
                            .on_hover_text(
                                "Replace the current show with this backup. The current show \
                                 is backed up first, and Ctrl+Shift+Z undoes the restore.",
                            )
                            .clicked()
                        {
                            self.confirm_restore = Some(e.path.clone());
                        }
                        ui.label(egui::RichText::new(&e.name).monospace().size(11.0));
                        ui.weak(format!("{} · {} KB", backup::ago(e.modified), e.bytes.max(1024) / 1024));
                    });
                }
            });

        ui.add_space(8.0);
        theme::section(ui, "Export / import");
        ui.horizontal(|ui| {
            if ui
                .button("Export show…")
                .on_hover_text(
                    "Write the whole show to one file in the exports folder and show it \
                     there, ready to copy to another machine or send to someone",
                )
                .clicked()
            {
                let name = if self.config_name.trim().is_empty() {
                    "show".to_string()
                } else {
                    self.config_name.trim().to_string()
                };
                match backup::export(&name, &self.snapshot_configuration()) {
                    Ok(path) => {
                        self.log.push(format!("Exported show to {}", path.display()));
                        backup::reveal(&path);
                    }
                    Err(e) => self.log.push(format!("Export failed: {e}")),
                }
            }
            theme::hint(ui, "Named after the configuration name above.");
        });
        theme::hint(ui, "To import, drop a show file onto the window, or paste its path:");
        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut self.import_path)
                    .hint_text("path to a .json show file")
                    .desired_width(220.0),
            );
            if ui.button("Import…").clicked() && !self.import_path.trim().is_empty() {
                self.pending_import = Some(PathBuf::from(self.import_path.trim()));
            }
        });
    }

    /// Every frame: a dropped show file asks to be imported, and the
    /// restore / import confirmations are shown while pending.
    pub(crate) fn safety_windows(&mut self, ctx: &egui::Context) {
        let dropped: Option<PathBuf> = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .filter_map(|f| f.path.clone())
                .find(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("json")))
        });
        if let Some(path) = dropped {
            self.pending_import = Some(path);
        }

        if let Some(path) = self.pending_import.clone() {
            let mut decided = false;
            egui::Window::new("Import show?")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    ui.label(format!(
                        "Replace the current show with\n{}?",
                        path.display()
                    ));
                    theme::hint(
                        ui,
                        "The current show is backed up first, and Ctrl+Shift+Z undoes the import.",
                    );
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if ui.button("Import").clicked() {
                            self.replace_show_from(&path, "before-import");
                            decided = true;
                        }
                        if ui.button("Cancel").clicked() {
                            decided = true;
                        }
                    });
                });
            if decided {
                self.pending_import = None;
            }
        }

        if let Some(path) = self.confirm_restore.clone() {
            let mut decided = false;
            let name = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            egui::Window::new("Restore backup?")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    ui.label(format!("Replace the current show with backup '{name}'?"));
                    theme::hint(
                        ui,
                        "The current show is backed up first, and Ctrl+Shift+Z undoes the restore.",
                    );
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if ui.button("Restore").clicked() {
                            self.replace_show_from(&path, "before-restore");
                            decided = true;
                        }
                        if ui.button("Cancel").clicked() {
                            decided = true;
                        }
                    });
                });
            if decided {
                self.confirm_restore = None;
            }
        }
    }

    /// Load a show file over the current show, backing the current one up
    /// first as `<kind>-<stamp>`.
    fn replace_show_from(&mut self, path: &Path, kind: &str) {
        match backup::read(path) {
            Ok(cfg) => {
                if let Some(saved) = self.backup_now_if_changed(kind) {
                    self.log.push(format!("Current show backed up to {}", saved.display()));
                }
                self.apply_configuration(cfg);
                self.log.push(format!("Loaded show from {}", path.display()));
            }
            Err(e) => self.log.push(format!("Could not load {}: {e}", path.display())),
        }
    }
}
