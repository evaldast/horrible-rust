#![deny(clippy::all)]
#![deny(clippy::pedantic)]

use console::style;
use dialoguer::{theme::ColorfulTheme, Checkboxes, Select};
use failure::Error;
use kuchiki::traits::*;
use kuchiki::*;
use regex::Regex;
use rss::Channel;
use rusqlite::{Connection, NO_PARAMS};
use std::collections::HashMap;
use std::fs;
use std::process::Command;
use std::thread;

#[macro_use]
extern crate failure;
#[macro_use]
extern crate serde_derive;

const BACK_SELECTION: &str = "<< BACK";

#[derive(Deserialize, Clone)]
struct UserConfig {
    player_path: String,
    show_resolution: String,
    feed_url: String,
    current_season_url: String,
}

#[derive(Debug)]
struct Show {
    id: u32,
    title: String,
    subscribed: u8,
    episodes: Option<HashMap<String, Episode>>,
}

#[derive(Clone, Debug)]
struct Episode {
    id: u32,
    show_id: u32,
    title: String,
    episode: String,
    watched: u8,
    resolution: String,
    torrent_link: String,
}

impl Show {
    fn add_show(&mut self, ep: Episode) {
        if ep.id < 1 {
            return;
        }

        match self.episodes {
            Some(ref mut episodes) => {
                episodes.insert(ep.title.clone(), ep);
            }
            _ => self.episodes = Some([(ep.title.clone(), ep)].iter().cloned().collect()),
        }
    }
}

fn main() {
    let config_str: String = fs::read_to_string("config.toml").unwrap();
    let config: UserConfig = toml::from_str(&config_str).unwrap();

    println!("{}", style("[LOADING..]").bold().on_red());

    let sql_conn = Connection::open("main.db").unwrap();

    match initialize_data(&sql_conn, &config.current_season_url) {
        Ok(_) => {}
        Err(e) => println!("{}", e),
    }

    let config_clone = config.clone();

    thread::spawn(move || {
        watch_feed(&config_clone);
    });

    println!("{}", style("[LOADING COMPLETE]").bold().on_green());

    loop {
        handle_user(&sql_conn, &config);
    }
}

fn handle_user(sql_conn: &Connection, user_config: &UserConfig) {
    let selections = &[
        "Available Subscriptions",
        "My Subscriptions",
        "New Episodes",
    ];

    match prompt_menu(selections) {
        0 => handle_available_subscriptions(&sql_conn, &user_config.show_resolution),
        1 => handle_my_subscriptions(&sql_conn, &user_config),
        2 => handle_new_episodes(&sql_conn, &user_config),
        _ => println!("Fail"),
    }
}

