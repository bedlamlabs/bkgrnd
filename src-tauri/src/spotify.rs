use base64::{engine::general_purpose, Engine as _};
use reqwest::header;
use serde::Deserialize;
use tauri::AppHandle;

use crate::ytdlp::{self, PlaylistItem};

#[derive(Debug)]
pub struct SpotifyQueue {
    pub items: Vec<PlaylistItem>,
    pub title: String,
    pub thumbnail: String,
    pub source_count: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SpotifyKind {
    Playlist,
    Album,
    Track,
}

pub fn is_spotify_url(input: &str) -> bool {
    parse_spotify_ref(input).is_some()
}

pub async fn enumerate(app: &AppHandle, input: &str) -> Result<SpotifyQueue, String> {
    let spotify_ref = parse_spotify_ref(input).ok_or_else(|| "Not a Spotify URL".to_string())?;
    let token = access_token().await?;
    let client = reqwest::Client::new();

    let (title, thumbnail, tracks) = match spotify_ref.kind {
        SpotifyKind::Playlist => playlist_tracks(&client, &token, &spotify_ref.id).await?,
        SpotifyKind::Album => album_tracks(&client, &token, &spotify_ref.id).await?,
        SpotifyKind::Track => track_item(&client, &token, &spotify_ref.id).await?,
    };

    if tracks.is_empty() {
        return Err("Spotify did not return any playable tracks.".to_string());
    }

    let max_tracks = spotify_track_limit();
    let source_count = tracks.len();
    let mut items = Vec::new();

    for track in tracks.into_iter().take(max_tracks) {
        let artist = track.artists.join(", ");
        let display_title = if artist.is_empty() {
            track.name.clone()
        } else {
            format!("{} - {}", artist, track.name)
        };
        let query = if artist.is_empty() {
            format!("{} audio", track.name)
        } else {
            format!("{} - {} audio", artist, track.name)
        };

        if let Some(result) = ytdlp::search_first_music(app, &query).await? {
            items.push(PlaylistItem {
                url: result.url,
                video_id: result.video_id,
                title: display_title,
                thumbnail: result.thumbnail,
                channel: artist,
                duration: result.duration,
            });
        }
    }

    if items.is_empty() {
        return Err("Could not find YouTube matches for this Spotify URL.".to_string());
    }

    let title = if source_count > max_tracks {
        format!("{} (first {} tracks)", title, max_tracks)
    } else {
        title
    };

    Ok(SpotifyQueue {
        items,
        title,
        thumbnail,
        source_count,
    })
}

#[derive(Debug)]
struct SpotifyRef {
    kind: SpotifyKind,
    id: String,
}

fn parse_spotify_ref(input: &str) -> Option<SpotifyRef> {
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

fn spotify_track_limit() -> usize {
    std::env::var("BKGRND_SPOTIFY_MAX_TRACKS")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(75)
}

async fn access_token() -> Result<String, String> {
    if let Ok(token) = std::env::var("BKGRND_SPOTIFY_ACCESS_TOKEN") {
        let token = token.trim();
        if !token.is_empty() {
            return Ok(token.to_string());
        }
    }

    let client_id =
        std::env::var("BKGRND_SPOTIFY_CLIENT_ID").map_err(|_| spotify_credentials_error())?;
    let client_secret =
        std::env::var("BKGRND_SPOTIFY_CLIENT_SECRET").map_err(|_| spotify_credentials_error())?;
    if client_id.trim().is_empty() || client_secret.trim().is_empty() {
        return Err(spotify_credentials_error());
    }

    #[derive(Deserialize)]
    struct TokenResponse {
        access_token: String,
    }

    let auth = general_purpose::STANDARD.encode(format!("{}:{}", client_id, client_secret));
    let response = reqwest::Client::new()
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
    Ok(token.access_token)
}

fn spotify_credentials_error() -> String {
    "Spotify playlist conversion requires BKGRND_SPOTIFY_CLIENT_ID and BKGRND_SPOTIFY_CLIENT_SECRET, or BKGRND_SPOTIFY_ACCESS_TOKEN.".to_string()
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

#[derive(Debug)]
struct TrackMeta {
    name: String,
    artists: Vec<String>,
}

impl From<SpotifyTrack> for TrackMeta {
    fn from(track: SpotifyTrack) -> Self {
        TrackMeta {
            name: track.name,
            artists: track
                .artists
                .into_iter()
                .map(|artist| artist.name)
                .collect(),
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
        artists: track
            .artists
            .into_iter()
            .map(|artist| artist.name)
            .collect(),
    };

    Ok((track.name, thumbnail, vec![meta]))
}

fn first_image(images: Vec<SpotifyImage>) -> String {
    images
        .into_iter()
        .next()
        .map(|image| image.url)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::{parse_spotify_ref, SpotifyKind};

    #[test]
    fn parses_open_spotify_playlist_urls() {
        let parsed =
            parse_spotify_ref("https://open.spotify.com/playlist/37i9dQZF1DXcBWIGoYBM5M?si=abc")
                .unwrap();

        assert_eq!(parsed.kind, SpotifyKind::Playlist);
        assert_eq!(parsed.id, "37i9dQZF1DXcBWIGoYBM5M");
    }

    #[test]
    fn parses_internationalized_open_spotify_urls() {
        let parsed =
            parse_spotify_ref("https://open.spotify.com/intl-us/album/4aawyAB9vmqN3uQ7FjRGTy")
                .unwrap();

        assert_eq!(parsed.kind, SpotifyKind::Album);
        assert_eq!(parsed.id, "4aawyAB9vmqN3uQ7FjRGTy");
    }

    #[test]
    fn parses_spotify_uri() {
        let parsed = parse_spotify_ref("spotify:track:11dFghVXANMlKmJXsNCbNl").unwrap();

        assert_eq!(parsed.kind, SpotifyKind::Track);
        assert_eq!(parsed.id, "11dFghVXANMlKmJXsNCbNl");
    }
}
