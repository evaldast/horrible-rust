extern crate kuchiki;
extern crate regex;
extern crate reqwest;
extern crate rss;
extern crate rusqlite;
extern crate tempdir;

use failure::Error;
use kuchiki::traits::*;
use kuchiki::*;
use regex::Regex;
use rss::Channel;
use rusqlite::types::ToSql;
use rusqlite::{Connection, NO_PARAMS};
use std::env;
use std::fs::File;
use std::io::{self, copy};
use std::path;
use std::process::Command;
use std::thread;
use tempdir::TempDir;

#[macro_use]
extern crate failure;

#[derive(Debug)]
struct Anime {
    title: String,
    subscribed: Option<u8>,
    episodes: Vec<Episode>,
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
    title: String,
    watched: Option<u8>,
    resolution: Resolution,
    torrent_link: String,
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
    match process() {
        Ok(_) => println!("Success"),
        Err(e) => println!("{}", e),
    }
}

fn process() -> Result<String, Error> {
    let download_dir = TempDir::new("horrible_rust")?;
    // let sql_conn = Connection::open_in_memory()?;

    // download_torrent(&download_dir, "https://nyaa.si/download/1114113.torrent");

    let sql_conn = Connection::open_in_memory()?;
    // let sql_conn = Connection::open("main.db")?;

    initialize_sql_tables(&sql_conn)?;

    let episodes = fetch_episodes()?;

    for ep in episodes {
        sql_conn.execute(
            "INSERT INTO episodes (title, resolution, torrent_link)
                  VALUES (?1, ?2, ?3)",
            &[
                &ep.title as &ToSql,
                &ep.resolution.to_str().unwrap(),
                &ep.torrent_link,
            ],
        )?;
    }

    let mut current_episodes_select =
        sql_conn.prepare("SELECT id, title, watched, resolution, torrent_link FROM episodes")?;
    let episode_iter = current_episodes_select.query_map(NO_PARAMS, |row| Episode {
        id: row.get(0),
        title: row.get(1),
        watched: row.get(2),
        resolution: Resolution::from_string(row.get(3)).unwrap(),
        torrent_link: row.get(4),
    })?;

    for episode in episode_iter {
        let episode = episode?;
        println!("Found episode {:?}", episode);
        // download_torrent(&download_dir, &episode.torrent_link);
    }

    
    let mut input = String::new();
    io::stdin().read_line(&mut input);

    println!("your input was: {}", input);

    let strng = "https://nyaa.si/download/1113607.torrent".to_string();

    let mut wanted_episode_select =
        sql_conn.prepare("SELECT id, title, watched, resolution, torrent_link FROM episodes WHERE torrent_link = ?")?;
    let wanted_episode_iter = wanted_episode_select.query_map(&[input.trim()], |row| Episode {
        id: row.get(0),
        title: row.get(1),
        watched: row.get(2),
        resolution: Resolution::from_string(row.get(3)).unwrap(),
        torrent_link: row.get(4),
    })?;

    for episode in wanted_episode_iter {
        let episode = episode?;
        println!("Found wanted episode {:?}", episode);
        // download_torrent(&download_dir, &episode.torrent_link);
    }

    // for title in fetch_current_season_titles()? {
    //     println!("Found title {:?}", title);
    // }


    return Ok("".to_string());
}

fn initialize_sql_tables(conn: &Connection) -> Result<(), Error> {
    conn.execute(
        "CREATE TABLE episodes (
                  id              INTEGER PRIMARY KEY AUTOINCREMENT,
                  title           TEXT NOT NULL,
                  watched         BOOLEAN DEFAULT 0,
                  resolution      TEXT NOT NULL,
                  torrent_link    TEXT NOT NULL
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
    // thread::spawn(move || {
    //     Command::new("C:\\Users\\snow\\AppData\\Local\\sodaplayer\\Soda Player.exe")
    //         .output()
    //         .expect("failed to execute process")
    // });
}
