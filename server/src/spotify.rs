//! Spotify Web API metadata access for playlist/album/track → YouTube queue
//! conversion. This module only talks to Spotify; YouTube matching lives in
//! main.rs so it can share the global yt-dlp semaphore.

use base64::{engine::general_purpose, Engine as _};
use reqwest::header;
use serde::Deserialize;
use std::sync::Mutex;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpotifyKind {
    Playlist,
    Album,
    Track,
}

#[derive(Debug)]
pub struct SpotifyRef {
    pub kind: SpotifyKind,
    pub id: String,
}

#[derive(Debug, Clone)]
pub struct TrackMeta {
    pub name: String,
    pub artists: Vec<String>,
}

impl TrackMeta {
    pub fn display_title(&self) -> String {
        let artist = self.artists.join(", ");
        if artist.is_empty() {
            self.name.clone()
        } else {
            format!("{} - {}", artist, self.name)
        }
    }

    pub fn search_query(&self) -> String {
        let artist = self.artists.join(", ");
        if artist.is_empty() {
            format!("{} audio", self.name)
        } else {
            format!("{} - {} audio", artist, self.name)
        }
    }

    pub fn artist(&self) -> String {
        self.artists.join(", ")
    }
}

#[derive(Debug)]
pub struct SpotifySource {
    pub title: String,
    pub thumbnail: String,
    pub tracks: Vec<TrackMeta>,
}

pub fn is_spotify_url(input: &str) -> bool {
    parse_spotify_ref(input).is_some()
}

pub fn parse_spotify_ref(input: &str) -> Option<SpotifyRef> {
    let trimmed = input.trim();

    if let Some(rest) = trimmed.strip_prefix("spotify:") {
        let mut parts = rest.split(':');
        let kind = match parts.next()? {
            "playlist" => SpotifyKind::Playlist,
            "album" => SpotifyKind::Album,
            "track" => SpotifyKind::Track,
            _ => return None,
        };
        let id = parts.next()?.to_string();
        if is_valid_spotify_id(&id) {
            return Some(SpotifyRef { kind, id });
        }
        return None;
    }

    let parsed = url::Url::parse(trimmed).ok()?;
    let host = parsed.host_str()?.to_ascii_lowercase();
    if host != "open.spotify.com" {
        return None;
    }

    let segments: Vec<&str> = parsed
        .path_segments()
        .map(|segments| segments.collect())
        .unwrap_or_default();
    let type_index = segments
        .iter()
        .position(|part| matches!(*part, "playlist" | "album" | "track"))?;
    let kind = match segments[type_index] {
        "playlist" => SpotifyKind::Playlist,
        "album" => SpotifyKind::Album,
        "track" => SpotifyKind::Track,
        _ => return None,
    };
    let id = segments.get(type_index + 1)?.to_string();
    if !is_valid_spotify_id(&id) {
        return None;
    }

    Some(SpotifyRef { kind, id })
}

fn is_valid_spotify_id(id: &str) -> bool {
    id.len() >= 22 && id.chars().all(|c| c.is_ascii_alphanumeric())
}

fn credentials_error() -> String {
    "Spotify conversion requires WOPR_SPOTIFY_CLIENT_ID and WOPR_SPOTIFY_CLIENT_SECRET (or WOPR_SPOTIFY_ACCESS_TOKEN) on the server.".to_string()
}

// Client-credentials tokens last ~1h; cache one per process instead of
// re-authenticating on every conversion.
static TOKEN_CACHE: Mutex<Option<(String, Instant)>> = Mutex::new(None);

