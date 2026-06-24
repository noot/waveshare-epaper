use eyre::WrapErr as _;
use reqwest::Client;
use serde::Deserialize;
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::debug;

pub struct NowPlaying {
    pub track: String,
    pub artist: String,
    pub album: String,
    pub cover_art: Option<Vec<u8>>,
    pub progress_secs: Option<u32>,
    pub duration_secs: Option<u32>,
    pub is_playing: bool,
}

pub struct NavidromeConfig {
    url: String,
    user: String,
    pass: String,
    client: Client,
}

pub struct SpotifyConfig {
    client_id: String,
    client_secret: String,
    refresh_token: String,
    access_token: Arc<Mutex<Option<String>>>,
    client: Client,
}

pub enum Backend {
    Navidrome(NavidromeConfig),
    Spotify(SpotifyConfig),
}

impl Backend {
    pub fn navidrome(url: String, user: String, pass: String) -> Self {
        Self::Navidrome(NavidromeConfig {
            url,
            user,
            pass,
            client: Client::new(),
        })
    }

    pub fn spotify(client_id: String, client_secret: String, refresh_token: String) -> Self {
        Self::Spotify(SpotifyConfig {
            client_id,
            client_secret,
            refresh_token,
            access_token: Arc::new(Mutex::new(None)),
            client: Client::new(),
        })
    }

    pub async fn now_playing(&self) -> eyre::Result<Option<NowPlaying>> {
        match self {
            Self::Navidrome(cfg) => cfg.now_playing().await,
            Self::Spotify(cfg) => cfg.now_playing().await,
        }
    }

    pub async fn play_pause(&self) -> eyre::Result<()> {
        match self {
            Self::Navidrome(_) => Ok(()),
            Self::Spotify(cfg) => cfg.play_pause().await,
        }
    }

    pub async fn next_track(&self) -> eyre::Result<()> {
        match self {
            Self::Navidrome(_) => Ok(()),
            Self::Spotify(cfg) => cfg.next_track().await,
        }
    }
}

// --- navidrome (subsonic API) ---

#[derive(Deserialize)]
struct SubsonicResponse {
    #[serde(rename = "subsonic-response")]
    inner: SubsonicInner,
}

#[derive(Deserialize)]
struct SubsonicInner {
    #[serde(rename = "nowPlaying")]
    now_playing: Option<SubsonicNowPlaying>,
}

#[derive(Deserialize)]
struct SubsonicNowPlaying {
    entry: Option<Vec<SubsonicEntry>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SubsonicEntry {
    title: String,
    artist: Option<String>,
    album: Option<String>,
    cover_art: Option<String>,
    duration: Option<u32>,
    #[allow(dead_code)]
    player_name: Option<String>,
}

impl NavidromeConfig {
    fn subsonic_params(&self) -> Vec<(&str, &str)> {
        vec![
            ("u", &self.user),
            ("p", &self.pass),
            ("v", "1.16.1"),
            ("c", "waveshare-epaper"),
            ("f", "json"),
        ]
    }

    async fn now_playing(&self) -> eyre::Result<Option<NowPlaying>> {
        let resp: SubsonicResponse = self
            .client
            .get(format!("{}/rest/getNowPlaying", self.url))
            .query(&self.subsonic_params())
            .send()
            .await
            .wrap_err("failed to reach navidrome")?
            .json()
            .await
            .wrap_err("failed to parse navidrome response")?;

        let entry = resp
            .inner
            .now_playing
            .and_then(|np| np.entry)
            .and_then(|mut entries| {
                if entries.is_empty() {
                    None
                } else {
                    Some(entries.swap_remove(0))
                }
            });

        let Some(entry) = entry else {
            return Ok(None);
        };

        debug!(track = %entry.title, artist = ?entry.artist, "navidrome: now playing");

        let cover_art = if let Some(ref id) = entry.cover_art {
            self.fetch_cover_art(id).await.ok()
        } else {
            None
        };

        Ok(Some(NowPlaying {
            track: entry.title,
            artist: entry.artist.unwrap_or_default(),
            album: entry.album.unwrap_or_default(),
            cover_art,
            progress_secs: None,
            duration_secs: entry.duration,
            is_playing: true,
        }))
    }

    async fn fetch_cover_art(&self, id: &str) -> eyre::Result<Vec<u8>> {
        let mut params = self.subsonic_params();
        let size = "300".to_string();
        params.push(("id", id));
        params.push(("size", &size));

        let bytes = self
            .client
            .get(format!("{}/rest/getCoverArt", self.url))
            .query(&params)
            .send()
            .await
            .wrap_err("failed to fetch cover art")?
            .bytes()
            .await
            .wrap_err("failed to read cover art bytes")?;

        Ok(bytes.to_vec())
    }
}

// --- spotify ---

#[derive(Deserialize)]
struct SpotifyTokenResponse {
    access_token: String,
}

#[derive(Deserialize)]
struct SpotifyCurrentlyPlaying {
    is_playing: bool,
    progress_ms: Option<u64>,
    item: Option<SpotifyTrack>,
}

#[derive(Deserialize)]
struct SpotifyTrack {
    name: String,
    duration_ms: u64,
    artists: Vec<SpotifyArtist>,
    album: SpotifyAlbum,
}

#[derive(Deserialize)]
struct SpotifyArtist {
    name: String,
}

#[derive(Deserialize)]
struct SpotifyAlbum {
    name: String,
    images: Vec<SpotifyImage>,
}

#[derive(Deserialize)]
struct SpotifyImage {
    url: String,
}

impl SpotifyConfig {
    async fn refresh_access_token(&self) -> eyre::Result<String> {
        let resp: SpotifyTokenResponse = self
            .client
            .post("https://accounts.spotify.com/api/token")
            .basic_auth(&self.client_id, Some(&self.client_secret))
            .form(&[
                ("grant_type", "refresh_token"),
                ("refresh_token", &self.refresh_token),
            ])
            .send()
            .await
            .wrap_err("failed to refresh spotify token")?
            .json()
            .await
            .wrap_err("failed to parse spotify token response")?;

        let token = resp.access_token;
        *self.access_token.lock().await = Some(token.clone());
        Ok(token)
    }

