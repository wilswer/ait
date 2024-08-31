use genai::adapter::AdapterKind;
use genai::chat::{ChatMessage, ChatOptions, ChatRequest};
use genai::{Client, ClientBuilder, ClientConfig};

use crate::app::{AppResult, Message};

pub const MODELS: [(&str, &str); 5] = [
    ("OpenAI", "gpt-4o-mini"),
    ("OpenAI", "gpt-4o"),
    ("Anthropic", "claude-3-haiku-20240307"),
    ("Anthropic", "claude-3-5-sonnet-20240620"),
    ("Ollama", "gemma:2b"),
];

fn get_api_key_name(kind: &AdapterKind) -> &'static str {
    match kind {
        AdapterKind::OpenAI => "OPENAI_API_KEY",
        AdapterKind::Ollama => "",
        AdapterKind::Gemini => "GEMINI_API_KEY",
        AdapterKind::Anthropic => "ANTHROPIC_API_KEY",
        AdapterKind::Groq => "GROQ_API_KEY",
        AdapterKind::Cohere => "COHERE_API_KEY",
    }
}

pub async fn get_models() -> AppResult<Vec<(String, String)>> {
    const KINDS: &[AdapterKind] = &[
        AdapterKind::OpenAI,
        AdapterKind::Ollama,
        AdapterKind::Gemini,
        AdapterKind::Anthropic,
        AdapterKind::Groq,
        AdapterKind::Cohere,
    ];

    let client = Client::default();
    let mut models = Vec::new();
    for &kind in KINDS {
        let env_name = get_api_key_name(&kind);
        if !env_name.is_empty() && std::env::var(env_name).is_err() {
            continue;
        }
        let models_provider_res = client.all_model_names(kind).await;
        let models_provider = match models_provider_res {
            Ok(m) => m
                .into_iter()
                .map(|m| (kind.as_str().to_string(), m))
                .collect::<Vec<(String, String)>>(),
            Err(_) => Vec::new(),
        };
        models.extend(models_provider);
    }
    Ok(models)
}

pub async fn assistant_response(
    messages: &[Message],
    model: &str,
    system_prompt: &str,
    temperature: &f64,
) -> AppResult<Message> {
    let chat_messages = messages
        .iter()
        .map(|m| match m {
            Message::User(m) => ChatMessage::user(m),
            Message::Assistant(m) => ChatMessage::assistant(m),
            _ => ChatMessage::assistant(""),
        })
        .collect::<Vec<ChatMessage>>();
    let mut chat_req = ChatRequest::new(vec![ChatMessage::system(system_prompt)]);

    for chat_message in chat_messages {
        chat_req = chat_req.append_message(chat_message);
    }

    let chat_opts = ChatOptions::default().with_temperature(*temperature);
    let client_config = ClientConfig::default().with_chat_options(chat_opts);

    let client = ClientBuilder::default().with_config(client_config).build();
    let chat_res = match client.exec_chat(model, chat_req, None).await {
        Ok(res) => {
            if let Some(m) = res.content_text_into_string() {
                Message::Assistant(m)
            } else {
                Message::Assistant("NO RESPONSE".to_string())
            }
        }
        Err(e) => Message::Error(format!("Error: {}", e)),
    };

    Ok(chat_res)
}
