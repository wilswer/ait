use genai::adapter::AdapterKind;
use genai::chat::{ChatMessage, ChatOptions, ChatRequest, ChatStream};
use genai::{Client, ClientBuilder, ClientConfig};

use crate::app::{AppResult, Message};

pub const MODELS: [(&str, &str); 7] = [
    ("OpenAI", "gpt-4o-mini"),
    ("OpenAI", "gpt-4o"),
    ("Anthropic", "claude-3-5-sonnet-latest"),
    ("Anthropic", "claude-3-haiku-20240307"),
    ("Anthropic", "claude-3-7-sonnet-latest"),
    ("Gemini", "gemini-2.5-pro-preview-05-06"),
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
        AdapterKind::Xai => "XAI_API_KEY",
        AdapterKind::DeepSeek => "DEEPSEEK_API_KEY",
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
    temperature: Option<f64>,
) -> AppResult<Message> {
    let chat_messages = messages
        .iter()
        .map(|m| match m {
            Message::User(m) => ChatMessage::user(m),
            Message::Assistant(m) => ChatMessage::assistant(m),
            _ => ChatMessage::assistant(""),
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
    let chat_opts = if let Some(temp) = temperature {
        ChatOptions::default().with_temperature(temp)
    } else {
        ChatOptions::default()
    };
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

pub async fn assistant_response_streaming(
    messages: &[Message],
    model: &str,
    system_prompt: Option<String>,
    temperature: Option<f64>,
) -> AppResult<ChatStream> {
    let chat_messages = messages
        .iter()
        .map(|m| match m {
            Message::User(m) => ChatMessage::user(m),
            Message::Assistant(m) => ChatMessage::assistant(m),
            _ => ChatMessage::assistant(""),
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
    let chat_opts = if let Some(temp) = temperature {
        ChatOptions::default().with_temperature(temp)
    } else {
        ChatOptions::default()
    };
    let client_config = ClientConfig::default().with_chat_options(chat_opts);

    let client = ClientBuilder::default().with_config(client_config).build();
    let chat_res = client.exec_chat_stream(model, chat_req, None).await?;
    Ok(chat_res.stream)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::Message;
    use futures::StreamExt;
    use genai::chat::{ChatStreamEvent, StreamChunk};
    use std::env;

    #[tokio::test]
    async fn test_assistant_response_streaming() {
        // Set up test environment
        // You might need to set an API key for the test to work
        if env::var("OPENAI_API_KEY").is_err() {
            println!("Skipping test_assistant_response_streaming: OPENAI_API_KEY not set");
            return;
        }

        // Create test messages
        let messages = vec![Message::User("Hello, how are you?".to_string())];

        // Use a model that's likely to be available
        let model = "o3-mini-low";
        let system_prompt = Some("You are a helpful assistant.".to_string());
        let temperature = Some(0.7);

        // Get streaming response
        let result =
            assistant_response_streaming(&messages, model, system_prompt, temperature).await;

        // Check if we got a valid stream
        assert!(
            result.is_ok(),
            "Failed to get streaming response: {:?}",
            result.err()
        );

        let mut stream = result.unwrap();

        // Collect some chunks to verify the stream works
        let mut chunks_received = 0;

        // Process up to 5 chunks or until stream ends
        while let Some(chunk) = stream.next().await {
            match chunk {
                Ok(chunk) => {
                    // Check if we received any content
                    match chunk {
                        ChatStreamEvent::Start => {}
                        ChatStreamEvent::Chunk(StreamChunk { content })
                        | ChatStreamEvent::ReasoningChunk(StreamChunk { content }) => {
                            if !content.is_empty() {
                                chunks_received += 1;
                            }
                        }
                        ChatStreamEvent::End(_) => {
                            break;
                        }
                    }
                    if chunks_received >= 5 {
                        break; // Don't process the entire response
                    }
                }
                Err(e) => {
                    panic!("Error in stream: {:?}", e);
                }
            }
        }

        assert!(chunks_received > 0, "No chunks received from stream");
    }
}
