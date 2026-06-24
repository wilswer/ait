use genai::adapter::AdapterKind;
use genai::chat::{ChatMessage, ChatOptions, ChatRequest, ChatStream, ReasoningEffort};
use genai::resolver::{AuthData, Endpoint, ProviderConfig, ServiceTargetResolver};
use genai::{ClientBuilder, ClientConfig, ModelIden, ServiceTarget};

use crate::app::{AppResult, Message, ThinkingEffort};

pub const MODELS: [(&str, &str); 1] = [("Gemini", "gemini-3.1-pro-preview")];

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
        _ => todo!(),
    }
}

fn init_clientbuilder(ollama_host_url: Option<&str>, chat_options: ChatOptions) -> ClientBuilder {
    let client_config = if let Some(host) = ollama_host_url {
        let host = host.to_string();
        let resolver = ServiceTargetResolver::from_resolver_fn(
            move |service_target: ServiceTarget| -> Result<ServiceTarget, genai::resolver::Error> {
                if service_target.model.adapter_kind == AdapterKind::Ollama {
                    let endpoint = Endpoint::from_owned(host.clone());
                    let auth = AuthData::from_single("ollama");
                    let model =
                        ModelIden::new(AdapterKind::Ollama, service_target.model.model_name);
                    Ok(ServiceTarget {
                        endpoint,
                        auth,
                        model,
                    })
                } else {
                    Ok(service_target)
                }
            },
        );
        ClientConfig::default()
            .with_chat_options(chat_options)
            .with_service_target_resolver(resolver)
    } else {
        ClientConfig::default().with_chat_options(chat_options)
    };
    ClientBuilder::default().with_config(client_config)
}

pub async fn get_models(ollama_host_url: Option<&str>) -> AppResult<Vec<(String, String)>> {
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

    let client = init_clientbuilder(ollama_host_url, ChatOptions::default()).build();
    let mut models = Vec::new();
    for &kind in KINDS {
        let env_name = get_api_key_name(&kind);
        if !env_name.is_empty() && std::env::var(env_name).is_err() {
            continue;
        }
        let provider_config = if let Some(host_url) = ollama_host_url
            && kind == AdapterKind::Ollama
        {
            let endpoint = Endpoint::from_owned(host_url);
            ProviderConfig::from_endpoint(endpoint)
        } else {
            ProviderConfig::default()
        };
        let models_provider_res = client.all_model_names(kind, provider_config).await;
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
    ollama_host_url: Option<&str>,
) -> AppResult<Message> {
    let chat_messages = messages
        .iter()
        .map(|m| match m {
            Message::User(m) => ChatMessage::user(m.clone()),
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
    let client = init_clientbuilder(ollama_host_url, ChatOptions::default()).build();
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
    thinking_effort: ThinkingEffort,
    ollama_host_url: Option<String>,
) -> AppResult<ChatStream> {
    let chat_messages = messages
        .iter()
        .map(|m| match m {
            Message::User(m) => ChatMessage::user(m.clone()),
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

    let chat_opts = match thinking_effort {
        ThinkingEffort::None => ChatOptions::default(),
        ThinkingEffort::Low => ChatOptions::default().with_reasoning_effort(ReasoningEffort::Low),
        ThinkingEffort::Medium => {
            ChatOptions::default().with_reasoning_effort(ReasoningEffort::Medium)
        }
        ThinkingEffort::High => ChatOptions::default().with_reasoning_effort(ReasoningEffort::High),
        ThinkingEffort::XHigh => {
            ChatOptions::default().with_reasoning_effort(ReasoningEffort::XHigh)
        }
        ThinkingEffort::Max => ChatOptions::default().with_reasoning_effort(ReasoningEffort::Max),
    };

    let clientbuilder = init_clientbuilder(ollama_host_url.as_deref(), chat_opts);
    let client = clientbuilder.build();
    let chat_res = client.exec_chat_stream(model, chat_req, None).await?;
    Ok(chat_res.stream)
}
