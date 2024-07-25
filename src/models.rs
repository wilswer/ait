use ratatui::{
    text::{Line, Span},
    widgets::{ListItem, ListState},
};

pub struct ModelList {
    pub items: Vec<ModelItem>,
    pub state: ListState,
}

#[derive(Debug)]
pub struct ModelItem {
    pub name: String,
    pub selected: bool,
}

impl FromIterator<(&'static str, bool)> for ModelList {
    fn from_iter<I: IntoIterator<Item = (&'static str, bool)>>(iter: I) -> Self {
        let items = iter
            .into_iter()
            .map(|(name, selected)| ModelItem::new(name, selected))
            .collect();
        let mut state = ListState::default();
        state.select_first();
        Self { items, state }
    }
}

impl ModelItem {
    pub fn new(model: &str, selected: bool) -> Self {
        Self {
            name: model.to_string(),
            selected,
        }
    }
}

impl From<&ModelItem> for ListItem<'_> {
    fn from(value: &ModelItem) -> Self {
        let line = Line::from(Span::raw(value.name.clone()));
        ListItem::new(line)
    }
}
