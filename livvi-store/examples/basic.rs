use anyhow::Result;
use livvi_store::{ConversationStorage, MockStore, PersonStorage};

/// Basic usage example for `livvi-store`.
///
/// This uses the in-memory `MockStore`; run with:
///
/// ```bash
/// cargo run -p livvi-store --example basic
/// ```
///
/// To try the SQLite backend instead, enable the `sqlite` feature:
///
/// ```bash
/// cargo run -p livvi-store --example basic --features sqlite
/// ```
#[tokio::main]
async fn main() -> Result<()> {
    let store = MockStore::new();

    let person = store
        .ensure_identity(
            "discord",
            "123456789",
            Some("hayden".to_string()),
            serde_json::json!({ "discriminator": "0001" }),
        )
        .await?;

    let conversation = store
        .ensure_conversation(
            "discord",
            "987654321",
            Some("general".to_string()),
            serde_json::json!({ "guild_id": "11111" }),
        )
        .await?;

    store.add_participant(&conversation.id, &person.id).await?;

    let participants = store.get_participants(&conversation.id).await?;
    println!("Participants: {:?}", participants);

    Ok(())
}