fn prompt_menu(selections: &[&'static str]) -> usize {
    Select::with_theme(&ColorfulTheme::default())
        .with_prompt("Choose an option")
        .default(0)
        .items(&selections)
        .interact()
        .unwrap()
}

fn handle_available_subscriptions(sql_conn: &Connection, user_resolution: &str) {
    let shows = fetch_shows(&sql_conn).unwrap();

    if shows.keys().count() == 0 {
        println!("{}", style("No available subscriptions").bold().red());
        return;
    }

    let mut titles: Vec<&str> = shows
        .values()
        .filter_map(|show| {
            if show.subscribed == 0 {
                Some(show.title.as_ref())
            } else {
                None
            }
        })
        .collect();
    titles.sort();
    let titles: &[&str] = &titles;

    let checks = Checkboxes::with_theme(&ColorfulTheme::default())
        .with_prompt("Select subscriptions")
        .items(&titles)
        .interact()
        .unwrap();

    for check in checks {
        let selected_show = &shows[titles[check]];

        subscribe_to_show(sql_conn, selected_show.id);

        println!(
            "{}{}{}",
            style("[SUBSCRIBED TO:").bold().magenta(),
            style(&selected_show.title).bold().dim(),
            style("]").bold().magenta()
        );

        persist_new_episodes(
            &sql_conn,
            fetch_episodes_from_feed(&selected_show.title).unwrap(),
            true,
            user_resolution,
        );
    }
}

fn handle_my_subscriptions(sql_conn: &Connection, user_config: &UserConfig) {
    let mut show_titles = fetch_subscribed_titles(&sql_conn);

    if show_titles.is_empty() {
        println!(
            "{}",
            style("No shows found! Please subscribe first").bold().red()
        );

        return;
    }

    show_titles.sort();
    show_titles.push(BACK_SELECTION.to_string());

    loop {
        let show_selection = Select::with_theme(&ColorfulTheme::default())
            .with_prompt("Choose a show")
            .default(0)
            .items(&show_titles[..])
            .interact()
            .unwrap();

        if show_titles[show_selection] == BACK_SELECTION {
            return;
        }

        handle_episodes(&show_titles[show_selection], &sql_conn, &user_config)
    }
}

fn handle_episodes(show_title: &str, sql_conn: &Connection, user_config: &UserConfig) {
    let episodes = fetch_episodes_for_show(sql_conn, show_title, &user_config.show_resolution);
    let mut available_selections: Vec<String> = episodes
        .iter()
        .map(|ep| format!("{} - {}", ep.title, ep.episode))
        .collect();
    available_selections.sort();
    available_selections.push(BACK_SELECTION.to_string());

    let selection = Select::with_theme(&ColorfulTheme::default())
        .with_prompt("Choose an episode")
        .default(0)
        .items(&available_selections[..])
        .interact()
        .unwrap();

    if available_selections[selection] == BACK_SELECTION {
        return;
    }

    let selected_episode = episodes
        .iter()
        .find(|ep| format!("{} - {}", ep.title, ep.episode) == available_selections[selection])
        .unwrap();

    println!("{:?}", selected_episode);

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
        Ok(_) => flag_episode_as_watched(&sql_conn, selected_episode.id),
        Err(error) => println!("{}", error),
    };
}

fn initialize_data(sql_conn: &Connection, current_season_url: &str) -> Result<(), Error> {
    initialize_sql_tables(&sql_conn)?;

    let titles = fetch_current_season_titles(current_season_url)?;

    for title in &titles {
        sql_conn.execute(
            "INSERT OR IGNORE INTO shows (title) VALUES (?1)",
            &[title.trim()],
        )?;
    }

    Ok(())
}

fn persist_new_episodes(
    sql_conn: &Connection,
    episodes: Vec<Episode>,
    watched: bool,
    user_resolution: &str,
) {
    let subscribed_titles = fetch_subscribed_titles(&sql_conn);
    let watched_value = if watched { "1" } else { "0" };

    for ep in episodes {
        if !subscribed_titles.contains(&ep.title.trim().to_string()) {
            continue;
        }

        let updated_rows = sql_conn
            .execute(
                "INSERT OR IGNORE INTO episodes (show_id, title, episode, resolution, torrent_link, watched)
                        VALUES ((SELECT id FROM shows WHERE title = ?1), ?1, ?2, ?3, ?4, ?5)",
                &[
                    &ep.title,
                    &ep.episode,
                    &ep.resolution,
                    &ep.torrent_link,
                    watched_value,
                ],
            )
            .unwrap();

        if updated_rows > 0 && ep.resolution == user_resolution {
            let announcement_text = if watched {
                style("[EPISODE ADDED:").magenta().dim()
            } else {
                style("[NEW EPISODE ARRIVAL:").magenta().dim()
            };

            println!(
                "{}{}{}{}{}",
                announcement_text,
                style(&ep.title).bold().dim(),
                style(" - ").bold().dim(),
                style(&ep.episode).bold().dim(),
                style("]").bold().magenta()
            );
        }
    }
}

fn initialize_sql_tables(conn: &Connection) -> Result<(), Error> {
    conn.execute(
        "CREATE TABLE IF NOT EXISTS shows (
                  id              INTEGER PRIMARY KEY AUTOINCREMENT,                  
                  title           TEXT NOT NULL UNIQUE,
                  subscribed      BOOLEAN DEFAULT 0
                  )",
        NO_PARAMS,
    )?;

    conn.execute(
        "CREATE TABLE IF NOT EXISTS episodes (
                  id              INTEGER PRIMARY KEY AUTOINCREMENT,
                  show_id         INTEGER NOT NULL,
                  title           TEXT NOT NULL,
                  episode         TEXT NOT NULL,
                  watched         BOOLEAN DEFAULT 0,
                  resolution      TEXT NOT NULL,
                  torrent_link    TEXT NOT NULL,
                  FOREIGN KEY(show_id) REFERENCES shows(id),
                  CONSTRAINT unique_episode UNIQUE (title, episode, resolution)
                  )",
        NO_PARAMS,
    )?;

    Ok(())
}

