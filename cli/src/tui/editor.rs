//! The line being typed at a window's prompt.

/// Text and a cursor, as a byte offset always on a character boundary.
#[derive(Debug, Default)]
pub struct Editor {
    text: String,
    cursor: usize,
    /// Which history entry is showing, and the line it replaced.
    recall: Option<(usize, String)>,
}

impl Editor {
    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn before_cursor(&self) -> &str {
        &self.text[..self.cursor]
    }

    pub fn at_end(&self) -> bool {
        self.cursor == self.text.len()
    }

    pub fn insert(&mut self, text: &str) {
        self.text.insert_str(self.cursor, text);
        self.cursor += text.len();
        self.recall = None;
    }

    pub fn take(&mut self) -> String {
        self.cursor = 0;
        self.recall = None;
        std::mem::take(&mut self.text)
    }

    /// Replace what lies between `start` and the cursor.
    pub fn replace_to_cursor(&mut self, start: usize, text: &str) {
        self.text.replace_range(start..self.cursor, text);
        self.cursor = start + text.len();
        self.recall = None;
    }

    pub fn left(&mut self) {
        self.cursor = self.previous_boundary();
    }

    pub fn right(&mut self) {
        self.cursor = self.next_boundary();
    }

    pub fn home(&mut self) {
        self.cursor = 0;
    }

    pub fn end(&mut self) {
        self.cursor = self.text.len();
    }

    pub fn word_left(&mut self) {
        self.cursor = self.previous_word();
    }

    pub fn word_right(&mut self) {
        let rest = &self.text[self.cursor..];
        let spaces = rest.len() - rest.trim_start().len();
        let word = rest[spaces..]
            .find(char::is_whitespace)
            .unwrap_or(rest.len() - spaces);
        self.cursor += spaces + word;
    }

    pub fn backspace(&mut self) {
        self.remove(self.previous_boundary(), self.cursor);
    }

    pub fn delete(&mut self) {
        self.remove(self.cursor, self.next_boundary());
    }

    pub fn delete_word(&mut self) {
        self.remove(self.previous_word(), self.cursor);
    }

    pub fn delete_to_start(&mut self) {
        self.remove(0, self.cursor);
    }

    pub fn delete_to_end(&mut self) {
        self.remove(self.cursor, self.text.len());
    }

    /// One entry further back in `history`.
    pub fn recall_previous(&mut self, history: &[String]) {
        let entry = match &self.recall {
            Some((entry, _)) => entry.saturating_sub(1),
            None if history.is_empty() => return,
            None => history.len() - 1,
        };
        let draft = match self.recall.take() {
            Some((_, draft)) => draft,
            None => self.text.clone(),
        };
        self.show(history[entry].clone());
        self.recall = Some((entry, draft));
    }

    /// One entry forward in `history`, ending at the line it replaced.
    pub fn recall_next(&mut self, history: &[String]) {
        let Some((entry, draft)) = self.recall.take() else {
            return;
        };
        if entry + 1 < history.len() {
            self.show(history[entry + 1].clone());
            self.recall = Some((entry + 1, draft));
        } else {
            self.show(draft);
        }
    }

    fn show(&mut self, text: String) {
        self.text = text;
        self.cursor = self.text.len();
    }

    fn remove(&mut self, start: usize, end: usize) {
        self.text.replace_range(start..end, "");
        self.cursor = start;
        self.recall = None;
    }

    fn previous_boundary(&self) -> usize {
        self.text[..self.cursor]
            .char_indices()
            .next_back()
            .map_or(0, |(at, _)| at)
    }

    fn next_boundary(&self) -> usize {
        self.text[self.cursor..]
            .chars()
            .next()
            .map_or(self.cursor, |c| self.cursor + c.len_utf8())
    }

    fn previous_word(&self) -> usize {
        let trimmed = self.text[..self.cursor].trim_end();
        trimmed
            .char_indices()
            .rfind(|(_, c)| c.is_whitespace())
            .map_or(0, |(at, c)| at + c.len_utf8())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn editor(text: &str) -> Editor {
        let mut editor = Editor::default();
        editor.insert(text);
        editor
    }

    #[test]
    fn the_cursor_moves_by_character_not_byte() {
        let mut line = editor("钱包a");
        line.left();
        line.left();
        assert_eq!(line.before_cursor(), "钱");
        line.backspace();
        assert_eq!(line.text(), "包a");
        line.right();
        line.delete();
        assert_eq!(line.text(), "包");
    }

    #[test]
    fn word_motions_skip_the_spaces_around_a_word() {
        let mut line = editor("wallet  show main");
        line.word_left();
        assert_eq!(line.before_cursor(), "wallet  show ");
        line.word_left();
        assert_eq!(line.before_cursor(), "wallet  ");
        line.word_right();
        assert_eq!(line.before_cursor(), "wallet  show");
        line.delete_word();
        assert_eq!(line.text(), "wallet   main");
    }

    #[test]
    fn deleting_to_either_end_keeps_the_other_side() {
        let mut line = editor("send build");
        line.word_left();
        line.delete_to_start();
        assert_eq!(line.text(), "build");
        line.right();
        line.delete_to_end();
        assert_eq!(line.text(), "b");
    }

    #[test]
    fn history_walks_back_and_returns_to_the_draft() {
        let history = ["wallet list".to_string(), "chains".to_string()];
        let mut line = editor("draft");
        line.recall_previous(&history);
        assert_eq!(line.text(), "chains");
        line.recall_previous(&history);
        assert_eq!(line.text(), "wallet list");
        line.recall_previous(&history);
        assert_eq!(line.text(), "wallet list", "stops at the oldest");
        line.recall_next(&history);
        assert_eq!(line.text(), "chains");
        line.recall_next(&history);
        assert_eq!(line.text(), "draft");
        line.recall_next(&history);
        assert_eq!(line.text(), "draft", "nothing past the draft");
    }

    #[test]
    fn empty_history_changes_nothing() {
        let mut line = editor("draft");
        line.recall_previous(&[]);
        assert_eq!(line.text(), "draft");
    }

    #[test]
    fn replacing_to_the_cursor_keeps_what_follows() {
        let mut line = editor("wal list");
        line.home();
        for _ in 0..3 {
            line.right();
        }
        line.replace_to_cursor(0, "wallet");
        assert_eq!(line.text(), "wallet list");
        assert_eq!(line.before_cursor(), "wallet");
    }
}
