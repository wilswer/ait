use std::error::Error;
use std::fs;

use genai::chat::{ChatMessage, ChatRequest};
use genai::Client;

pub const MODELS: [&str; 4] = [
    "gpt-4o-mini",
    "gpt-4o",
    "claude-3-haiku-20240307",
    "claude-3-5-sonnet-20240620",
];

// NOTE: Model to AdapterKind (AI Provider) type mapping rule
//  - starts_with "gpt"      -> OpenAI
//  - starts_with "claude"   -> Anthropic
//  - starts_with "command"  -> Cohere
//  - starts_with "gemini"   -> Gemini
//  - model in Groq models   -> Groq
//  - For anything else      -> Ollama
//
// Can be customized, see `examples/c03-kind.rs`

pub async fn assistant_response(
    messages: Vec<String>,
    model: &str,
) -> Result<String, Box<dyn Error + Send + Sync>> {
    fs::write(".chat.log", messages.join("\n")).expect("");
    let chat_messages = messages
        .iter()
        .enumerate()
        .map(|(i, m)| {
            if i % 2 == 0 {
                ChatMessage::user(m)
            } else {
                ChatMessage::assistant(m)
            }
        })
        .collect::<Vec<ChatMessage>>();
    let mut chat_req = ChatRequest::new(vec![
        // -- Messages (de/activate to see the differences)
        ChatMessage::system("You are a helpful and friendly assistant."),
    ]);

    for chat_message in chat_messages {
        chat_req = chat_req.append_message(chat_message);
    }

    let client = Client::default();
    let chat_res = match client.exec_chat(model, chat_req, None).await {
        Ok(res) => res.content_text_into_string(),
        Err(e) => Some(format!("Error: {}", e)),
    };
    let raw_chat_res_text = chat_res.unwrap_or("NO RESPONSE".to_string());
    let chat_res_text = textwrap::wrap(&raw_chat_res_text, 140).join("\n");
    Ok(chat_res_text)
}
