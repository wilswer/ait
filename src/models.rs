use genai::{ModelIden, ModelName, ModelSpec, adapter::AdapterKind};
use ratatui::{
    text::{Line, Span},
    widgets::{ListItem, ListState},
};

pub struct ModelList {
    pub items: Vec<ModelItem>,
    pub state: ListState,
}

#[derive(Debug, Clone)]
pub struct ModelItem {
    pub provider: String,
    pub name: String,
    pub selected: bool,
}

impl FromIterator<(&'static str, &'static str, bool)> for ModelList {
    fn from_iter<I: IntoIterator<Item = (&'static str, &'static str, bool)>>(iter: I) -> Self {
        let items = iter
            .into_iter()
            .map(|(provider, name, selected)| ModelItem::new(provider, name, selected))
            .collect();
        let mut state = ListState::default();
        state.select_first();
        Self { items, state }
    }
}

impl FromIterator<(String, String, bool)> for ModelList {
    fn from_iter<I: IntoIterator<Item = (String, String, bool)>>(iter: I) -> Self {
        let items = iter
            .into_iter()
            .map(|(provider, name, selected)| ModelItem::new(&provider, &name, selected))
            .collect();
        let mut state = ListState::default();
        state.select_first();
        Self { items, state }
    }
}

impl ModelItem {
    pub fn new(provider: &str, name: &str, selected: bool) -> Self {
        Self {
            provider: provider.to_string(),
            name: name.to_string(),
            selected,
        }
    }
}

impl From<&ModelItem> for ListItem<'_> {
    fn from(value: &ModelItem) -> Self {
        let line = Line::from(Span::raw(format!("{}: {}", value.provider, value.name)));
        ListItem::new(line)
    }
}

pub fn generate_model_spec(name: &str, provider: &str) -> ModelSpec {
    match provider {
        "OpenAI" => {
            ModelSpec::from_iden(ModelIden::new(AdapterKind::OpenAI, ModelName::from(name)))
        }
        "Anthropic" => ModelSpec::from_iden(ModelIden::new(
            AdapterKind::Anthropic,
            ModelName::from(name),
        )),
        "Gemini" => {
            ModelSpec::from_iden(ModelIden::new(AdapterKind::Gemini, ModelName::from(name)))
        }
        "Ollama" => {
            ModelSpec::from_iden(ModelIden::new(AdapterKind::Ollama, ModelName::from(name)))
        }
        "OpenRouter" => ModelSpec::from_iden(ModelIden::new(
            AdapterKind::OpenRouter,
            ModelName::from(name),
        )),
        _ => ModelSpec::from_name(name),
    }
}
