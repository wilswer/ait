use std::fs;
use std::path::PathBuf;

use anyhow::Context;
use directories::ProjectDirs;
use genai::chat::ContentPart;
use rusqlite::{params, Connection};

use crate::app::{AppResult, Message};

fn get_data_dir() -> AppResult<PathBuf> {
    let project_dirs =
        ProjectDirs::from("", "", "ait").context("Could not determine project directories")?;
    Ok(project_dirs.data_dir().to_path_buf())
}

pub fn get_cache_dir() -> AppResult<PathBuf> {
    let project_dirs =
        ProjectDirs::from("", "", "ait").context("Could not determine project directories")?;
    Ok(project_dirs.cache_dir().to_path_buf())
}

fn get_db_path() -> AppResult<PathBuf> {
    let mut path = get_data_dir()?;
    path.push("chats.db");
    Ok(path)
}

pub fn create_db() -> AppResult<()> {
    // Connect to the SQLite database (or create it if it doesn't exist)
    let data_dir = get_data_dir()?;
    fs::create_dir_all(&data_dir).context("Could not create data directory")?;
    let db_path = get_db_path()?;
    let conn = Connection::open(db_path).context("Could not open db connection")?;

    // Create the Conversations table
    conn.execute(
        "CREATE TABLE IF NOT EXISTS Conversations (
            conversation_id INTEGER PRIMARY KEY AUTOINCREMENT,
            system_prompt TEXT NOT NULL,
            started_at DATETIME DEFAULT CURRENT_TIMESTAMP,
            updated_at DATETIME
        )",
        [],
    )
    .context("Failed to create conversations table")?;

    // Create the Messages table
    conn.execute(
        "CREATE TABLE IF NOT EXISTS Messages (
            message_id INTEGER PRIMARY KEY AUTOINCREMENT,
            conversation_id INTEGER,
            sender TEXT CHECK(sender IN ('human', 'assistant')),
            message_text TEXT NOT NULL,
            timestamp DATETIME DEFAULT CURRENT_TIMESTAMP,
            FOREIGN KEY(conversation_id) REFERENCES Conversations(conversation_id)
        )",
        [],
    )
    .context("Failed to create messages table")?;

    Ok(())
}

pub fn migrate_db() -> AppResult<()> {
    let db_path = get_db_path()?;
    let conn = Connection::open(db_path).context("Could not open db connection")?;

    // Add updated_at column if it doesn't exist (existing rows will have NULL)
    let column_exists: bool = conn
        .query_row(
            "SELECT COUNT(*) FROM pragma_table_info('Conversations') WHERE name = 'updated_at'",
            [],
            |row| row.get::<_, i64>(0),
        )
        .context("Failed to check for updated_at column")?
        > 0;

    if !column_exists {
        conn.execute(
            "ALTER TABLE Conversations ADD COLUMN updated_at DATETIME",
            [],
        )
        .context("Failed to add updated_at column to Conversations")?;
    }

    Ok(())
}

pub fn touch_conversation(conversation_id: i64) -> AppResult<()> {
    let db_path = get_db_path()?;
    let conn = Connection::open(db_path).context("Could not open db connection")?;
    conn.execute(
        "UPDATE Conversations SET updated_at = CURRENT_TIMESTAMP WHERE conversation_id = ?1",
        params![conversation_id],
    )
    .context("Failed to update conversation updated_at")?;
    Ok(())
}

pub fn insert_message(conversation_id: i64, message: &Message) -> AppResult<()> {
    // Connect to the SQLite database
    let db_path = get_db_path()?;
    let conn = Connection::open(db_path)?;
    // Insert the message into the Messages table
    let (sender, message_text) = match message {
        Message::User(_) => ("human", &message.to_string()),
        Message::Assistant(text) => ("assistant", text),
    };
    conn.execute(
        "INSERT INTO Messages (conversation_id, sender, message_text) VALUES (?1, ?2, ?3)",
        params![conversation_id, sender, message_text],
    )?;
    Ok(())
}

