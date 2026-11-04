pub use discord_sdk as ds;
pub use tokio;

pub struct Client {
    pub discord: ds::Discord,
    pub wheel: ds::wheel::Wheel,
    pub user: ds::user::User,
}

pub async fn make_client(app_id: ds::AppId, subs: ds::Subscriptions) -> Result<Client, String> {
    println!("Creating Discord client with app ID: {}", app_id);
    let (wheel, handler) = ds::wheel::Wheel::new(Box::new(|err| {
        eprintln!("Discord wheel error: {:?}", err);
    }));

    let mut user = wheel.user();

    let discord = ds::Discord::new(ds::DiscordApp::PlainId(app_id), subs, Box::new(handler))
        .map_err(|e| format!("unable to create discord client: {}", e))?;

    let connect_timeout = std::time::Duration::from_secs(10);
    match tokio::time::timeout(connect_timeout, user.0.changed()).await {
        Ok(Ok(())) => {}
        Ok(Err(e)) => return Err(format!("user state channel error: {}", e)),
        Err(_) => return Err("timed out waiting for Discord connection".to_string()),
    }

    let user = match &*user.0.borrow() {
        ds::wheel::UserState::Connected(user) => user.clone(),
        ds::wheel::UserState::Disconnected(err) => {
            return Err(format!("failed to connect to Discord: {}", err))
        }
    };

    println!("connected to Discord");

    Ok(Client {
        discord,
        wheel,
        user,
    })
}