async fn access_token(client: &reqwest::Client) -> Result<String, String> {
    if let Ok(token) = std::env::var("WOPR_SPOTIFY_ACCESS_TOKEN") {
        let token = token.trim();
        if !token.is_empty() {
            return Ok(token.to_string());
        }
    }

    if let Some((token, expires_at)) = TOKEN_CACHE.lock().unwrap().clone() {
        if expires_at > Instant::now() {
            return Ok(token);
        }
    }

    let client_id = std::env::var("WOPR_SPOTIFY_CLIENT_ID").map_err(|_| credentials_error())?;
    let client_secret =
        std::env::var("WOPR_SPOTIFY_CLIENT_SECRET").map_err(|_| credentials_error())?;
    if client_id.trim().is_empty() || client_secret.trim().is_empty() {
        return Err(credentials_error());
    }

    #[derive(Deserialize)]
    struct TokenResponse {
        access_token: String,
        #[serde(default)]
        expires_in: u64,
    }

    let auth = general_purpose::STANDARD.encode(format!("{}:{}", client_id, client_secret));
    let response = client
        .post("https://accounts.spotify.com/api/token")
        .header(header::AUTHORIZATION, format!("Basic {}", auth))
        .form(&[("grant_type", "client_credentials")])
        .send()
        .await
        .map_err(|e| format!("Could not reach Spotify auth: {}", e))?;

    if !response.status().is_success() {
        return Err(format!(
            "Spotify auth failed with status {}.",
            response.status()
        ));
    }

    let token = response
        .json::<TokenResponse>()
        .await
        .map_err(|e| format!("Spotify auth returned invalid JSON: {}", e))?;

    let ttl = Duration::from_secs(token.expires_in.max(60).saturating_sub(60));
    *TOKEN_CACHE.lock().unwrap() = Some((token.access_token.clone(), Instant::now() + ttl));
    Ok(token.access_token)
}

async fn spotify_get<T: for<'de> Deserialize<'de>>(
    client: &reqwest::Client,
    token: &str,
    url: &str,
) -> Result<T, String> {
    let response = client
        .get(url)
        .bearer_auth(token)
        .send()
        .await
        .map_err(|e| format!("Could not reach Spotify: {}", e))?;

    if !response.status().is_success() {
        return Err(format!("Spotify returned status {}.", response.status()));
    }

    response
        .json::<T>()
        .await
        .map_err(|e| format!("Spotify returned invalid JSON: {}", e))
}

#[derive(Debug, Deserialize)]
struct SpotifyImage {
    url: String,
}

#[derive(Debug, Deserialize)]
struct SpotifyArtist {
    name: String,
}

#[derive(Debug, Deserialize)]
struct SpotifyTrack {
    name: String,
    #[serde(default)]
    artists: Vec<SpotifyArtist>,
}

impl From<SpotifyTrack> for TrackMeta {
    fn from(track: SpotifyTrack) -> Self {
        TrackMeta {
            name: track.name,
            artists: track.artists.into_iter().map(|a| a.name).collect(),
        }
    }
}

#[derive(Debug, Deserialize)]
struct PlaylistMeta {
    name: String,
    #[serde(default)]
    images: Vec<SpotifyImage>,
}

#[derive(Debug, Deserialize)]
struct PlaylistTrackItem {
    track: Option<SpotifyTrack>,
}

#[derive(Debug, Deserialize)]
struct PlaylistTracksPage {
    items: Vec<PlaylistTrackItem>,
    next: Option<String>,
}

#[derive(Debug, Deserialize)]
struct AlbumMeta {
    name: String,
    #[serde(default)]
    images: Vec<SpotifyImage>,
}

#[derive(Debug, Deserialize)]
struct AlbumTracksPage {
    items: Vec<SpotifyTrack>,
    next: Option<String>,
}

fn first_image(images: Vec<SpotifyImage>) -> String {
    images
        .into_iter()
        .next()
        .map(|image| image.url)
        .unwrap_or_default()
}

pub async fn fetch_source(client: &reqwest::Client, input: &str) -> Result<SpotifySource, String> {
    let spotify_ref = parse_spotify_ref(input).ok_or_else(|| "Not a Spotify URL".to_string())?;
    let token = access_token(client).await?;

    let (title, thumbnail, tracks) = match spotify_ref.kind {
        SpotifyKind::Playlist => playlist_tracks(client, &token, &spotify_ref.id).await?,
        SpotifyKind::Album => album_tracks(client, &token, &spotify_ref.id).await?,
        SpotifyKind::Track => track_item(client, &token, &spotify_ref.id).await?,
    };

    if tracks.is_empty() {
        return Err("Spotify did not return any playable tracks.".to_string());
    }

    Ok(SpotifySource {
        title,
        thumbnail,
        tracks,
    })
}

