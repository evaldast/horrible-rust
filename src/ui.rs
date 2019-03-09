use crate::Episode;
use console::{style, Style};
use dialoguer::{theme::ColorfulTheme, Checkboxes, Confirmation, Input, Select};

pub fn announce_new_episode(episode: &Episode, watched: bool) {
    let announcement_text = if watched {
        style("[EPISODE ADDED:").magenta().dim()
    } else {
        style("[NEW EPISODE ARRIVAL:").magenta().dim()
    };

    println!(
        "{}{}{}{}{}",
        announcement_text,
        style(&episode.title).bold().dim(),
        style(" - ").bold().dim(),
        style(&episode.episode).bold().dim(),
        style("]").bold().magenta()
    );
}
