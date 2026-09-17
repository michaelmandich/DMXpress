//! Left "Fixtures" raid-grid panel and the dockable side-panel shell
//! (`dockable_side`, `side_rail`) the Inspector reuses.

use eframe::egui;

use super::{apply_zoom, icons, raid, theme, zoom_controls};
use crate::app::App;
use crate::stage::{self, RaidLook};

impl App {
    pub(crate) fn fixtures_panel(&mut self, ctx: &egui::Context) {
        let _ = self.dockable_side(
            ctx,
            "fixtures",
            "Fixtures",
            egui::panel::Side::Left,
            240.0,
            260.0,
            |app, ui| {
                app.fixtures_contents(ui);
            },
        );
    }

    fn fixtures_contents(&mut self, ui: &mut egui::Ui) {
        if self.patch.fixtures.is_empty() {
            ui.label("No patch loaded.");
        }
        let z = self.zoom.fixtures;
        // Hint on the left, the look picker on the right.
        ui.horizontal(|ui| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let mut look = self.settings.raid_look;
                egui::ComboBox::from_id_salt("raid_look")
                    .selected_text(look.label())
                    .width(92.0 * z)
                    .show_ui(ui, |ui| {
                        for l in RaidLook::ALL {
                            ui.selectable_value(&mut look, l, l.label())
                                .on_hover_text(l.hint());
                        }
                    })
                    .response
                    .on_hover_text("How the raid grid is drawn");
                if look != self.settings.raid_look {
                    self.settings.raid_look = look;
                    self.settings.save();
                }
                // The hint only when the row has room for all of it: a
                // truncated hint says nothing.
                let hint = egui::WidgetText::from(
                    egui::RichText::new("⇧ multi · ⌘ type").weak(),
                )
                .into_galley(ui, Some(egui::TextWrapMode::Extend), f32::INFINITY, egui::TextStyle::Body);
                if hint.size().x <= ui.available_width() {
                    ui.label(egui::RichText::new("⇧ multi · ⌘ type").weak());
                }
            });
        });
        let look = self.settings.raid_look;
        let buf = *self.net.dmx.lock();
        egui::ScrollArea::vertical().show(ui, |ui| {
            // Compact tiles in a wrapping grid, one per fixture, drawn in
            // whichever material the picker chose.
            let tile = raid::tile_size(look, z);
            let gap = 4.0 * z;
            ui.spacing_mut().item_spacing = egui::vec2(gap, gap);
            let mut click: Option<usize> = None;
            let mut mods = egui::Modifiers::default();
            let painter = ui.painter().clone();
            ui.horizontal_wrapped(|ui| {
                for (i, f) in self.patch.fixtures.iter().enumerate() {
                    let in_sel =
                        self.stage.fixture_selected(i) || self.sel_fixture == Some(i);
                    let (rect, resp) =
                        ui.allocate_exact_size(tile, egui::Sense::click());
                    let addr = format!("@{}", f.from);
                    raid::paint(
                        &painter,
                        look,
                        rect,
                        z,
                        &raid::Tile {
                            name: &f.display,
                            addr: &addr,
                            color: stage::fixture_swatch(f, &buf),
                            level: stage::fixture_level(f, &buf),
                            selected: in_sel,
                        },
                    );
                    let resp = resp.on_hover_text(format!(
                        "{}  ·  DMX {}..{}  ·  {}ch",
                        f.display,
                        f.from,
                        f.to,
                        f.channel_count()
                    ));
                    if resp.clicked() {
                        click = Some(i);
                        mods = ui.input(|inp| inp.modifiers);
                    }
                }
            });
            if let Some(i) = click {
                if mods.command {
                    self.stage.select_same_type(&self.patch, i);
                } else if mods.shift {
                    self.stage.toggle_fixture(i);
                } else {
                    self.stage.select_fixture(i, false);
                }
                self.sel_fixture = Some(i);
            }
        });
    }

    /// The zoom level a docked panel keeps, by its key.
    fn panel_zoom(&mut self, key: &str) -> &mut f32 {
        match key {
            "fixtures" => &mut self.zoom.fixtures,
            _ => &mut self.zoom.inspector,
        }
    }

    /// A side panel that can be folded to a thin rail or torn off into its
    /// own OS window, so the stage can have the whole screen.
    ///
    /// `key` names the panel in `popped_out` / `collapsed` and is its egui
    /// id. Closing the OS window folds the panel rather than losing it.
    /// `default_width` is the docked width on a fresh launch (egui keeps
    /// the dragged width for the session). Returns the docked panel's
    /// width, or `None` while it is popped out or folded.
    pub(crate) fn dockable_side(
        &mut self,
        ctx: &egui::Context,
        key: &'static str,
        title: &str,
        side: egui::panel::Side,
        min_width: f32,
        default_width: f32,
        contents: impl FnOnce(&mut Self, &mut egui::Ui),
    ) -> Option<f32> {
        let popped = self.popped_out.contains(key);
        let collapsed = self.collapsed.contains(key);
        let mut zoom_level = *self.panel_zoom(key);
        let width = if popped {
            let outcome = super::popped_viewport(
                ctx,
                key,
                title,
                Some(&mut zoom_level),
                [380.0, 640.0],
                |ui| contents(&mut *self, ui),
            );
            if outcome.dock {
                self.popped_out.remove(key);
            }
            if outcome.closed {
                self.popped_out.remove(key);
                self.collapsed.insert(key);
            }
            None
        } else if collapsed {
            if side_rail(ctx, key, title, side) {
                self.collapsed.remove(key);
            }
            None
        } else {
            let mut pop = false;
            let mut fold = false;
            let frame = super::theme::panel_frame(&ctx.style());
            let inner = egui::SidePanel::new(side, key)
                .min_width(min_width)
                .default_width(default_width)
                .frame(frame)
                .show(ctx, |ui| {
                    super::theme::panel_header(ui, title, |ui| {
                        let chevron = match side {
                            egui::panel::Side::Left => icons::Icon::ChevronLeft,
                            egui::panel::Side::Right => icons::Icon::ChevronRight,
                        };
                        let fold_hint = if key == "inspector" {
                            "Fold this panel away, leaving more room for the stage (Alt+I)"
                        } else {
                            "Fold this panel away, leaving more room for the stage"
                        };
                        if icons::icon_button(ui, chevron, None).on_hover_text(fold_hint).clicked() {
                            fold = true;
                        }
                        if icons::icon_button(ui, icons::Icon::PopOut, None)
                            .on_hover_text("Open this panel in its own window")
                            .clicked()
                        {
                            pop = true;
                        }
                        zoom_controls(ui, &mut zoom_level);
                    });
                    apply_zoom(ui, zoom_level);
                    contents(&mut *self, ui);
                });
            if pop {
                self.popped_out.insert(key);
            }
            if fold {
                self.collapsed.insert(key);
            }
            Some(inner.response.rect.width())
        };
        *self.panel_zoom(key) = zoom_level;
        width
    }
}

