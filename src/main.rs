#![deny(clippy::all)]
#![deny(clippy::pedantic)]

extern crate console;
extern crate dialoguer;
extern crate kuchiki;
extern crate regex;
extern crate reqwest;
extern crate rss;
extern crate rusqlite;

use console::style;
use dialoguer::{theme::ColorfulTheme, Checkboxes, Confirmation, Select};
use failure::Error;
use kuchiki::traits::*;
use kuchiki::*;
use regex::Regex;
use rss::Channel;
use rusqlite::{Connection, NO_PARAMS};
use std::collections::HashMap;
use std::process::Command;
use std::thread;

#[macro_use]
extern crate failure;

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
    watched: u8,
    resolution: Resolution,
    torrent_link: String,
}

#[derive(Clone, Debug)]
enum Resolution {
    Q480p,
    Q720p,
    Q1080p,
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

trait Str {
    fn to_str(&self) -> Result<&'static str, ()>;
    fn from_string(s: String) -> Result<Resolution, Error>;
}

impl Str for Resolution {
    fn to_str(&self) -> Result<&'static str, ()> {
        match self {
            Resolution::Q480p => Ok("480p"),
            Resolution::Q720p => Ok("720p"),
            Resolution::Q1080p => Ok("1080p"),
        }
    }

    fn from_string(s: String) -> Result<Resolution, Error> {
        match s.as_ref() {
            "480p" => Ok(Resolution::Q480p),
            "720p" => Ok(Resolution::Q720p),
            "1080p" => Ok(Resolution::Q1080p),
            _ => bail!("Resolution::from_string failed"),
        }
    }
}

fn main() {
    println!("{}", style("[LOADING..]").bold().magenta());

    let sql_conn = Connection::open("main.db").unwrap();
    //let sql_conn = Connection::open_in_memory()?;

    match initialize_data(&sql_conn) {
        Ok(_) => {}
        Err(e) => println!("{}", e),
    }

    update_last_boot_date(&sql_conn);

    thread::spawn(|| {
        watch_feed();
    });

    loop {
        let shows = load_shows(&sql_conn).unwrap();
        handle_user(&shows, &sql_conn);
    }
}

fn update_last_boot_date(sql_conn: &Connection) {
    sql_conn
        .execute(
            "INSERT OR REPLACE INTO settings(id, last_boot_date) VALUES(1 ,date())",
            NO_PARAMS,
        )
        .unwrap();
}

