use std::error::Error;

use genai::chat::{ChatMessage, ChatRequest};
use genai::Client;

const MODEL_OPENAI: &str = "gpt-4o-mini";

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
    query: String,
    messages: Vec<String>,
) -> Result<String, Box<dyn Error + Send + Sync>> {
    let chat_messages = messages
        .iter()
        .map(|m| {
            if m.starts_with("USER:") {
                ChatMessage::user(m.clone())
            } else if m.starts_with("ASSISTANT:") {
                ChatMessage::assistant(m.clone())
            } else {
                panic!("Unknown message type: {}", m);
            }
        })
        .collect::<Vec<ChatMessage>>();
    let mut chat_req = ChatRequest::new(vec![
        // -- Messages (de/activate to see the differences)
        ChatMessage::system("You are a helpful, consise, and friendly assistant."),
    ]);

    for chat_message in chat_messages {
        chat_req = chat_req.append_message(chat_message);
    }
    chat_req = chat_req.append_message(ChatMessage::user(query));

    let client = Client::default();
    let chat_res = client.exec_chat(MODEL_OPENAI, chat_req, None).await?;
    let chat_res_text = chat_res
        .content_text_into_string()
        .unwrap_or("NO RESPONSE".to_string());
    Ok(chat_res_text)
}
