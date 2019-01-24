extern crate rss;
extern crate rusqlite;

use rss::Channel;
use rusqlite::types::ToSql;
use rusqlite::{Connection, NO_PARAMS};

#[derive(Debug)]
struct Anime {
    title: String,
    episodes: Vec<Episode>,
}

#[derive(Debug)]
enum Resolution {
    Q480p,
    Q720p,
    Q1080p
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
            Resolution::Q720p => Ok("480p"),
            Resolution::Q1080p => Ok("480p"),
            _ => Err(()),
        }
    }

    fn from_str(s: String) -> Result<Resolution, ()> {
        println!("{}", s);

        match s.as_ref() {
            "480p" => Ok(Resolution::Q480p),
            "720p" => Ok(Resolution::Q720p),
            "1080p"=> Ok(Resolution::Q1080p),
            _ => Err(()),
        }
    }
}

fn main() {
    process()
}

fn process() {
    let conn = Connection::open_in_memory().unwrap();

    conn.execute(
        "CREATE TABLE episode (
                  id              INTEGER PRIMARY KEY AUTOINCREMENT,
                  title           TEXT NOT NULL,
                  watched         BOOLEAN DEFAULT 0,
                  resolution      TEXT NOT NULL,
                  torrent_link    TEXT NOT NULL
                  )",
        NO_PARAMS,
    )
    .unwrap();

    let channel = Channel::from_url("https://nyaa.si/?page=rss&c=0_0&f=0&u=HorribleSubs").unwrap();

    for item in channel.items() {
        let title = item.title().unwrap();

        let ep = Episode {
            id: None,
            title: title.to_string(),
            watched: None,
            resolution: parse_resolution_from_title(title),
            torrent_link: item.link().unwrap().to_string()            
        };

        conn.execute(
            "INSERT INTO episode (title, resolution, torrent_link)
                  VALUES (?1, ?2, ?3)",
            &[&ep.title as &ToSql, &ep.resolution.to_str().unwrap(), &ep.torrent_link]
        )
        .unwrap();
    }

    let mut stmt = conn
        .prepare("SELECT id, title, watched, resolution, torrent_link FROM episode")
        .unwrap();

    let episode_iter = stmt
        .query_map(NO_PARAMS, |row| Episode {
            id: row.get(0),
            title: row.get(1),
            watched: row.get(2),
            resolution: Resolution::from_str(row.get(3)).unwrap(),                        
            torrent_link: row.get(4),
        })
        .unwrap();

    for episode in episode_iter {
        println!("Found episode {:?}", episode.unwrap());
    }
}

fn parse_resolution_from_title(title: &str) -> Resolution {
    Resolution::Q480p
}
