use ratatui::widgets::ListState;

#[derive(Debug)]
pub struct ChatList {
    pub items: Vec<ChatItem>,
    pub state: ListState,
}

#[derive(Debug)]
pub struct ChatItem {
    pub chat_id: i64,
    pub started_at: String,
    pub selected: bool,
}

impl FromIterator<(i64, String, bool)> for ChatList {
    fn from_iter<I: IntoIterator<Item = (i64, String, bool)>>(iter: I) -> Self {
        let items: Vec<ChatItem> = iter
            .into_iter()
            .map(|(id, started_at, selected)| ChatItem::new(id, started_at, selected))
            .collect();

        let index = items
            .iter()
            .enumerate()
            .find(|(_, item)| item.selected) // assuming ChatItem has a `selected` field
            .map(|(idx, _)| idx)
            .or(Some(0));

        let mut state = ListState::default();
        state.select(index);
        Self { items, state }
    }
}

impl ChatItem {
    pub fn new(chat_id: i64, started_at: String, selected: bool) -> Self {
        Self {
            chat_id,
            started_at,
            selected,
        }
    }
}