    async fn get_token(&self) -> eyre::Result<String> {
        let guard = self.access_token.lock().await;
        if let Some(ref token) = *guard {
            return Ok(token.clone());
        }
        drop(guard);
        self.refresh_access_token().await
    }

    async fn now_playing(&self) -> eyre::Result<Option<NowPlaying>> {
        let token = self.get_token().await?;

        let resp = self
            .client
            .get("https://api.spotify.com/v1/me/player/currently-playing")
            .bearer_auth(&token)
            .send()
            .await
            .wrap_err("failed to reach spotify")?;

        if resp.status() == reqwest::StatusCode::UNAUTHORIZED {
            let token = self.refresh_access_token().await?;
            return self.fetch_with_token(&token).await;
        }

        if resp.status() == reqwest::StatusCode::NO_CONTENT {
            return Ok(None);
        }

        let body: SpotifyCurrentlyPlaying = resp
            .json()
            .await
            .wrap_err("failed to parse spotify response")?;

        self.parse_response(body).await
    }

    async fn fetch_with_token(&self, token: &str) -> eyre::Result<Option<NowPlaying>> {
        let resp = self
            .client
            .get("https://api.spotify.com/v1/me/player/currently-playing")
            .bearer_auth(token)
            .send()
            .await
            .wrap_err("failed to reach spotify after token refresh")?;

        if resp.status() == reqwest::StatusCode::NO_CONTENT {
            return Ok(None);
        }

        let body: SpotifyCurrentlyPlaying = resp
            .json()
            .await
            .wrap_err("failed to parse spotify response")?;

        self.parse_response(body).await
    }

    async fn spotify_request(&self, method: reqwest::Method, url: &str) -> eyre::Result<()> {
        let token = self.get_token().await?;
        let mut resp = self
            .client
            .request(method.clone(), url)
            .bearer_auth(&token)
            .header(reqwest::header::CONTENT_LENGTH, 0)
            .send()
            .await
            .wrap_err("failed to reach spotify")?;

        debug!("spotify {} {} → {}", method, url, resp.status());

        if resp.status() == reqwest::StatusCode::UNAUTHORIZED {
            let token = self.refresh_access_token().await?;
            resp = self
                .client
                .request(method.clone(), url)
                .bearer_auth(&token)
                .header(reqwest::header::CONTENT_LENGTH, 0)
                .send()
                .await
                .wrap_err("failed to reach spotify after token refresh")?;
            debug!("spotify {} {} (retry) → {}", method, url, resp.status());
        }

        let status = resp.status();
        if !status.is_success() && status != reqwest::StatusCode::NO_CONTENT {
            let body = resp.text().await.unwrap_or_default();
            eyre::bail!("spotify {} {} returned {}: {}", method, url, status, body);
        }

        Ok(())
    }

    async fn play_pause(&self) -> eyre::Result<()> {
        let token = self.get_token().await?;
        let resp = self
            .client
            .get("https://api.spotify.com/v1/me/player")
            .bearer_auth(&token)
            .send()
            .await
            .wrap_err("failed to get playback state")?;

        if resp.status() == reqwest::StatusCode::NO_CONTENT {
            return self
                .spotify_request(
                    reqwest::Method::PUT,
                    "https://api.spotify.com/v1/me/player/play",
                )
                .await;
        }

        let body: serde_json::Value = resp
            .json()
            .await
            .wrap_err("failed to parse playback state")?;

        let is_playing = body
            .get("is_playing")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        if is_playing {
            self.spotify_request(
                reqwest::Method::PUT,
                "https://api.spotify.com/v1/me/player/pause",
            )
            .await
        } else {
            self.spotify_request(
                reqwest::Method::PUT,
                "https://api.spotify.com/v1/me/player/play",
            )
            .await
        }
    }

    async fn next_track(&self) -> eyre::Result<()> {
        self.spotify_request(
            reqwest::Method::POST,
            "https://api.spotify.com/v1/me/player/next",
        )
        .await
    }

    async fn parse_response(
        &self,
        body: SpotifyCurrentlyPlaying,
    ) -> eyre::Result<Option<NowPlaying>> {
        let Some(item) = body.item else {
            return Ok(None);
        };

        debug!(track = %item.name, playing = body.is_playing, "spotify: track");

        let cover_art = if let Some(img) = item.album.images.first() {
            match self.client.get(&img.url).send().await {
                Ok(resp) => resp.bytes().await.ok().map(|b| b.to_vec()),
                Err(_) => None,
            }
        } else {
            None
        };

        let artist = item
            .artists
            .into_iter()
            .map(|a| a.name)
            .collect::<Vec<_>>()
            .join(", ");

        Ok(Some(NowPlaying {
            track: item.name,
            artist,
            album: item.album.name,
            cover_art,
            progress_secs: body.progress_ms.map(|ms| (ms / 1000) as u32),
            duration_secs: Some((item.duration_ms / 1000) as u32),
            is_playing: body.is_playing,
        }))
    }
}