/// The thin strip a folded side panel leaves behind: a chevron pointing
/// back into the stage and the panel's name written down the strip. The
/// whole strip is one button; returns true when it is clicked.
fn side_rail(ctx: &egui::Context, key: &str, title: &str, side: egui::panel::Side) -> bool {
    const RAIL_W: f32 = 26.0;
    let mut clicked = false;
    egui::SidePanel::new(side, egui::Id::new(key).with("rail"))
        .exact_width(RAIL_W)
        .resizable(false)
        .frame(egui::Frame::none().fill(theme::SURFACE))
        .show(ctx, |ui| {
            let rect = ui.available_rect_before_wrap();
            let resp = ui.allocate_rect(rect, egui::Sense::click());
            let hovered = resp.hovered();
            let painter = ui.painter();
            if hovered {
                painter.rect_filled(rect, 0.0, theme::HOVER);
            }
            let color = if hovered { theme::TEXT } else { theme::TEXT_DIM };
            let chevron = match side {
                egui::panel::Side::Left => icons::Icon::ChevronRight,
                egui::panel::Side::Right => icons::Icon::ChevronLeft,
            };
            let icon_rect = egui::Rect::from_center_size(
                egui::Pos2::new(rect.center().x, rect.top() + 18.0),
                egui::Vec2::splat(14.0),
            );
            icons::draw(painter, icon_rect, chevron, color);
            // The name reads top to bottom: rotated a quarter turn clockwise
            // about its top-left corner, so that corner ends up on the right
            // edge of the text's final footprint.
            let galley = painter.layout_no_wrap(
                title.to_uppercase(),
                egui::FontId::new(11.0, theme::medium()),
                color,
            );
            let pos = egui::Pos2::new(rect.center().x + galley.size().y * 0.5, rect.top() + 38.0);
            painter.add(egui::Shape::Text(
                egui::epaint::TextShape::new(pos, galley, color)
                    .with_angle(std::f32::consts::FRAC_PI_2),
            ));
            clicked = resp.on_hover_text(format!("Show {title}")).clicked();
        });
    clicked
}
