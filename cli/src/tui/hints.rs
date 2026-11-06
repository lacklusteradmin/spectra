//! kitty's hints: label what is worth copying on screen, and copy it by
//! pressing the label. Here that is an address, a hash, an id or a URL.

use std::collections::VecDeque;

/// A run of cells holding one word, which may wrap across rows.
pub struct Token {
    pub text: String,
    pub cells: Vec<(u16, u16)>,
}

pub struct Hint {
    pub label: char,
    pub text: String,
    pub cells: Vec<(u16, u16)>,
}

/// The characters an address, hash, id or URL is made of. `…` is not one: an
/// abbreviated hash is not the hash, and copying it would be worse than not.
fn in_word(c: char) -> bool {
    c.is_ascii_alphanumeric() || "-_:/.?&=+%#@~".contains(c)
}

/// Long enough to be tedious to retype, and not just prose or a rule line.
fn worth_copying(text: &str) -> bool {
    text.starts_with("http://")
        || text.starts_with("https://")
        || (text.chars().count() >= 20 && text.chars().any(|c| c.is_ascii_digit()))
}

pub fn tokens(screen: &vt100::Screen) -> Vec<Token> {
    let (rows, cols) = screen.size();
    let mut tokens = Vec::new();
    let mut current = Token {
        text: String::new(),
        cells: Vec::new(),
    };
    for row in 0..rows.get() {
        for col in 0..cols.get() {
            let c = screen
                .cell(row, col)
                .and_then(|cell| cell.contents().chars().next());
            match c {
                Some(c) if in_word(c) => {
                    current.text.push(c);
                    current.cells.push((row, col));
                }
                _ => finish(&mut tokens, &mut current),
            }
        }
        if !screen.row_wrapped(row) {
            finish(&mut tokens, &mut current);
        }
    }
    finish(&mut tokens, &mut current);
    tokens
}

/// Close the current word, without the punctuation that follows one in prose.
fn finish(tokens: &mut Vec<Token>, current: &mut Token) {
    while current.text.ends_with(['.', ':', '?', '-', '=']) && !current.text.ends_with("==") {
        current.text.pop();
        current.cells.pop();
    }
    if !current.text.is_empty() {
        tokens.push(Token {
            text: std::mem::take(&mut current.text),
            cells: std::mem::take(&mut current.cells),
        });
    }
    current.text.clear();
    current.cells.clear();
}

/// What can be copied on screen, labelled from the newest up.
pub fn scan(screen: &vt100::Screen) -> Vec<Hint> {
    let mut labels: VecDeque<char> = ('a'..='z').collect();
    tokens(screen)
        .into_iter()
        .rev()
        .filter(|token| worth_copying(&token.text))
        .map_while(|token| {
            Some(Hint {
                label: labels.pop_front()?,
                text: token.text,
                cells: token.cells,
            })
        })
        .collect()
}

/// The word under a cell, for a double click.
pub fn word_at(screen: &vt100::Screen, row: u16, col: u16) -> Option<Token> {
    tokens(screen)
        .into_iter()
        .find(|token| token.cells.contains(&(row, col)))
}

#[cfg(test)]
mod tests {
    use std::num::NonZeroU16;

    use super::*;

    fn screen(cols: u16, text: &str) -> vt100::Parser {
        let cols = NonZeroU16::new(cols).unwrap();
        let mut parser = vt100::Parser::new(NonZeroU16::new(6).unwrap(), cols, 0);
        parser.process(text.as_bytes());
        parser
    }

    const ETH: &str = "0xfB6916095ca1df60bB79Ce92cE3Ea74c37c5d359";

    #[test]
    fn finds_an_address_in_a_coloured_report() {
        let parser = screen(
            80,
            &format!("  address  \x1b[38;2;130;200;255m{ETH}\x1b[0m\r\n"),
        );
        let hints = scan(parser.screen());
        assert_eq!(hints.len(), 1);
        assert_eq!(hints[0].text, ETH);
        assert_eq!(hints[0].cells[0], (0, 11));
    }

    #[test]
    fn newest_gets_the_first_label() {
        let parser = screen(
            80,
            "txid aaaaaaaaaaaaaaaaaaaaaaaaaaaa1\r\ntxid bbbbbbbbbbbbbbbbbbbbbbbbbbbb2\r\n",
        );
        let hints = scan(parser.screen());
        assert_eq!(hints[0].label, 'a');
        assert!(hints[0].text.ends_with('2'));
        assert_eq!(hints[1].label, 'b');
    }

    #[test]
    fn a_wrapped_address_is_copied_whole() {
        let parser = screen(20, &format!("{ETH}\r\n"));
        let hints = scan(parser.screen());
        assert_eq!(hints.len(), 1);
        assert_eq!(hints[0].text, ETH);
        assert_eq!(hints[0].cells.last(), Some(&(2, 1)));
    }

    #[test]
    fn prose_rules_and_abbreviated_hashes_are_not_offered() {
        let parser = screen(
            80,
            "abcdefghijklmnopqrstuvwxyz words\r\n----------------------\r\n0x12345678…abcdef\r\n",
        );
        assert!(scan(parser.screen()).is_empty());
    }

    #[test]
    fn a_url_is_offered_even_when_short() {
        let parser = screen(80, "see https://a.io.\r\n");
        assert_eq!(scan(parser.screen())[0].text, "https://a.io");
    }

    #[test]
    fn a_double_click_takes_the_word_it_lands_on() {
        let parser = screen(80, &format!("to {ETH} now"));
        let word = word_at(parser.screen(), 0, 10).unwrap();
        assert_eq!(word.text, ETH);
        assert!(word_at(parser.screen(), 0, 2).is_none());
    }

    #[test]
    fn labels_run_out_rather_than_repeat() {
        let lines: String = (0..30)
            .map(|index| format!("aaaaaaaaaaaaaaaaaaaaaaaaaaaaa{index}\r\n"))
            .collect();
        let mut parser = vt100::Parser::new(
            NonZeroU16::new(40).unwrap(),
            NonZeroU16::new(80).unwrap(),
            0,
        );
        parser.process(lines.as_bytes());
        assert_eq!(scan(parser.screen()).len(), 26);
    }
}
