use serde::{Deserialize, Serialize};
use url::Url;

#[derive(Serialize)]
pub struct PlayRequest {
    pub uris: Vec<String>,
    pub position_ms: u32,
}

#[derive(Serialize)]
struct QueueRequest {
    uri: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct SpotifyProfile {
    pub id: String,
    pub display_name: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct SpotifyTrack {
    pub id: String,
    pub name: String,
    pub uri: String,
    pub duration_ms: u32,
    pub artists: Vec<SpotifyArtist>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct SpotifySearchResult {
    pub tracks: SpotifyTrackList,
}

#[derive(Debug, Deserialize, Clone)]
pub struct SpotifyTrackList {
    pub items: Vec<SpotifyTrack>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct SpotifyArtist {
    pub name: String,
}

pub fn login_url() -> String {
    Url::parse_with_params(
        "https://accounts.spotify.com/authorize",
        &[
            ("client_id", dotenv_codegen::dotenv!("CLIENT_ID")),
            ("response_type", "token"),
            ("redirect_uri", dotenv_codegen::dotenv!("REDIRECT_URI")),
            ("scope", "user-modify-playback-state"),
        ],
    )
    .unwrap()
    .to_string()
}
