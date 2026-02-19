use std::{
    collections::HashSet,
    sync::{atomic::AtomicBool, Arc, OnceLock},
    time::Duration,
};

use reqwest::header::{HeaderValue, CONTENT_TYPE};

use crate::{DeadlineMonitor, Result};

#[derive(serde::Serialize, serde::Deserialize)]
pub struct Voice {
    pub id: String,
    pub name: String,
}

pub struct State {
    daemon_url: reqwest::Url,
    health_path: String,
    voices_path: String,
    tts_path: String,
    semaphore: Arc<tokio::sync::Semaphore>,
    client: reqwest::Client,
}

impl State {
    pub async fn new() -> Result<Self> {
        let daemon_url = std::env::var("GWENT_DAEMON_URL")
            .unwrap_or_else(|_| String::from("http://127.0.0.1:9000"))
            .parse()?;

        let connect_timeout = parse_env_u64("GWENT_CONNECT_TIMEOUT_MS", 500)?;
        let request_timeout = parse_env_u64("GWENT_REQUEST_TIMEOUT_MS", 10_000)?;
        let max_concurrency = parse_env_u64("GWENT_MAX_CONCURRENCY", 32)?;
        let max_concurrency = usize::try_from(max_concurrency)?;

        if max_concurrency == 0 {
            anyhow::bail!("GWENT_MAX_CONCURRENCY must be greater than 0");
        }

        let state = Self {
            daemon_url,
            health_path: env_path("GWENT_HEALTH_PATH", "/health"),
            voices_path: env_path("GWENT_VOICES_PATH", "/voices"),
            tts_path: env_path("GWENT_TTS_PATH", "/tts"),
            semaphore: Arc::new(tokio::sync::Semaphore::new(max_concurrency)),
            client: reqwest::Client::builder()
                .connect_timeout(Duration::from_millis(connect_timeout))
                .timeout(Duration::from_millis(request_timeout))
                .build()?,
        };

        state.probe_daemon().await;
        Ok(state)
    }

    fn endpoint_url(&self, path: &str) -> reqwest::Url {
        let mut url = self.daemon_url.clone();
        url.set_path(path);
        url
    }

    async fn probe_daemon(&self) {
        let health_url = self.endpoint_url(&self.health_path);
        match self.client.get(health_url).send().await {
            Ok(resp) if resp.status().is_success() => {
                tracing::info!("Gwent daemon healthcheck passed");
            }
            Ok(resp) => {
                tracing::warn!("Gwent daemon healthcheck returned {}", resp.status());
                return;
            }
            Err(err) => {
                tracing::warn!("Unable to reach Gwent daemon at startup: {err}");
                return;
            }
        }

        let static_voices = get_voices().into_iter().collect::<HashSet<_>>();
        match self.fetch_daemon_voice_ids().await {
            Ok(daemon_voices) => {
                let missing = static_voices.difference(&daemon_voices).collect::<Vec<_>>();
                let extra = daemon_voices.difference(&static_voices).collect::<Vec<_>>();

                if !missing.is_empty() {
                    tracing::warn!(
                        "Configured static Gwent voices missing from daemon: {:?}",
                        missing
                    );
                }

                if !extra.is_empty() {
                    tracing::warn!(
                        "Daemon reported additional Gwent voices not in static map: {:?}",
                        extra
                    );
                }
            }
            Err(err) => tracing::warn!("Failed to fetch Gwent daemon voices for validation: {err}"),
        }
    }

    async fn fetch_daemon_voice_ids(&self) -> Result<HashSet<String>> {
        let voices_url = self.endpoint_url(&self.voices_path);
        let resp = self
            .client
            .get(voices_url)
            .send()
            .await?
            .error_for_status()?;
        let value: serde_json::Value = resp.json().await?;

        let Some(array) = value.as_array() else {
            anyhow::bail!("Gwent daemon /voices returned non-array payload");
        };

        let mut out = HashSet::new();
        for item in array {
            if let Some(voice_id) = item.as_str() {
                out.insert(voice_id.to_owned());
            } else if let Some(voice_id) = item.get("id").and_then(serde_json::Value::as_str) {
                out.insert(voice_id.to_owned());
            }
        }

        Ok(out)
    }
}

fn parse_env_u64(name: &str, default: u64) -> Result<u64> {
    match std::env::var(name) {
        Ok(value) => Ok(value.parse::<u64>()?),
        Err(_) => Ok(default),
    }
}

fn env_path(name: &str, default: &str) -> String {
    let path = std::env::var(name).unwrap_or_else(|_| default.to_owned());
    if path.starts_with('/') {
        path
    } else {
        format!("/{path}")
    }
}

fn get_voice_map() -> &'static Vec<Voice> {
    static GWENT_VOICES: OnceLock<Vec<Voice>> = OnceLock::new();
    GWENT_VOICES.get_or_init(|| {
        serde_json::from_str(include_str!("data/voices-gwent.json"))
            .expect("Invalid Gwent voices file")
    })
}

pub fn get_raw_voices() -> &'static [Voice] {
    get_voice_map()
}

pub fn get_voices() -> Vec<String> {
    get_voice_map()
        .iter()
        .map(|voice| voice.id.clone())
        .collect::<Vec<_>>()
}

pub fn check_voice(voice: &str) -> bool {
    get_voice_map().iter().any(|v| v.id == voice)
}

#[derive(Clone, Copy)]
enum AudioFormat {
    Ogg,
    Mp3,
}

impl AudioFormat {
    fn parse(preferred: Option<&str>) -> Self {
        if preferred.is_some_and(|f| f.eq_ignore_ascii_case("mp3")) {
            Self::Mp3
        } else {
            Self::Ogg
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Mp3 => "mp3",
            Self::Ogg => "ogg",
        }
    }

    fn default_content_type(self) -> HeaderValue {
        match self {
            Self::Mp3 => HeaderValue::from_static("audio/mpeg"),
            Self::Ogg => HeaderValue::from_static("audio/ogg"),
        }
    }
}

#[derive(serde::Serialize)]
struct TtsRequest<'a> {
    text: &'a str,
    voice: &'a str,
    speaking_rate: f32,
    format: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_length: Option<u64>,
}

pub async fn get_tts(
    state: &State,
    text: &str,
    voice: &str,
    speaking_rate: f32,
    preferred_format: Option<&str>,
    max_length: Option<u64>,
    hit_any_deadline: Arc<AtomicBool>,
) -> Result<(bytes::Bytes, Option<HeaderValue>)> {
    let _guard = DeadlineMonitor::new(Duration::from_millis(4_000), hit_any_deadline, |took| {
        tracing::warn!("Fetching Gwent audio took {} millis!", took.as_millis());
    });

    let _permit = state.semaphore.acquire().await?;
    let format = AudioFormat::parse(preferred_format);

    let payload = TtsRequest {
        text,
        voice,
        speaking_rate,
        format: format.as_str(),
        max_length,
    };

    let req_url = state.endpoint_url(&state.tts_path);
    let resp = state.client.post(req_url).json(&payload).send().await?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        anyhow::bail!("Gwent daemon request failed ({status}): {body}");
    }

    let mut content_type = resp.headers().get(CONTENT_TYPE).cloned();
    let audio = resp.bytes().await?;
    if content_type.is_none() {
        content_type = Some(format.default_content_type());
    }

    Ok((audio, content_type))
}