pub fn delete_message(conversation_id: i64, message: &Message) -> AppResult<()> {
    let db_path = get_db_path()?;
    let conn = Connection::open(db_path).context("Could not connect to database")?;

    let (sender, message_text) = match message {
        Message::User(_) => ("human", &message.to_string()),
        Message::Assistant(text) => ("assistant", text),
    };

    conn.execute(
        "DELETE FROM Messages WHERE conversation_id = ?1 AND sender = ?2 AND message_text = ?3",
        params![conversation_id, sender, message_text],
    )
    .context("Failed to delete message")?;

    Ok(())
}

pub fn create_db_conversation(system_prompt: &str) -> AppResult<i64> {
    // Connect to the SQLite database
    let db_path = get_db_path()?;
    let conn = Connection::open(db_path).context("Could not connect to database")?;
    conn.execute(
        "INSERT INTO Conversations (system_prompt) VALUES (?1)",
        params![system_prompt],
    )
    .context("Could not create new conversation")?;
    // Get the ID of the newly created conversation
    let conversation_id = conn.last_insert_rowid();
    Ok(conversation_id)
}

pub fn list_conversations(query_filter: Option<String>) -> AppResult<Vec<(i64, String)>> {
    // Connect to the SQLite database
    let db_path = get_db_path()?;
    let conn = Connection::open(db_path).context("Could not connect to database")?;
    let conversation_ids = if let Some(filter) = query_filter {
        let filter_param = format!("%{}%", filter);
        let mut stmt = conn.prepare(
            "SELECT DISTINCT c.conversation_id, COALESCE(c.updated_at, c.started_at)
             FROM Conversations c
             JOIN Messages m ON c.conversation_id = m.conversation_id
             WHERE m.message_text LIKE ?1
             ORDER BY COALESCE(c.updated_at, c.started_at) DESC",
        )?;
        let res = stmt
            .query_map(params![filter_param], |row| Ok((row.get(0)?, row.get(1)?)))
            .context("Failed to query conversations table with filter")?
            .collect::<rusqlite::Result<Vec<(i64, String)>>>()?;
        res
    } else {
        let mut stmt = conn.prepare(
            "SELECT conversation_id, COALESCE(updated_at, started_at) FROM Conversations ORDER BY COALESCE(updated_at, started_at) DESC",
        )?;
        let res = stmt
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .context("Failed to query conversations table")?
            .collect::<rusqlite::Result<Vec<(i64, String)>>>()?;
        res
    };

    Ok(conversation_ids)
}

pub fn list_all_messages(conversation_id: i64) -> AppResult<Vec<Message>> {
    // Connect to the SQLite database
    let db_path = get_db_path()?;
    let conn = Connection::open(db_path).context("Could not connect to database")?;
    // Query the Messages table for all messages in the specified conversation
    let mut stmt = conn.prepare("SELECT * FROM Messages WHERE conversation_id = ?1")?;
    let messages = stmt
        .query_map(params![conversation_id], |row| {
            Ok(DBMessage {
                sender: row.get(2)?,
                message_text: row.get(3)?,
            })
        })
        .context("Failed to query messages table")?
        .collect::<rusqlite::Result<Vec<DBMessage>>>()?;
    let messages = messages
        .into_iter()
        .map(Message::from)
        .collect::<Vec<Message>>();
    Ok(messages)
}

pub fn delete_conversation(conversation_id: i64) -> AppResult<()> {
    // Connect to the SQLite database
    let db_path = get_db_path()?;
    let conn = Connection::open(db_path).context("Could not connect to database")?;
    // Delete the messages from the Messages table
    conn.execute(
        "DELETE FROM Messages WHERE conversation_id = ?1",
        params![conversation_id],
    )
    .context("Failed to delete messages")?;
    // Delete the conversation from the Conversations table
    conn.execute(
        "DELETE FROM Conversations WHERE conversation_id = ?1",
        params![conversation_id],
    )
    .context("Failed to delete conversation")?;
    Ok(())
}

struct DBMessage {
    sender: String,
    message_text: String,
}

impl From<DBMessage> for Message {
    fn from(db_message: DBMessage) -> Self {
        let sender = match db_message.sender.as_str() {
            "human" => Message::User(vec![ContentPart::from_text(db_message.message_text)]),
            "assistant" => Message::Assistant(db_message.message_text),
            _ => Message::Assistant("Error".to_string()),
        };
        sender
    }
}