fn handle_user(shows: &HashMap<String, Show>, sql_conn: &Connection) {
    let selections = &[
        "Available Subscriptions",
        "My Subscriptions",
        "New Episodes",
    ];

    match prompt_menu(selections) {
        0 => handle_available_subscriptions(&shows, &sql_conn),
        1 => handle_my_subscriptions(&shows, &sql_conn),
        2 => handle_new_episodes(&sql_conn),
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

fn handle_available_subscriptions(shows: &HashMap<String, Show>, sql_conn: &Connection) {
    if shows.values().count() == 0 { println!("{}", style("No available subscriptions").bold().red()); return }

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
        subscribe_to_show(&sql_conn, selected_show.id);
        println!(
            "{}{}{}",
            style("[SUBSCRIBED TO:").bold().magenta(),
            style(&selected_show.title).bold().dim(),
            style("]").bold().magenta()
        );
        persist_new_episodes(
            &sql_conn,
            fetch_episodes(&selected_show.title).unwrap(),
            true,
        );
    }
}

fn handle_my_subscriptions(shows: &HashMap<String, Show>, sql_conn: &Connection) {
    let mut show_titles: Vec<&str> = shows
        .values()
        .filter_map(|show| {
            if show.subscribed == 1 {
                Some(show.title.as_ref())
            } else {
                None
            }
        })
        .collect();

    if show_titles.is_empty() { println!("{}", style("No shows found! Please subscribe first").bold().red()); return }
    
    show_titles.sort();
    let show_titles: &[&str] = &show_titles;

    let show_selection = Select::with_theme(&ColorfulTheme::default())
        .with_prompt("Choose a show")
        .default(0)
        .items(&show_titles[..])
        .interact()
        .unwrap();

    let empty_hashmap: HashMap<String, Episode> = HashMap::new();
    let show = shows.get(show_titles[show_selection]).unwrap();
    let episodes = &show.episodes;
    let episodes = match episodes {
        Some(eps) => eps,
        None => &empty_hashmap,
    };
    let mut episode_titles: Vec<&str> = episodes.keys().map(AsRef::as_ref).collect();
    episode_titles.sort();
    let episode_titles: &[&str] = &episode_titles;

    let episode_selection = Select::with_theme(&ColorfulTheme::default())
        .with_prompt("Choose an episode")
        .default(0)
        .items(&episode_titles[..])
        .interact()
        .unwrap();

    let selected_episode = episodes.get(episode_titles[episode_selection]).unwrap();

    println!(
        "{}{}{}",
        style("[LOADING EPISODE:").bold().magenta(),
        style(&selected_episode.title).bold().dim(),
        style("]").bold().magenta()
    );

    match open_episode(selected_episode.torrent_link.clone()) {
        Ok(_) => {
            if prompt_mark_episode_as_watched() {
                flag_episode_as_watched(&sql_conn, selected_episode.id)
            }
        }
        Err(error) => println!("{}", error),
    };
}

fn prompt_mark_episode_as_watched() -> bool {
    Confirmation::new()
        .with_text("Mark episode as watched?")
        .interact()
        .unwrap()
}

fn initialize_data(sql_conn: &Connection) -> Result<(), Error> {
    initialize_sql_tables(&sql_conn)?;

    let titles = fetch_current_season_titles()?;

    for title in &titles {
        sql_conn.execute(
            "INSERT OR IGNORE INTO shows (title) VALUES (?1)",
            &[title.trim()],
        )?;
    }

    Ok(())
}

fn persist_new_episodes(sql_conn: &Connection, episodes: Vec<Episode>, watched: bool) {
    let subscribed_titles = fetch_subscribed_titles(&sql_conn);
    let watched_value = if watched { "1" } else { "0" };

    for ep in episodes {
        if !subscribed_titles.contains(&parse_show_name_from_torrent_title(&ep.title).unwrap()) {
            continue;
        }

        let updated_rows = sql_conn
            .execute(
                "INSERT OR IGNORE INTO episodes (show_id, title, resolution, torrent_link, watched)
                        VALUES ((SELECT id FROM shows WHERE title = ?1), ?2, ?3, ?4, ?5)",
                &[
                    &parse_show_name_from_torrent_title(&ep.title).unwrap(),
                    &ep.title,
                    parse_resolution_from_title(&ep.title)
                        .unwrap()
                        .to_str()
                        .unwrap(),
                    &ep.torrent_link,
                    &watched_value,
                ],
            )
            .unwrap();

        if updated_rows > 0            
        {
            println!(
                "{}{}{}",
                style("[NEW EPISODE ARRIVAL:").magenta().dim(),
                style(&ep.title).bold().dim(),
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
                  title           TEXT NOT NULL UNIQUE,
                  watched         BOOLEAN DEFAULT 0,
                  resolution      TEXT NOT NULL,
                  torrent_link    TEXT NOT NULL,
                  FOREIGN KEY(show_id) REFERENCES shows(id)
                  )",
        NO_PARAMS,
    )?;

    conn.execute(
        "CREATE TABLE IF NOT EXISTS settings (
                  id              INTEGER PRIMARY KEY AUTOINCREMENT,
                  player_path     TEXT,
                  prefered_resolution TEXT,
                  last_boot_date  TEXT
                  )",
        NO_PARAMS,
    )?;

    Ok(())
}

fn watch_feed() {
    let sql_conn = Connection::open("main.db").unwrap();
    let mut current_feed_string = "".to_string();

    loop {
        let feed = Channel::from_url("https://nyaa.si/?page=rss&c=0_0&f=0&u=HorribleSubs").unwrap();
        let new_feed_string = feed.to_string();

        if current_feed_string != new_feed_string {
            // https://github.com/rust-syndication/rss/issues/74
            persist_new_episodes(&sql_conn, map_feed_to_episodes(&feed), false);
            current_feed_string = new_feed_string;
        }

        thread::sleep(std::time::Duration::from_secs(30));
    }
}

fn fetch_episodes(show_title: &str) -> Result<Vec<Episode>, Error> {
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
        .map(|i| Episode {
            id: 0,
            title: i.title().unwrap().to_string(),
            show_id: 0,
            watched: 0,
            resolution: parse_resolution_from_title(i.title().unwrap()).unwrap(),
            torrent_link: i.link().unwrap().to_string(),
        })
        .collect()
}

fn parse_resolution_from_title(title: &str) -> Result<Resolution, Error> {
    let regex = Regex::new(r"(?x)(?P<res>([0-1][0-8][0-8][0]p|[4-7][2-8][0]p))")?;
    let captures = regex.captures(title).unwrap();

    Resolution::from_string(captures["res"].to_string())
}

fn parse_show_name_from_torrent_title(title: &str) -> Result<String, Error> {
    let horrible_subs_prefix = "[HorribleSubs]";
    let prefix_length = horrible_subs_prefix.chars().count();

    Ok(
        title[prefix_length..].split(" - ").collect::<Vec<&str>>()[0]
            .trim()
            .to_string(),
    )
}

fn fetch_current_season_titles() -> Result<Vec<String>, Error> {
    let doc = fetch_document("https://horriblesubs.info/current-season/")
        .ok()
        .unwrap();
    let mut title_selector = doc.select(".shows-wrapper").unwrap();
    let title_wrapper: NodeDataRef<ElementData> = title_selector.next().unwrap();
    let text_content = title_wrapper.text_contents();
    let titles: Vec<String> = text_content
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

fn open_episode(torrent_path: String) -> Result<(), Error> {
    let child_thread = thread::spawn(|| {
        Command::new("C:\\Users\\snow\\AppData\\Local\\sodaplayer\\Soda Player.exe")
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

fn load_shows(conn: &Connection) -> Result<HashMap<String, Show>, Error> {
    let mut shows: HashMap<String, Show> = HashMap::new();
    let mut current_episodes_select = conn.prepare(
        "SELECT s.id, s.title, s.subscribed, 
            COALESCE(e.id, 0),
            COALESCE(e.show_id, 0),
            COALESCE(e.title, 'NULL'),
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
                watched: row.get(6),
                resolution: Resolution::from_string(row.get(7)).unwrap(),
                torrent_link: row.get(8),
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
                            [(episode.title.clone(), episode)].iter().cloned().collect(),
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
        empty_vec.push(title.unwrap());
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

fn fetch_new_episodes(sql_conn: &Connection) -> Vec<Episode> {
    let mut new_episodes_select = sql_conn
        .prepare(
            "SELECT e.id, e.show_id, e.title, e.watched, e.resolution, e.torrent_link
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
                watched: row.get(3),
                resolution: Resolution::Q480p,
                torrent_link: row.get(5),
            }
        })
        .unwrap()
        .map(|ep| ep.unwrap())
        .collect()
}

fn handle_new_episodes(sql_conn: &Connection) {
    let new_episodes = fetch_new_episodes(&sql_conn);

    if new_episodes.is_empty() { println!("{}", style("No new episodes found :( Patience you must").bold().red()); return }
    
    let episode_titles: Vec<&str> = new_episodes.iter().map(|ep| ep.title.as_ref()).collect();
    let episode_titles: &[&str] = &episode_titles;

    let episode_selection = Select::with_theme(&ColorfulTheme::default())
        .with_prompt("Choose a show")
        .default(0)
        .items(&episode_titles[..])
        .interact()
        .unwrap();

    let selected_episode = new_episodes
        .iter()
        .find(|ep| ep.title == episode_titles[episode_selection])
        .unwrap();

    println!(
        "{}{}{}",
        style("[LOADING EPISODE:").bold().magenta(),
        style(&selected_episode.title).bold().dim(),
        style("]").bold().magenta()
    );

    match open_episode(selected_episode.torrent_link.clone()) {
        Ok(_) => {
            if prompt_mark_episode_as_watched() {
                flag_episode_as_watched(&sql_conn, selected_episode.id)
            }
        }
        Err(error) => println!("{}", error),
    };
}
