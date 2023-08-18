//! Requires the "client", "standard_framework", and "voice" features be enabled
//! in your Cargo.toml, like so:
//!
//! ```toml
//! [dependencies.serenity]
//! git = "https://github.com/serenity-rs/serenity.git"
//! features = ["client", "standard_framework", "voice"]
//! ```
use std::{
    io::Cursor,
    num::NonZeroUsize,
    sync::{Arc, OnceLock},
};

use dashmap::DashMap;

use serenity::{
    all::GuildId,
    async_trait,
    client::{Client, Context, EventHandler},
    framework::StandardFramework,
    model::{gateway::Ready, id::ChannelId},
    prelude::GatewayIntents,
};

use songbird::{
    driver::DecodeMode,
    events::context_data::VoiceTick,
    model::{id::UserId, payload::Speaking},
    Config, CoreEvent, Event, EventContext, EventHandler as VoiceEventHandler,
};

struct Handler;

#[async_trait]
impl EventHandler for Handler {
    async fn ready(&self, ctx: Context, ready: Ready) {
        if let Ok(handler_lock) = songbird::get(&ctx)
            .await
            .unwrap()
            .join(
                CONFIG.get().unwrap().guild_id,
                CONFIG.get().unwrap().channel_id,
            )
            .await
        {
            let mut handler = handler_lock.lock().await;

            let evt_receiver = Receiver::new();

            handler.add_global_event(CoreEvent::SpeakingStateUpdate.into(), evt_receiver.clone());
            handler.add_global_event(CoreEvent::RtpPacket.into(), evt_receiver.clone());
            handler.add_global_event(CoreEvent::RtcpPacket.into(), evt_receiver.clone());
            handler.add_global_event(CoreEvent::ClientDisconnect.into(), evt_receiver.clone());
            handler.add_global_event(CoreEvent::VoiceTick.into(), evt_receiver);
        }
        tracing::info!("{} is connected!", ready.user.name);
    }
}

#[derive(Clone)]
struct Receiver {
    users: Arc<DashMap<u32, (Vec<i16>, UserId)>>,
    http: reqwest::Client,
}

impl Receiver {
    pub fn new() -> Self {
        // You can manage state here, such as a buffer of audio packet bytes so
        // you can later store them in intervals.
        Self {
            users: Arc::new(DashMap::with_capacity(10)),
            http: reqwest::Client::new(),
        }
    }
}

const DEFAULT_SAMPLE_COUNT: usize = 44_100 * 10 * 2;
const AUDIO_SPEC: hound::WavSpec = hound::WavSpec {
    channels: 2,
    sample_rate: 44100,
    bits_per_sample: 16,
    sample_format: hound::SampleFormat::Int,
};

#[derive(Debug)]
pub enum FireError {
    Reqwest(reqwest::Error),
    Hound(hound::Error),
}

impl From<hound::Error> for FireError {
    fn from(value: hound::Error) -> Self {
        Self::Hound(value)
    }
}

impl From<reqwest::Error> for FireError {
    fn from(value: reqwest::Error) -> Self {
        Self::Reqwest(value)
    }
}

async fn fire_request(
    client: reqwest::Client,
    id: UserId,
    audio: Vec<i16>,
) -> Result<(), FireError> {
    let mut buf = Vec::with_capacity(audio.len() * 2);
    let mut writer = hound::WavWriter::new(Cursor::new(&mut buf), AUDIO_SPEC)?;
    for sample in audio {
        writer.write_sample(sample)?;
    }
    writer.finalize()?;
    // tokio::fs::write("./out.wav", &buf).await.unwrap();
    client
        .put(&CONFIG.get().unwrap().endpoint)
        .header("User-Id", id.to_string())
        .header(
            "Authorization",
            format!("Bearer {}", CONFIG.get().unwrap().endpoint_token),
        )
        .header("Content-Type", "audio/wav")
        .body(buf)
        .send()
        .await?
        .error_for_status()?;
    Ok(())
}

async fn fire_tick(tick: VoiceTick, ctx: Receiver) {
    for (ssrc, data) in &tick.speaking {
        tracing::trace!("got ssrc {ssrc}");
        if let Some(decoded_voice) = data.decoded_voice.as_ref() {
            if let Some(mut user) = ctx.users.get_mut(ssrc) {
                user.0.extend_from_slice(decoded_voice);
            }
        } else {
            tracing::error!("Decode disabled.");
        }
    }
    let mut deletions: Vec<u32> = Vec::with_capacity(10);
    for i in ctx.users.iter() {
        let (ssrc, user) = i.pair();
        if tick.silent.contains(ssrc) && !user.0.is_empty() {
            deletions.push(*ssrc);
        }
    }
    for ssrc in deletions {
        if let Some((_ssrc, (voicedata, userid))) = ctx.users.remove(&ssrc) {
            let http = ctx.http.clone();
            tokio::spawn(async move {
                if let Err(source) = fire_request(http, userid, voicedata).await {
                    tracing::error!(?source);
                }
            });
        }
    }
}

#[async_trait]
impl VoiceEventHandler for Receiver {
    #[allow(unused_variables)]
    async fn act(&self, ctx: &EventContext<'_>) -> Option<Event> {
        use EventContext as Ctx;
        match ctx {
            Ctx::SpeakingStateUpdate(Speaking {
                speaking,
                ssrc,
                user_id: Some(user_id),
                ..
            }) => {
                self.users
                    .insert(*ssrc, (Vec::with_capacity(DEFAULT_SAMPLE_COUNT), *user_id));
            }
            Ctx::VoiceTick(tick) => {
                let rcvr = self.clone();
                let tick = tick.to_owned();
                tokio::spawn(fire_tick(tick, rcvr));
            }
            _ => {
                // We won't be registering this struct for any more event classes.
            }
        }

        None
    }
}

static CONFIG: OnceLock<AppConfig> = OnceLock::new();

#[tokio::main]
async fn main() {
    dotenvy::dotenv().ok();
    tracing_subscriber::fmt::init();

    let config: AppConfig = envy::from_env().expect("Failed to read config");
    let framework = StandardFramework::new();

    let intents = GatewayIntents::GUILD_VOICE_STATES;
    let songbird_config = Config::default()
        .decode_mode(DecodeMode::Decode)
        .playout_buffer_length(NonZeroUsize::new(20).unwrap());
    let chickadee = songbird::Songbird::serenity();
    chickadee.set_config(songbird_config);
    let mut client_builder = Client::builder(&config.discord_token, intents);
    client_builder = songbird::serenity::register_with(client_builder, chickadee.clone());
    let mut client = client_builder
        .event_handler(Handler)
        .framework(framework)
        .await
        .expect("Err creating client");
    CONFIG.set(config).unwrap();
    let _ = client
        .start()
        .await
        .map_err(|why| tracing::info!("Client ended: {:?}", why));
}

#[derive(serde::Deserialize, Debug)]
pub struct AppConfig {
    discord_token: String,
    endpoint_token: String,
    endpoint: String,
    guild_id: GuildId,
    channel_id: ChannelId,
}
