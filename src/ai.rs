use genai::adapter::AdapterKind;
use genai::chat::{ChatMessage, ChatOptions, ChatRequest, ChatStream};
use genai::{Client, ClientBuilder, ClientConfig};

use crate::app::{AppResult, Message};

pub const MODELS: [(&str, &str); 7] = [
    ("OpenAI", "gpt-5-mini"),
    ("OpenAI", "gpt-5-nano"),
    ("Anthropic", "claude-haiku-4-5"),
    ("Anthropic", "claude-sonnet-4-6"),
    ("Anthropic", "claude-opus-4-6"),
    ("Gemini", "gemini-3-pro-preview"),
    ("Gemini", "gemini-3.1-pro-preview"),
];

fn get_api_key_name(kind: &AdapterKind) -> &'static str {
    match kind {
        AdapterKind::OpenAI | AdapterKind::OpenAIResp => "OPENAI_API_KEY",
        AdapterKind::Ollama => "",
        AdapterKind::Gemini => "GEMINI_API_KEY",
        AdapterKind::Anthropic => "ANTHROPIC_API_KEY",
        AdapterKind::Groq => "GROQ_API_KEY",
        AdapterKind::Cohere => "COHERE_API_KEY",
        AdapterKind::Xai => "XAI_API_KEY",
        AdapterKind::DeepSeek => "DEEPSEEK_API_KEY",
        AdapterKind::Fireworks => "FIREWORKS_API_KEY",
        AdapterKind::Together => "TOGETHER_API_KEY",
        AdapterKind::Nebius => "NEBIUS_API_KEY",
        AdapterKind::Zai => "ZAI_API_KEY",
        AdapterKind::BigModel => "BIGMODEL_API_KEY",
        AdapterKind::Mimo => "MIMO_API_KEY",
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
        AdapterKind::Xai,
        AdapterKind::DeepSeek,
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
    for (p, m) in MODELS {
        if !models.contains(&(p.to_string(), m.to_string())) {
            models.push((p.to_string(), m.to_string()));
        }
    }
    models.sort();
    Ok(models)
}

pub async fn assistant_response(
    messages: &[Message],
    model: &str,
    system_prompt: Option<String>,
) -> AppResult<Message> {
    let chat_messages = messages
        .iter()
        .map(|m| match m {
            Message::User(m) => ChatMessage::user(m),
            Message::Assistant(m) => ChatMessage::assistant(m),
        })
        .collect::<Vec<ChatMessage>>();
    let mut chat_req = if let Some(system_prompt) = system_prompt {
        ChatRequest::new(vec![ChatMessage::system(system_prompt)])
    } else {
        ChatRequest::new(vec![])
    };

    for chat_message in chat_messages {
        chat_req = chat_req.append_message(chat_message);
    }
    let chat_opts = ChatOptions::default();
    let client_config = ClientConfig::default().with_chat_options(chat_opts);

    let client = ClientBuilder::default().with_config(client_config).build();
    match client.exec_chat(model, chat_req, None).await {
        Ok(res) => {
            let chat_res = if let Some(m) = res.into_first_text() {
                Message::Assistant(m)
            } else {
                Message::Assistant("NO RESPONSE".to_string())
            };
            Ok(chat_res)
        }
        Err(e) => Err(e.into()),
    }
}

pub async fn assistant_response_streaming(
    messages: &[Message],
    model: &str,
    system_prompt: Option<String>,
) -> AppResult<ChatStream> {
    let chat_messages = messages
        .iter()
        .map(|m| match m {
            Message::User(m) => ChatMessage::user(m),
            Message::Assistant(m) => ChatMessage::assistant(m),
        })
        .collect::<Vec<ChatMessage>>();
    let mut chat_req = if let Some(system_prompt) = system_prompt {
        ChatRequest::new(vec![ChatMessage::system(system_prompt)])
    } else {
        ChatRequest::new(vec![])
    };

    for chat_message in chat_messages {
        chat_req = chat_req.append_message(chat_message);
    }
    let chat_opts = ChatOptions::default();
    let client_config = ClientConfig::default().with_chat_options(chat_opts);

    let client = ClientBuilder::default().with_config(client_config).build();
    let chat_res = client.exec_chat_stream(model, chat_req, None).await?;
    Ok(chat_res.stream)
}
