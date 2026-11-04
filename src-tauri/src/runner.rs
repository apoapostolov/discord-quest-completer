use discord_sdk::activity::ActivityBuilder;

use crate::rpc::{self, ds};
use serde::Deserialize;

#[derive(Deserialize)]
pub struct ActivityParams {
    pub app_id: String,
    pub details: Option<String>,
    pub state: Option<String>,
    #[serde(rename = "largeImageKey")]
    pub large_image_key: Option<String>,
    #[serde(rename = "largeImageText")]
    pub large_image_text: Option<String>,
    pub timestamp: Option<i64>,
    pub activity_kind: Option<i32>,
}

pub struct CreateActivityResult {
    pub activity: ActivityBuilder,
    pub app_id: u64,
}

fn to_app_id(app_id: &str) -> Result<u64, String> {
    app_id.parse::<u64>().map_err(|e| {
        eprintln!("Failed to parse app_id: {}", e);
        format!("Failed to parse app_id: {}", e)
    })
}

pub fn parse_activity_json(activity_json: &str) -> Result<ActivityParams, String> {
    serde_json::from_str(activity_json).map_err(|e| {
        eprintln!("Failed to parse activity JSON: {}", e);
        format!("Failed to parse activity JSON: {}", e)
    })
}

pub fn create_activity(activity_json: String) -> Result<CreateActivityResult, String> {
    let activity: ActivityParams = parse_activity_json(&activity_json)?;

    let app_id: u64 = to_app_id(&activity.app_id).map_err(|e| {
        eprintln!("Failed to parse app_id: {}", e);
        format!("Failed to parse app_id: {}", e)
    })?;

    let details = activity.details.unwrap_or_default();
    let state = activity.state.unwrap_or_default();
    let large_image_key = activity.large_image_key.unwrap_or_default();
    let large_image_text = activity.large_image_text;
    let timestamp = activity.timestamp;
    let activity_kind = activity.activity_kind.unwrap_or(0);

    let mut rp: discord_sdk::activity::ActivityBuilder =
        ds::activity::ActivityBuilder::default();

    match activity_kind {
        0 => rp = rp.kind(ds::activity::ActivityKind::Playing),
        2 => rp = rp.kind(ds::activity::ActivityKind::Listening),
        3 => rp = rp.kind(ds::activity::ActivityKind::Watching),
        5 => rp = rp.kind(ds::activity::ActivityKind::Competing),
        _ => rp = rp.kind(ds::activity::ActivityKind::Playing),
    }

    if !details.is_empty() {
        rp = rp.details(details);
    }

    if !state.is_empty() {
        rp = rp.state(state);
    }

    if let Some(ts) = timestamp {
        rp = rp.start_timestamp(ts as i64);
    }

    if !large_image_key.is_empty() {
        rp = rp
            .assets(ds::activity::Assets::default().large(&large_image_key, large_image_text));
    }

    Ok(CreateActivityResult {
        activity: rp,
        app_id,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_malformed_json() {
        assert!(parse_activity_json("not json").is_err());
    }

    #[test]
    fn rejects_invalid_app_id() {
        let json = r#"{"app_id":"not-a-number"}"#;
        assert!(create_activity(json.to_string()).is_err());
    }

    #[test]
    fn builds_minimal_activity() {
        let json = r#"{"app_id":"12345"}"#;
        let result = create_activity(json.to_string()).unwrap();
        assert_eq!(result.app_id, 12345);
    }
}
