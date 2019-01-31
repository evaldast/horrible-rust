extern crate dialoguer;
extern crate kuchiki;
extern crate regex;
extern crate reqwest;
extern crate rss;
extern crate rusqlite;
extern crate tempdir;

use dialoguer::{theme::ColorfulTheme, theme::SelectionStyle, Checkboxes, Select};
use failure::Error;
use kuchiki::traits::*;
use kuchiki::*;
use regex::Regex;
use rss::Channel;
use rusqlite::{Connection, NO_PARAMS};
use std::collections::HashMap;
use std::fs::File;
use std::io;
use std::path;
use std::process::Command;
use tempdir::TempDir;

#[macro_use]
extern crate failure;

#[derive(Debug)]
struct Show {
    id: Option<u32>,
    title: String,
    subscribed: Option<u8>,
    episodes: Option<Vec<Episode>>,
}

#[derive(Debug)]
enum Resolution {
    Q480p,
    Q720p,
    Q1080p,
}

#[derive(Debug)]
struct Episode {
    id: Option<u32>,
    show_id: Option<u32>,
    title: String,
    watched: Option<u8>,
    resolution: Resolution,
    torrent_link: String,
}

impl Show {
    fn add_show(&mut self, ep: Episode) {
        if ep.title == "NULL" {
            return;
        }

        match self.episodes {
            Some(ref mut episodes) => episodes.push(ep),
            _ => self.episodes = Some(vec![ep]),
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
    let sql_conn = Connection::open("main.db").unwrap();
    //let sql_conn = Connection::open_in_memory()?;

    match load_data(&sql_conn) {
        Ok(_) => println!("Load success"),
        Err(e) => println!("{}", e),
    }

    let shows = load_shows(&sql_conn).unwrap();

    let values: Vec<&Show> = shows
        .values()
        .filter(|show| show.subscribed == Some(1))
        .collect();

    for value in values {
        println!("{:?}", value)
    }

    //prompt_menu(all_shows);

    handle_user();
}

fn handle_user() {
    let selections = &[
        "Available Subscriptions",
        "My Subscriptions",
        "New Episodes",
    ];

    match prompt_menu(selections) {
        0 => prompt_menu_current_season(),
        1 => println!("{}", selections[1]),
        2 => println!("{}", selections[2]),
        _ => println!("Fail"),
    }
}

fn prompt_menu(selections: &[&'static str]) -> usize {
    Select::with_theme(&ColorfulTheme::default())
        .with_prompt("Choose an option")
        .default(0)
        .items(&selections[..])
        .interact()
        .unwrap()
}

fn prompt_menu_current_season() {
    let current_shows = fetch_current_season_titles().unwrap();
    let current_shows: Vec<&str> = current_shows.iter().map(AsRef::as_ref).collect();
    let current_shows: &[&str] = &current_shows;

    let checks = Checkboxes::with_theme(&ColorfulTheme::default())
        .with_prompt("Pick your subscriptions")
        .items(&current_shows[..])
        .interact()
        .unwrap();

    for check in checks {
        println! {"{}", current_shows[check]};
    }
}

fn load_data(sql_conn: &Connection) -> Result<(), Error> {
    initialize_sql_tables(&sql_conn)?;

    let download_dir = TempDir::new("horrible_rust")?;
    let titles = fetch_current_season_titles()?;

    for title in titles {
        sql_conn.execute("INSERT OR IGNORE INTO shows (title) VALUES (?1)", &[title])?;
    }

    let episodes = fetch_episodes()?;

    for ep in episodes {
        sql_conn.execute(
            "INSERT OR IGNORE INTO episodes (show_id, title, resolution, torrent_link)
                        VALUES ((SELECT id FROM shows WHERE title = ?1), ?2, ?3, ?4)",
            &[
                &parse_show_name_from_torrent_title(&ep.title)?,
                &ep.title,
                parse_resolution_from_title(&ep.title)?.to_str().unwrap(),
                &ep.torrent_link,
            ],
        )?;
    }

    Ok(())
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

    Ok(())
}

fn fetch_episodes() -> Result<Vec<Episode>, Error> {
    let channel = Channel::from_url("https://nyaa.si/?page=rss&c=0_0&f=0&u=HorribleSubs")?;
    let episodes = channel
        .items()
        .iter()
        .map(|i| Episode {
            id: None,
            title: i.title().unwrap().to_string(),
            show_id: None,
            watched: None,
            resolution: parse_resolution_from_title(i.title().unwrap()).unwrap(),
            torrent_link: i.link().unwrap().to_string(),
        })
        .collect();

    Ok(episodes)
}

fn parse_resolution_from_title(title: &str) -> Result<Resolution, Error> {
    let regex = Regex::new(r"(?x)(?P<res>([0-1][0-8][0-8][0]p|[4-7][2-8][0]p))")?;
    let captures = regex.captures(title).unwrap();

    return Resolution::from_string(captures["res"].to_string());
}

fn parse_show_name_from_torrent_title(title: &str) -> Result<String, Error> {
    let horrible_subs_prefix = "[HorribleSubs]";
    let prefix_length = horrible_subs_prefix.chars().count();

    return Ok(title[prefix_length..].split('-').collect::<Vec<&str>>()[0]
        .trim()
        .to_string());
}

fn fetch_current_season_titles() -> Result<Vec<String>, Error> {
    let doc = fetch_document("https://horriblesubs.info/current-season/")
        .ok()
        .unwrap();
    let title_selector = doc.select(".shows-wrapper").unwrap();
    let title_wrapper: NodeDataRef<ElementData> = title_selector.into_iter().next().unwrap();
    let text_content = title_wrapper.text_contents();
    let titles: Vec<String> = text_content
        .lines()
        .filter(|t| !t.is_empty())
        .map(|s| s.to_string())
        .collect();

    Ok(titles)
}

fn fetch_document(url: &str) -> Result<NodeRef, Error> {
    let mut res = reqwest::get(url)?;

    Ok(kuchiki::parse_html().one(res.text()?))
}

fn download_torrent(directory: &TempDir, url: &str) -> Result<path::PathBuf, Error> {
    let mut response = reqwest::get(url)?;
    let file_name = response
        .url()
        .path_segments()
        .and_then(|segments| segments.last())
        .and_then(|name| if name.is_empty() { None } else { Some(name) })
        .unwrap_or("tmp.bin");

    println!("file to download: '{}'", file_name);

    let file_path = directory.path().join(&file_name);

    println!("will be located under: '{:?}'", &file_path);

    let mut file = File::create(&file_path)?;

    io::copy(&mut response, &mut file)?;

    println!("Done");

    Ok(file_path)
}

fn open_episode(player_path: &str, torrent_path: &str) {
    let t = Command::new("C:\\Users\\snow\\AppData\\Local\\sodaplayer\\Soda Player.exe")
        .args(&[torrent_path])
        .output()
        .expect("failed to execute process");
}

fn load_shows(conn: &Connection) -> Result<HashMap<String, Show>, Error> {
    let mut shows: HashMap<String, Show> = HashMap::new();
    let mut current_episodes_select = conn.prepare(
        "SELECT s.title, s.subscribed, COALESCE(e.title, 'NULL'), COALESCE(e.watched, 0), COALESCE(e.resolution, '480p'), COALESCE(e.torrent_link, 'NULL')
            FROM shows as s
            LEFT JOIN episodes as e ON s.id = e.show_id",
    )?;

    current_episodes_select
        .query_and_then(NO_PARAMS, |row| -> Result<(), rusqlite::Error> {
            let episode = Episode {
                id: None,
                show_id: None,
                title: row.get(2),
                watched: row.get(3),
                resolution: Resolution::from_string(row.get(4)).unwrap(),
                torrent_link: row.get(5),
            };
            let show_title: String = row.get(0);

            match shows.get_mut(&show_title) {
                Some(show) => show.add_show(episode),
                None => {
                    shows.insert(
                        show_title.clone(),
                        Show {
                            title: show_title,
                            subscribed: row.get(1),
                            episodes: Some(vec![episode]),
                            id: None,
                        },
                    );
                }
            };

            Ok(())
        })
        .unwrap()
        .count();

    return Ok(shows);
}
