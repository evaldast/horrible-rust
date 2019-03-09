#![deny(clippy::all)]
#![deny(clippy::pedantic)]

#[macro_use]
extern crate failure;
#[macro_use]
extern crate serde_derive;

mod persistence;
mod ui;

use console::{style, Style};
use dialoguer::{theme::ColorfulTheme, Checkboxes, Confirmation, Input, Select};
use failure::Error;
use kuchiki::traits::TendrilSink;
use kuchiki::{ElementData, NodeDataRef, NodeRef};
use persistence::Episode;
use regex::Regex;
use rss::Channel;
use rusqlite::Connection;
use std::{fs, io::prelude::Write, path::Path, process::Command, thread};

const BACK_SELECTION: &str = "<< BACK";
const CONFIG_FILE_NAME: &str = "config.toml";
const DB_FILE_NAME: &str = "main.db";
const AVAILABLE_RESOLUTIONS: &[&str] = &["480p", "720p", "1080p"];

#[derive(Deserialize, Serialize, Clone)]
pub struct Config {
    player_path: String,
    show_resolution: String,
    feed_url: String,
    current_season_url: String,
}

fn main() {
    match run() {
        Ok(_) => {}
        Err(error) => println!("{} - {}", style("An error occured").bold().red(), error),
    }
}

fn run() -> Result<(), Error> {
    let config = setup_config()?;

    println!("{}", style("[LOADING..]").bold().on_red());

    let sql_conn = Connection::open(DB_FILE_NAME)?;

    persistence::initialize_sql_tables(&sql_conn)?;
    initialize_subscriptions(&sql_conn, &config.current_season_url)?;
    start_rss_thread(config.clone());

    println!("{}", style("[LOADING COMPLETE]").bold().on_green());

    loop {
        handle_user(&sql_conn, &config)?;
    }
}

fn setup_config() -> Result<Config, Error> {
    if !Path::new(CONFIG_FILE_NAME).exists() {
        match prompt_config_wizard() {
            Ok(None) => bail!("Configuration Wizard failed"),
            Ok(Some(config)) => {
                let mut file = std::fs::File::create(CONFIG_FILE_NAME)?;
                file.write_all(toml::to_string(&config)?.as_bytes())?;

                return Ok(config);
            }
            Err(error) => bail!(error),
        }
    };

    let config_str: String = fs::read_to_string(CONFIG_FILE_NAME)?;

    Ok(toml::from_str(&config_str)?)
}

fn handle_user(sql_conn: &Connection, user_config: &Config) -> Result<(), Error> {
    let selections = &[
        "Available Subscriptions",
        "My Subscriptions",
        "New Episodes",
    ];

    match prompt_menu(selections) {
        0 => handle_available_subscriptions(&sql_conn, &user_config.show_resolution),
        1 => handle_my_subscriptions(&sql_conn, &user_config),
        2 => handle_new_episodes(&sql_conn, &user_config),
        _ => bail!("Menu selection failed"),
    }
}

fn prompt_config_wizard() -> Result<Option<Config>, Box<Error>> {
    let theme = ColorfulTheme {
        values_style: Style::new().yellow().dim(),
        indicator_style: Style::new().yellow().bold(),
        yes_style: Style::new().yellow().dim(),
        no_style: Style::new().yellow().dim(),
        ..ColorfulTheme::default()
    };

    println!("Welcome. It seems like configuration is missing, let's set you up!");

    if !Confirmation::with_theme(&theme)
        .with_text("Do you want to continue?")
        .interact()
        .unwrap()
    {
        return Ok(None);
    }

    let feed_url = Input::with_theme(&theme)
        .with_prompt("RSS Feed URL:")
        .interact()
        .unwrap();

    let current_season_url = Input::with_theme(&theme)
        .with_prompt("Current season URL:")
        .interact()
        .unwrap();

    let player_path = Input::with_theme(&theme)
        .with_prompt("Path to video player:")
        .interact()
        .unwrap();

    let selected_resolution = Select::with_theme(&theme)
        .with_prompt("Select show resolution:")
        .default(0)
        .items(AVAILABLE_RESOLUTIONS)
        .interact()
        .unwrap();

    Ok(Some(Config {
        feed_url,
        player_path,
        show_resolution: AVAILABLE_RESOLUTIONS[selected_resolution].to_string(),
        current_season_url,
    }))
}