fn watch_feed(user_config: &UserConfig) {
    let sql_conn = Connection::open("main.db").unwrap();
    let mut current_feed_string = "".to_string();

    loop {
        let feed = Channel::from_url(&user_config.feed_url).expect("Unwrapping channel failed");
        let new_feed_string = feed.to_string();

        if current_feed_string != new_feed_string {
            // https://github.com/rust-syndication/rss/issues/74
            persist_new_episodes(
                &sql_conn,
                map_feed_to_episodes(&feed),
                false,
                &user_config.show_resolution,
            );
            current_feed_string = new_feed_string;
        }

        thread::sleep(std::time::Duration::from_secs(30));
    }
}

fn fetch_episodes_from_feed(show_title: &str) -> Result<Vec<Episode>, Error> {
    let feed_url = format!(
        "https://nyaa.si/?page=rss&q={}&c=0_0&f=0&u=HorribleSubs",
        show_title.replace(" ", "+")
    );
    let feed = Channel::from_url(&feed_url)?;
    let episodes = map_feed_to_episodes(&feed);

    Ok(episodes)
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
                show_id: 0,
                watched: 0,
                resolution: captures["resolution"].to_string(),
                torrent_link: item.link().unwrap().to_string(),
            }
        })
        .collect()
}

fn fetch_current_season_titles(url: &str) -> Result<Vec<String>, Error> {
    let doc = fetch_document(url).ok().unwrap();
    let mut title_selector = doc.select(".shows-wrapper").unwrap();
    let title_wrapper: NodeDataRef<ElementData> = title_selector.next().unwrap();
    let text_content = title_wrapper.text_contents();
    let titles: Vec<String> = text_content
        .replace("\u{2013}", "-")
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

fn fetch_shows(conn: &Connection) -> Result<HashMap<String, Show>, Error> {
    let mut shows: HashMap<String, Show> = HashMap::new();
    let mut current_episodes_select = conn.prepare(
        "SELECT s.id, s.title, s.subscribed, 
            COALESCE(e.id, 0),
            COALESCE(e.show_id, 0),
            COALESCE(e.title, 'NULL'),
            COALESCE(e.episode, 'NULL'),
            COALESCE(e.watched, 0),
            COALESCE(e.resolution, '480p'),
            COALESCE(e.torrent_link, 'NULL')
        FROM shows as s
            LEFT JOIN episodes as e ON s.id = e.show_id",
    )?;

    current_episodes_select
        .query_and_then(NO_PARAMS, |row| -> Result<(), rusqlite::Error> {
            let episode = Episode {
                id: row.get(3),
                show_id: row.get(4),
                title: row.get(5),
                episode: row.get(6),
                watched: row.get(7),
                resolution: row.get(8),
                torrent_link: row.get(9),
            };

            let show_title: String = row.get(1);

            if let Some(show) = shows.get_mut(&show_title) {
                show.add_show(episode)
            } else {
                shows.insert(
                    show_title.clone(),
                    Show {
                        id: row.get(0),
                        title: show_title,
                        subscribed: row.get(2),
                        episodes: Some(
                            [(
                                format!("{} - {}", episode.title.clone(), episode.episode),
                                episode,
                            )]
                            .iter()
                            .cloned()
                            .collect(),
                        ),
                    },
                );
            };

            Ok(())
        })
        .unwrap()
        .count();

    Ok(shows)
}

fn subscribe_to_show(sql_conn: &Connection, id: u32) {
    sql_conn
        .execute(
            "UPDATE shows
                SET subscribed = 1
                WHERE id = ?",
            &[id],
        )
        .unwrap();
}

fn fetch_subscribed_titles(sql_conn: &Connection) -> Vec<String> {
    let mut subscribed_shows_select = sql_conn
        .prepare("SELECT title FROM shows WHERE subscribed = 1")
        .unwrap();

    let titles = subscribed_shows_select
        .query_map(NO_PARAMS, |row| -> String { row.get(0) })
        .unwrap();

    let mut empty_vec: Vec<String> = vec![];

    for title in titles {
        empty_vec.push(title.unwrap().trim().to_string());
    }

    empty_vec
}

fn flag_episode_as_watched(sql_conn: &Connection, episode_id: u32) {
    sql_conn
        .execute(
            "UPDATE episodes
                    SET watched = 1
                    WHERE id = ?",
            &[episode_id],
        )
        .unwrap();
}

fn fetch_episodes_for_show(
    sql_conn: &Connection,
    show_title: &str,
    user_resolution: &str,
) -> Vec<Episode> {
    let mut new_episodes_select = sql_conn
        .prepare(
            "SELECT e.id, e.show_id, e.title, e.episode, e.watched, e.resolution, e.torrent_link
                FROM shows AS s
	                JOIN episodes as e ON s.id = e.show_id 
	            WHERE s.subscribed = 1
                    AND s.title = ?1
                    AND e.resolution = ?2",
        )
        .unwrap();

    new_episodes_select
        .query_map(&[show_title, user_resolution], |row| -> Episode {
            Episode {
                id: row.get(0),
                show_id: row.get(1),
                title: row.get(2),
                episode: row.get(3),
                watched: row.get(4),
                resolution: row.get(5),
                torrent_link: row.get(6),
            }
        })
        .unwrap()
        .map(|ep| ep.unwrap())
        .collect()
}

fn fetch_new_episodes(sql_conn: &Connection) -> Vec<Episode> {
    let mut new_episodes_select = sql_conn
        .prepare(
            "SELECT e.id, e.show_id, e.title, e.episode, e.watched, e.resolution, e.torrent_link
                FROM shows AS s
	                JOIN episodes as e ON s.id = e.show_id 
	            WHERE s.subscribed = 1
		            AND e.watched = 0",
        )
        .unwrap();

    new_episodes_select
        .query_map(NO_PARAMS, |row| -> Episode {
            Episode {
                id: row.get(0),
                show_id: row.get(1),
                title: row.get(2),
                episode: row.get(3),
                watched: row.get(4),
                resolution: row.get(5),
                torrent_link: row.get(6),
            }
        })
        .unwrap()
        .map(|ep| ep.unwrap())
        .collect()
}

fn handle_new_episodes(sql_conn: &Connection, user_config: &UserConfig) {
    let new_episodes = fetch_new_episodes(&sql_conn);

    if new_episodes.is_empty() {
        println!(
            "{}",
            style("No new episodes found :( Patience you must")
                .bold()
                .red()
        );
        return;
    }

    let mut episode_titles: Vec<String> = new_episodes
        .iter()
        .map(|ep| format!("{} - {}", ep.title, ep.episode))
        .collect();
    episode_titles.push(BACK_SELECTION.to_string());

    let episode_selection = Select::with_theme(&ColorfulTheme::default())
        .with_prompt("Choose a show")
        .default(0)
        .items(&episode_titles[..])
        .interact()
        .unwrap();

    if episode_titles[episode_selection] == BACK_SELECTION {
        return;
    }

    let selected_episode = new_episodes
        .iter()
        .find(|ep| format!("{} - {}", ep.title, ep.episode) == episode_titles[episode_selection])
        .unwrap();

    println!(
        "{}{}{}",
        style("[LOADING EPISODE:").bold().magenta(),
        style(&selected_episode.title).bold().dim(),
        style("]").bold().magenta()
    );

    match open_episode(
        selected_episode.torrent_link.clone(),
        user_config.player_path.to_string(),
    ) {
        Ok(_) => flag_episode_as_watched(&sql_conn, selected_episode.id),
        Err(error) => println!("{}", error),
    };
}

fn capture_variables_from_title(feed_item: &rss::Item) -> regex::Captures<'_> {
    let regex = Regex::new("\\[(?P<subber>.+)\\]\\s*(?P<title>.+?)[\\s*\\-*]+(?P<episode>\\d{1,4}.\\d*)[\\s*]\\[(?P<resolution>\\d{3,4}p)\\](?P<version>v*\\d*)").unwrap();

    regex.captures(feed_item.title().unwrap()).unwrap()
}
