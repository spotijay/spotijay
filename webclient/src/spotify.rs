use seed::browser::url::UrlSearch;
use serde::{Deserialize, Serialize};

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
pub struct SpotifyImage {
    pub height: u32,
    pub width: u32,
    pub url: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct SpotifyAlbum {
    pub id: String,
    pub name: String,
    pub images: Vec<SpotifyImage>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct SpotifyTrack {
    pub id: String,
    pub name: String,
    pub uri: String,
    pub duration_ms: u32,
    pub artists: Vec<SpotifyArtist>,
    pub album: SpotifyAlbum,
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
    let search = UrlSearch::new(vec![
        ("client_id", vec![dotenv_codegen::dotenv!("CLIENT_ID")]),
        ("response_type", vec!["token"]),
        (
            "redirect_uri",
            vec![dotenv_codegen::dotenv!("REDIRECT_URI")],
        ),
        ("scope", vec!["user-modify-playback-state"]),
    ]);

    [
        "https://accounts.spotify.com/authorize",
        &search.to_string(),
    ]
    .join("?")
}
