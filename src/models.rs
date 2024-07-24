use ratatui::widgets::ListState;

pub struct ModelList {
    items: Vec<ModelItem>,
    state: ListState,
}

#[derive(Debug)]
struct ModelItem {
    name: String,
    selected: bool,
}

impl FromIterator<(&'static str, bool)> for ModelList {
    fn from_iter<I: IntoIterator<Item = (&'static str, bool)>>(iter: I) -> Self {
        let items = iter
            .into_iter()
            .map(|(name, selected)| ModelItem::new(name, selected))
            .collect();
        let state = ListState::default();
        Self { items, state }
    }
}

impl ModelItem {
    fn new(model: &str, selected: bool) -> Self {
        Self {
            name: model.to_string(),
            selected,
        }
    }
}
