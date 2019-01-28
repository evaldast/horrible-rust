extern crate failure;
extern crate kuchiki;
extern crate reqwest;
extern crate rss;
extern crate rusqlite;

use failure::Error;
use kuchiki::traits::*;
use kuchiki::*;
use rss::Channel;
use rusqlite::types::ToSql;
use rusqlite::{Connection, NO_PARAMS};

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
    fn from_str(s: String) -> Result<Resolution, ()>;
}

impl Str for Resolution {
    fn to_str(&self) -> Result<&'static str, ()> {
        match self {
            Resolution::Q480p => Ok("480p"),
            Resolution::Q720p => Ok("720p"),
            Resolution::Q1080p => Ok("1080p"),
            _ => Err(()),
        }
    }

    fn from_str(s: String) -> Result<Resolution, ()> {
        match s.as_ref() {
            "480p" => Ok(Resolution::Q480p),
            "720p" => Ok(Resolution::Q720p),
            "1080p" => Ok(Resolution::Q1080p),
            _ => Err(()),
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
    let conn = Connection::open("main.db")?;

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

    let channel = Channel::from_url("https://nyaa.si/?page=rss&c=0_0&f=0&u=HorribleSubs")?;

    for item in channel.items() {
        let title = item.title().unwrap();

        let ep = Episode {
            id: None,
            title: title.to_string(),
            watched: None,
            resolution: parse_resolution_from_title(title),
            torrent_link: item.link().unwrap().to_string(),
        };

        conn.execute(
            "INSERT INTO episodes (title, resolution, torrent_link)
                  VALUES (?1, ?2, ?3)",
            &[
                &ep.title as &ToSql,
                &ep.resolution.to_str().unwrap(),
                &ep.torrent_link,
            ],
        )?;
    }

    let mut episode_selection =
        conn.prepare("SELECT id, title, watched, resolution, torrent_link FROM episodes")?;
    let episode_iter = episode_selection.query_map(NO_PARAMS, |row| Episode {
        id: row.get(0),
        title: row.get(1),
        watched: row.get(2),
        resolution: Resolution::from_str(row.get(3)).unwrap(),
        torrent_link: row.get(4),
    })?;

    for episode in episode_iter {
        println!("Found episode {:?}", episode?);
    }

    for title in fetch_current_season_titles()? {
        println!("Found title {:?}", title);
    }

    return Ok("".to_string());
}

fn parse_resolution_from_title(title: &str) -> Resolution {
    Resolution::Q480p
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
