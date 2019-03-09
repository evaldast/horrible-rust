use crate::ui;
use crate::Config;
use failure::Error;
use rusqlite::{Connection, NO_PARAMS};
use std::cmp::Ordering;

pub struct Show {
    pub id: u32,
    pub title: String,
}

#[derive(Clone, Eq)]
pub struct Episode {
    pub id: u32,
    pub show_id: u32,
    pub title: String,
    pub episode: String,
    pub version: String,
    pub watched: u8,
    pub resolution: String,
    pub torrent_link: String,
}

impl Ord for Episode {
    fn cmp(&self, other: &Self) -> Ordering {
        self.episode.cmp(&other.episode)
    }
}

impl PartialOrd for Episode {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl PartialEq for Episode {
    fn eq(&self, other: &Self) -> bool {
        self.formatted_title() == other.formatted_title()
    }
}

impl Episode {
    pub fn formatted_title(&self) -> String {
        format!("{} - {}{}", self.title, self.episode, self.version)
    }
}

pub fn persist_new_episodes(
    sql_conn: &Connection,
    episodes: Vec<Episode>,
    watched: bool,
    user_resolution: &str,
) -> Result<(), Error> {
    let subscribed_shows = fetch_shows(&sql_conn, true)?;
    let watched_value = if watched { "1" } else { "0" };

    for ep in episodes {
        match subscribed_shows
            .iter()
            .find(|&show| show.title == ep.title.trim())
        {
            Some(_) => {}
            None => continue,
        };

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
            )?;

        if updated_rows > 0 && ep.resolution == user_resolution {
            ui::announce_new_episode(&ep, watched);
        }
    }

    Ok(())
}

pub fn initialize_sql_tables(conn: &Connection) -> Result<(), Error> {
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
                  version         TEXT NOT NULL DEFAULT '',
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

pub fn subscribe_to_show(sql_conn: &Connection, id: u32) -> Result<(), Error> {
    sql_conn.execute(
        "UPDATE shows
                SET subscribed = 1
                WHERE id = ?",
        &[id],
    )?;

    Ok(())
}

pub fn fetch_shows(sql_conn: &Connection, subscribed: bool) -> Result<Vec<Show>, rusqlite::Error> {
    sql_conn
        .prepare("SELECT id, title, subscribed FROM shows WHERE subscribed = ?")?
        .query_map(&[subscribed], |row| -> Show {
            Show {
                id: row.get(0),
                title: row.get(1),
            }
        })?
        .collect()
}

pub fn flag_episode_as_watched(
    sql_conn: &Connection,
    episode_id: u32,
) -> Result<usize, rusqlite::Error> {
    sql_conn.execute(
        "UPDATE episodes
                    SET watched = 1
                    WHERE id = ?",
        &[episode_id],
    )
}

pub fn fetch_episodes_for_show(
    sql_conn: &Connection,
    show_title: &str,
    user_resolution: &str,
) -> Result<Vec<Episode>, rusqlite::Error> {
    sql_conn
        .prepare(
            "SELECT e.id, e.show_id, e.title, e.episode, e.version, e.watched, e.resolution, e.torrent_link
                FROM shows AS s
	                JOIN episodes as e ON s.id = e.show_id 
	            WHERE s.subscribed = 1
                    AND s.title = ?1
                    AND e.resolution = ?2",
        )?
        .query_map(&[show_title, user_resolution], |row| -> Episode {
            Episode {
                id: row.get(0),
                show_id: row.get(1),
                title: row.get(2),
                episode: row.get(3),
                version: row.get(4),
                watched: row.get(5),
                resolution: row.get(6),
                torrent_link: row.get(7),
            }
        })?
        .collect()
}

pub fn fetch_new_episodes(
    sql_conn: &Connection,
    config: &Config,
) -> Result<Vec<Episode>, rusqlite::Error> {
    sql_conn
        .prepare(
            "SELECT e.id, e.show_id, e.title, e.episode, e.version, e.watched, e.resolution, e.torrent_link
                FROM shows AS s
	                JOIN episodes as e ON s.id = e.show_id 
	            WHERE s.subscribed = 1
		            AND e.watched = 0
                    AND e.resolution = ?",
        )?
        .query_map(&[&config.show_resolution], |row| -> Episode {
            Episode {
                id: row.get(0),
                show_id: row.get(1),
                title: row.get(2),
                episode: row.get(3),
                version: row.get(4),
                watched: row.get(5),
                resolution: row.get(6),
                torrent_link: row.get(7),
            }
        })?
        .collect()
}

pub fn insert_show_titles(sql_conn: &Connection, titles: &Vec<String>) -> Result<(), Error> {
    for title in titles {
        sql_conn.execute(
            "INSERT OR IGNORE INTO shows (title) VALUES (?1)",
            &[title.trim()],
        )?;
    }

    Ok(())
}
