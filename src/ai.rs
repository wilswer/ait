use genai::chat::{ChatMessage, ChatRequest};
use genai::Client;

const MODEL_OPENAI: &str = "gpt-4o";

// NOTE: Model to AdapterKind (AI Provider) type mapping rule
//  - starts_with "gpt"      -> OpenAI
//  - starts_with "claude"   -> Anthropic
//  - starts_with "command"  -> Cohere
//  - starts_with "gemini"   -> Gemini
//  - model in Groq models   -> Groq
//  - For anything else      -> Ollama
//
// Can be customized, see `examples/c03-kind.rs`

pub async fn bot_response(question: &str) -> Result<String, Box<dyn std::error::Error>> {
    let chat_req = ChatRequest::new(vec![
        // -- Messages (de/activate to see the differences)
        ChatMessage::system("Answer in one sentence"),
        ChatMessage::user(question),
    ]);
    let client = Client::default();
    let chat_res = client
        .exec_chat(MODEL_OPENAI, chat_req.clone(), None)
        .await?;
    let chat_res = chat_res.content_text_as_str().unwrap_or("NO ANSWER");
    Ok(chat_res.into())
}