async fn playlist_tracks(
    client: &reqwest::Client,
    token: &str,
    id: &str,
) -> Result<(String, String, Vec<TrackMeta>), String> {
    let meta_url = format!(
        "https://api.spotify.com/v1/playlists/{}?fields=name,images(url)",
        id
    );
    let meta: PlaylistMeta = spotify_get(client, token, &meta_url).await?;
    let mut next = Some(format!(
        "https://api.spotify.com/v1/playlists/{}/tracks?limit=50&fields=next,items(track(name,artists(name)))",
        id
    ));
    let mut tracks = Vec::new();

    while let Some(url) = next {
        let page: PlaylistTracksPage = spotify_get(client, token, &url).await?;
        tracks.extend(
            page.items
                .into_iter()
                .filter_map(|item| item.track)
                .map(TrackMeta::from),
        );
        next = page.next;
    }

    Ok((meta.name, first_image(meta.images), tracks))
}

async fn album_tracks(
    client: &reqwest::Client,
    token: &str,
    id: &str,
) -> Result<(String, String, Vec<TrackMeta>), String> {
    let meta_url = format!("https://api.spotify.com/v1/albums/{}", id);
    let meta: AlbumMeta = spotify_get(client, token, &meta_url).await?;
    let mut next = Some(format!(
        "https://api.spotify.com/v1/albums/{}/tracks?limit=50",
        id
    ));
    let mut tracks = Vec::new();

    while let Some(url) = next {
        let page: AlbumTracksPage = spotify_get(client, token, &url).await?;
        tracks.extend(page.items.into_iter().map(TrackMeta::from));
        next = page.next;
    }

    Ok((meta.name, first_image(meta.images), tracks))
}

async fn track_item(
    client: &reqwest::Client,
    token: &str,
    id: &str,
) -> Result<(String, String, Vec<TrackMeta>), String> {
    #[derive(Debug, Deserialize)]
    struct TrackResponse {
        name: String,
        #[serde(default)]
        artists: Vec<SpotifyArtist>,
        album: Option<AlbumMeta>,
    }

    let url = format!("https://api.spotify.com/v1/tracks/{}", id);
    let track: TrackResponse = spotify_get(client, token, &url).await?;
    let thumbnail = track
        .album
        .map(|album| first_image(album.images))
        .unwrap_or_default();
    let meta = TrackMeta {
        name: track.name.clone(),
        artists: track.artists.into_iter().map(|a| a.name).collect(),
    };

    Ok((track.name, thumbnail, vec![meta]))
}

#[cfg(test)]
mod tests {
    use super::{parse_spotify_ref, SpotifyKind, TrackMeta};

    #[test]
    fn parses_open_spotify_playlist_urls() {
        let parsed =
            parse_spotify_ref("https://open.spotify.com/playlist/37i9dQZF1DXcBWIGoYBM5M?si=abc")
                .unwrap();
        assert_eq!(parsed.kind, SpotifyKind::Playlist);
        assert_eq!(parsed.id, "37i9dQZF1DXcBWIGoYBM5M");
    }

    #[test]
    fn parses_internationalized_urls_and_uris() {
        let album =
            parse_spotify_ref("https://open.spotify.com/intl-us/album/4aawyAB9vmqN3uQ7FjRGTy")
                .unwrap();
        assert_eq!(album.kind, SpotifyKind::Album);

        let track = parse_spotify_ref("spotify:track:11dFghVXANMlKmJXsNCbNl").unwrap();
        assert_eq!(track.kind, SpotifyKind::Track);
        assert_eq!(track.id, "11dFghVXANMlKmJXsNCbNl");
    }

    #[test]
    fn rejects_non_spotify_input() {
        assert!(parse_spotify_ref("https://www.youtube.com/watch?v=dQw4w9WgXcQ").is_none());
        assert!(parse_spotify_ref("spotify:episode:11dFghVXANMlKmJXsNCbNl").is_none());
        assert!(parse_spotify_ref("https://open.spotify.com/playlist/short").is_none());
    }

    #[test]
    fn builds_display_title_and_query() {
        let meta = TrackMeta {
            name: "Song".to_string(),
            artists: vec!["A".to_string(), "B".to_string()],
        };
        assert_eq!(meta.display_title(), "A, B - Song");
        assert_eq!(meta.search_query(), "A, B - Song audio");

        let bare = TrackMeta {
            name: "Song".to_string(),
            artists: vec![],
        };
        assert_eq!(bare.display_title(), "Song");
        assert_eq!(bare.search_query(), "Song audio");
    }
}
