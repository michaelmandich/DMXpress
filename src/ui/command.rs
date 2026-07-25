//! The command line — a single text field that parses short console-style
//! commands, the renamed grandMA3 *command bar*.
//!
//! Supported verbs (case-insensitive):
//!   clear            release the programmer
//!   black / bo       blackout (release every stack + clear programmer)
//!   go [n]           advance current stack, or stack n
//!   off [n]          release current/all stacks, or stack n
//!   store            record the programmer into the current stack
//!   cue n            jump the current stack to cue position n
//!   group n          recall (select) group n
//!   gm n             set grand master to n percent
//!   full             grand master to 100%

use eframe::egui;

use crate::app::App;

impl App {
    pub(crate) fn run_command(&mut self, raw: &str) {
        let line = raw.trim().to_lowercase();
        if line.is_empty() {
            return;
        }
        let tok: Vec<&str> = line.split_whitespace().collect();
        let num = |s: &str| s.parse::<usize>().ok();
        match tok.as_slice() {
            ["clear"] | ["cl"] | ["c"] => self.clear_programmer(),
            ["black"] | ["blackout"] | ["bo"] => {
                for st in &mut self.stacks {
                    st.release();
                }
                self.clear_programmer();
                self.log.push("Blackout — everything released".into());
            }
            ["full"] => {
                self.grand_master = 1.0;
                self.log.push("Grand master → 100%".into());
            }
            ["gm", n] => match num(n) {
                Some(v) => {
                    let v = v.min(100);
                    self.grand_master = v as f32 / 100.0;
                    self.log.push(format!("Grand master → {v}%"));
                }
                None => self.log.push("Usage: gm <0-100>".into()),
            },
            ["go"] => match self.cur_stack {
                Some(i) => self.go_stack(i),
                None => self.log.push("No stack selected (open 🎬 Stacks)".into()),
            },
            ["go", n] => match num(n) {
                Some(s) if (1..=self.stacks.len()).contains(&s) => self.go_stack(s - 1),
                _ => self.log.push("Usage: go <stack #>".into()),
            },
            ["off"] => {
                for i in 0..self.stacks.len() {
                    self.release_stack(i);
                }
            }
            ["off", n] => match num(n) {
                Some(s) if (1..=self.stacks.len()).contains(&s) => self.release_stack(s - 1),
                _ => self.log.push("Usage: off <stack #>".into()),
            },
            ["store"] => match self.cur_stack {
                Some(i) => self.store_cue(i),
                None => self.log.push("No stack selected to store into".into()),
            },
            ["cue", n] => match (self.cur_stack, num(n)) {
                (Some(i), Some(c)) if c >= 1 => self.fire_cue(i, c - 1),
                (None, _) => self.log.push("No stack selected".into()),
                _ => self.log.push("Usage: cue <#>".into()),
            },
            ["group", n] | ["grp", n] | ["g", n] => match num(n) {
                Some(gi) if (1..=self.groups.len()).contains(&gi) => self.recall_group(gi - 1),
                _ => self.log.push("Usage: group <#>".into()),
            },
            _ => self.log.push(format!("? unknown command: {raw}")),
        }
    }

    pub(crate) fn command_bar(&mut self, ctx: &egui::Context) {
        if !self.show_command {
            return;
        }
        let mut submit = false;
        egui::TopBottomPanel::bottom("command").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.strong("⌨");
                let resp = ui.add(
                    egui::TextEdit::singleline(&mut self.command)
                        .desired_width(f32::INFINITY)
                        .hint_text(
                            "clear · go [n] · off [n] · store · cue n · group n · gm n · black",
                        ),
                );
                if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    submit = true;
                }
            });
        });
        if submit {
            let line = std::mem::take(&mut self.command);
            self.run_command(&line);
        }
    }
}
