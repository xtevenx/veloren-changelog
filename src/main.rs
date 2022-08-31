use std::fs;
use std::io;
use std::sync::Arc;

use serenity::client::bridge::gateway::ShardManager;
use serenity::model::gateway::Ready;
use serenity::prelude::*;

const CHANGELOG_PATH: &str = "CHANGELOG.md";
const CHANGELOG_URL: &str = "https://gitlab.com/veloren/veloren/-/raw/nightly/CHANGELOG.md";

const UNRELEASED_HEADER: &str = "## [Unreleased]";

// There is definitely a way of doing this without abusing unsafe but I cannot currently find a way
// to achieve that. *Surely* this doesn't come back to bite me. :D
static mut SHARD_MANAGER: Option<Arc<Mutex<ShardManager>>> = None;

#[tokio::main]
async fn main() -> reqwest::Result<()> {
    let changelog_old = match read_changelog().await {
        Ok(s) => s,
        Err(_) => download_changelog().await?,
    };

    let changelog_new = download_changelog().await?;

    // Store the changes in this vector.
    let mut changes: Vec<String> = vec![];

    // Skip to the "Unreleased" section.
    let mut old = changelog_old.split('\n').peekable();
    while old.next().unwrap() != UNRELEASED_HEADER {}
    old.next(); // Remove the succeeding newline.
    old.next(); // Remove the succeeding sub-section header.

    let mut new = changelog_new.split('\n');
    while new.next().unwrap() != UNRELEASED_HEADER {}

    // Find the lines in "new" that do not exist in "old".
    for line in new {
        if line.starts_with("## ") {
            // Start of first versioned section.
            break;
        } else if line.is_empty() {
            // Don't add blank lines automatically.
            continue;
        } else if let Some(s) = line.strip_prefix("### ") {
            // If the line starts a new sub-section while the last sub-section is empty, remove the
            // last sub-section. Then add the new sub-section header.
            if let Some(s) = changes.last() {
                if s.starts_with("\n**") {
                    changes.pop();
                }
            }
            changes.push("\n**".to_string() + s + "**")
        } else if &line != old.peek().unwrap() {
            // If the new line is not equal to the old line, add it. However, if the line does not
            // start with a bullet point, add it to the previous line.
            if line.starts_with("- ") {
                changes.push(line.to_string());
            } else {
                changes.last_mut().unwrap().push_str(&line[1..]);
            }
        } else {
            // If the two lines are equal, advance both of them. Also keep advancing the old
            // iterator over empty lines and sub-section headers.
            old.next();
            loop {
                let next = old.peek().unwrap();
                if next.is_empty() || next.starts_with("### ") {
                    old.next();
                } else {
                    break;
                }
            }
        }
    }

    // If the last sub-section is empty, remove the last sub-section.
    if let Some(s) = changes.last() {
        if s.starts_with("\n**") {
            changes.pop();
        }
    }

    // If any changes have occured, message the channel.
    if !changes.is_empty() {
        let discord_token = fs::read_to_string("DISCORD_TOKEN").unwrap();
        let mut client = Client::builder(
            &discord_token,
            serenity::model::gateway::GatewayIntents::default(),
        )
        .event_handler(Handler {
            message: "New Veloren Update!\n".to_string() + &changes.join("\n"),
        })
        .await
        .expect("Unable to start the bot.");

        // Save the shard manager for shutting down soon(tm). See note by SHARD_MANAGER for more
        // information about this unsafe block.
        unsafe {
            SHARD_MANAGER = Some(client.shard_manager.clone());
        }

        if let Err(e) = client.start().await {
            println!("Bot crashed due to error: {:?}", e);
        }
    }

    Ok(())
}

async fn download_changelog() -> reqwest::Result<String> {
    fs::write(
        CHANGELOG_PATH,
        reqwest::get(CHANGELOG_URL).await?.text().await?,
    )
    .expect("Unable to write to file.");
    Ok(read_changelog().await.unwrap())
}

async fn read_changelog() -> io::Result<String> {
    fs::read_to_string(CHANGELOG_PATH)
}

struct Handler {
    message: String,
}

#[serenity::async_trait]
impl EventHandler for Handler {
    async fn ready(&self, context: Context, _: Ready) {
        for guild_id in context.cache.guilds() {
            for (_, channel) in guild_id.channels(&context.http).await.unwrap() {
                if channel.name == "veloren-updates" {
                    channel.say(&context.http, &self.message).await.unwrap();
                }
            }
        }

        // Close the shards and consequently the bot. See note by SHARD_MANAGER for more
        // information about this unsafe block.
        unsafe {
            if let Some(sm) = &SHARD_MANAGER {
                sm.lock().await.shutdown_all().await;
            }
        }
    }
}