fn prompt_menu(selections: &[&'static str]) -> usize {
    Select::with_theme(&ColorfulTheme::default())
        .with_prompt("Choose an option")
        .default(0)
        .items(&selections)
        .interact()
        .unwrap()
}

fn handle_available_subscriptions(
    sql_conn: &Connection,
    user_resolution: &str,
) -> Result<(), Error> {
    let shows = persistence::fetch_shows(&sql_conn, false)?;

    if shows.is_empty() {
        println!("{}", style("No available subscriptions").bold().red());

        return Ok(());
    }

    let mut show_titles: Vec<&str> = shows.iter().map(|show| show.title.as_ref()).collect();

    show_titles.sort();

    let checks = Checkboxes::with_theme(&ColorfulTheme::default())
        .with_prompt("Select subscriptions")
        .items(&show_titles[..])
        .interact()?;

    for check in checks {
        let selected_show = &shows[check];

        persistence::subscribe_to_show(sql_conn, selected_show.id)?;

        println!(
            "{}{}{}",
            style("[SUBSCRIBED TO:").bold().magenta(),
            style(&selected_show.title).bold().dim(),
            style("]").bold().magenta()
        );

        persistence::persist_new_episodes(
            sql_conn,
            fetch_episodes_from_feed(&selected_show.title)?,
            true,
            user_resolution,
        )?;
    }

    Ok(())
}

fn handle_my_subscriptions(sql_conn: &Connection, user_config: &Config) -> Result<(), Error> {
    let shows = persistence::fetch_shows(&sql_conn, true)?;

    if shows.is_empty() {
        println!(
            "{}",
            style("No shows found! Please subscribe first").bold().red()
        );

        return Ok(());
    }

    let mut show_titles: Vec<&str> = shows.iter().map(|show| show.title.as_ref()).collect();

    show_titles.sort();
    show_titles.push(BACK_SELECTION);

    loop {
        let show_selection = Select::with_theme(&ColorfulTheme::default())
            .with_prompt("Choose a show")
            .default(0)
            .items(&show_titles[..])
            .interact()?;

        if show_titles[show_selection] == BACK_SELECTION {
            return Ok(());
        }

        handle_episodes(&show_titles[show_selection], &sql_conn, &user_config)?
    }
}

fn handle_episodes(
    show_title: &str,
    sql_conn: &Connection,
    user_config: &Config,
) -> Result<(), Error> {
    let mut episodes: Vec<Episode> =
        persistence::fetch_episodes_for_show(sql_conn, show_title, &user_config.show_resolution)?;

    episodes.sort();

    let mut available_selections: Vec<String> =
        episodes.iter().map(|ep| ep.formatted_title()).collect();

    available_selections.push(BACK_SELECTION.to_string());

    let selection = Select::with_theme(&ColorfulTheme::default())
        .with_prompt("Choose an episode")
        .default(0)
        .items(&available_selections[..])
        .interact()?;

    if available_selections[selection] == BACK_SELECTION {
        return Ok(());
    }

    let selected_episode = &episodes[selection];

    println!(
        "{}{}{}",
        style("[LOADING EPISODE:").bold().magenta(),
        style(&available_selections[selection]).bold().dim(),
        style("]").bold().magenta()
    );

    match open_episode(
        selected_episode.torrent_link.clone(),
        user_config.player_path.to_string(),
    ) {
        Ok(_) => {
            persistence::flag_episode_as_watched(&sql_conn, selected_episode.id)?;
            Ok(())
        }
        Err(error) => bail!(error),
    }
}

fn initialize_subscriptions(sql_conn: &Connection, current_season_url: &str) -> Result<(), Error> {
    persistence::insert_show_titles(&sql_conn, &fetch_current_season_titles(current_season_url)?)
}

fn start_rss_thread(user_config: Config) {
    thread::spawn(move || loop {
        match watch_feed(&user_config) {
            Ok(_) => {}
            Err(error) => println!(
                "{} - {}",
                style("An error occured in RSS thread. Restarting")
                    .bold()
                    .red(),
                error
            ),
        };
        thread::sleep(std::time::Duration::from_secs(10));
    });
}

fn watch_feed(user_config: &Config) -> Result<(), Error> {
    let sql_conn = Connection::open("main.db")?;
    let mut current_feed_string = "".to_string();

    loop {
        let feed = Channel::from_url(&user_config.feed_url)?;
        let new_feed_string = feed.to_string();

        if current_feed_string != new_feed_string {
            // https://github.com/rust-syndication/rss/issues/74
            persistence::persist_new_episodes(
                &sql_conn,
                map_feed_to_episodes(&feed),
                false,
                &user_config.show_resolution,
            )?;

            current_feed_string = new_feed_string;
        }

        thread::sleep(std::time::Duration::from_secs(60));
    }
}

fn fetch_episodes_from_feed(show_title: &str) -> Result<Vec<Episode>, Error> {
    let feed_url = format!(
        "https://nyaa.si/?page=rss&q={}&c=0_0&f=0&u=HorribleSubs",
        show_title.replace(" ", "+")
    );

    let feed = Channel::from_url(&feed_url)?;

    Ok(map_feed_to_episodes(&feed))
}

fn map_feed_to_episodes(feed: &Channel) -> Vec<Episode> {
    feed.items()
        .iter()
        .map(|item| {
            let captures = capture_variables_from_title(&item);

            Episode {
                id: 0,
                title: captures["title"].to_string(),
                episode: captures["episode"].to_string(),
                version: captures["version"].to_string(),
                show_id: 0,
                watched: 0,
                resolution: captures["resolution"].to_string(),
                torrent_link: item.link().unwrap().to_string(),
            }
        })
        .collect()
}

fn fetch_current_season_titles(url: &str) -> Result<Vec<String>, Error> {
    let doc = fetch_document(url)?;
    let mut title_selector = doc.select(".shows-wrapper").unwrap();
    let title_wrapper: NodeDataRef<ElementData> = title_selector.next().unwrap();
    let text_content = title_wrapper.text_contents();

    let titles: Vec<String> = text_content
        .replace("\u{2013}", "-")
        .replace("\u{2019}", "'")
        .lines()
        .filter_map(|t| {
            if t.is_empty() {
                None
            } else {
                Some(t.to_string())
            }
        })
        .collect();

    Ok(titles)
}

fn fetch_document(url: &str) -> Result<NodeRef, Error> {
    let mut res = reqwest::get(url)?;

    Ok(kuchiki::parse_html().one(res.text()?))
}

fn open_episode(torrent_path: String, player_path: String) -> Result<(), Error> {
    let child_thread = thread::spawn(|| {
        Command::new(player_path)
            .args(&[torrent_path])
            .output()
            .expect("failed to execute process");
    })
    .join();

    match child_thread {
        Ok(_) => Ok(()),
        Err(_) => bail!("Problem with opening episode"),
    }
}

fn handle_new_episodes(sql_conn: &Connection, user_config: &Config) -> Result<(), Error> {
    let new_episodes = persistence::fetch_new_episodes(sql_conn, user_config)?;

    if new_episodes.is_empty() {
        println!("{}", style("No new episodes found").bold().red());

        return Ok(());
    }

    let mut episode_titles: Vec<String> =
        new_episodes.iter().map(|ep| ep.formatted_title()).collect();

    episode_titles.push(BACK_SELECTION.to_string());

    let episode_selection = Select::with_theme(&ColorfulTheme::default())
        .with_prompt("Choose a show")
        .default(0)
        .items(&episode_titles[..])
        .interact()?;

    if episode_titles[episode_selection] == BACK_SELECTION {
        return Ok(());
    }

    let selected_episode = &new_episodes[episode_selection];

    println!(
        "{}{}{}",
        style("[LOADING EPISODE:").bold().magenta(),
        style(&selected_episode.formatted_title()).bold().dim(),
        style("]").bold().magenta()
    );

    match open_episode(
        selected_episode.torrent_link.clone(),
        user_config.player_path.to_string(),
    ) {
        Ok(_) => {
            persistence::flag_episode_as_watched(&sql_conn, selected_episode.id)?;
            Ok(())
        }
        Err(error) => bail!(error),
    }
}

fn capture_variables_from_title(feed_item: &rss::Item) -> regex::Captures<'_> {
    let regex = Regex::new("\\[(?P<subber>.+)\\]\\s*(?P<title>.+?)[\\s*\\-*]+(?P<episode>\\d{1,4}.\\d*)[\\s*]\\[(?P<resolution>\\d{3,4}p)\\](?P<version>v*\\d*)").unwrap();

    regex.captures(feed_item.title().unwrap()).unwrap()
}
