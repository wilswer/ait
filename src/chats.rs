use ratatui::widgets::ListState;

#[derive(Debug)]
pub struct ChatList {
    pub items: Vec<ChatItem>,
    pub state: ListState,
}

#[derive(Debug)]
pub struct ChatItem {
    pub chat_id: i64,
    pub selected: bool,
}

impl FromIterator<(i64, bool)> for ChatList {
    fn from_iter<I: IntoIterator<Item = (i64, bool)>>(iter: I) -> Self {
        let items = iter
            .into_iter()
            .map(|(id, selected)| ChatItem::new(id, selected))
            .collect();
        let mut state = ListState::default();
        state.select_first();
        Self { items, state }
    }
}

impl ChatItem {
    pub fn new(chat_id: i64, selected: bool) -> Self {
        Self { chat_id, selected }
    }
}
