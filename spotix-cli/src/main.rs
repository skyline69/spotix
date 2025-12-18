use spotix_core::{
    audio::{
        normalize::NormalizationLevel,
        output::{AudioOutput, AudioSink, DefaultAudioOutput},
    },
    cache::{Cache, CacheHandle},
    cdn::{Cdn, CdnHandle},
    connection::Credentials,
    error::Error,
    item_id::{ItemId, ItemIdType},
    player::{PlaybackConfig, Player, PlayerCommand, PlayerEvent, item::PlaybackItem},
    session::{SessionConfig, SessionService},
};
use std::{env, io, io::BufRead, path::PathBuf, thread};

fn main() {
    env_logger::init();

    let args: Vec<String> = env::args().collect();
    let track_id = match args.get(1) {
        Some(id) => id,
        None => {
            let exe = args.first().map(String::as_str).unwrap_or("spotix-cli");
            eprintln!("Usage: {exe} <track_id>");
            std::process::exit(1);
        }
    };

    let username = match env::var("SPOTIFY_USERNAME") {
        Ok(u) => u,
        Err(_) => {
            eprintln!("Set SPOTIFY_USERNAME and SPOTIFY_PASSWORD environment variables.");
            std::process::exit(1);
        }
    };
    let password = match env::var("SPOTIFY_PASSWORD") {
        Ok(p) => p,
        Err(_) => {
            eprintln!("Set SPOTIFY_USERNAME and SPOTIFY_PASSWORD environment variables.");
            std::process::exit(1);
        }
    };
    let login_creds = Credentials::from_username_and_password(username, password);
    let session = SessionService::with_config(SessionConfig {
        login_creds,
        proxy_url: None,
    });

    start(track_id, session).unwrap();
}

fn start(track_id: &str, session: SessionService) -> Result<(), Error> {
    let cdn = Cdn::new(session.clone(), None)?;
    let cache = Cache::new(PathBuf::from("cache"))?;
    let item_id = ItemId::from_base62(track_id, ItemIdType::Track).unwrap();
    play_item(
        session,
        cdn,
        cache,
        PlaybackItem {
            item_id,
            norm_level: NormalizationLevel::Track,
        },
    )
}

fn play_item(
    session: SessionService,
    cdn: CdnHandle,
    cache: CacheHandle,
    item: PlaybackItem,
) -> Result<(), Error> {
    let output = DefaultAudioOutput::open()?;
    let config = PlaybackConfig::default();

    let mut player = Player::new(session, cdn, cache, config, &output);

    let _ui_thread = thread::spawn({
        let player_sender = player.sender();

        player_sender
            .send(PlayerEvent::Command(PlayerCommand::LoadQueue {
                items: vec![item, item, item],
                position: 0,
            }))
            .unwrap();

        move || {
            for line in io::stdin().lock().lines() {
                match line.as_ref().map(|s| s.as_str()) {
                    Ok("p") => {
                        player_sender
                            .send(PlayerEvent::Command(PlayerCommand::Pause))
                            .unwrap();
                    }
                    Ok("r") => {
                        player_sender
                            .send(PlayerEvent::Command(PlayerCommand::Resume))
                            .unwrap();
                    }
                    Ok("s") => {
                        player_sender
                            .send(PlayerEvent::Command(PlayerCommand::Stop))
                            .unwrap();
                    }
                    Ok("<") => {
                        player_sender
                            .send(PlayerEvent::Command(PlayerCommand::Previous))
                            .unwrap();
                    }
                    Ok(">") => {
                        player_sender
                            .send(PlayerEvent::Command(PlayerCommand::Next))
                            .unwrap();
                    }
                    _ => log::warn!("unknown command"),
                }
            }
        }
    });

    for event in player.receiver() {
        player.handle(event);
    }
    output.sink().close();

    Ok(())
}
