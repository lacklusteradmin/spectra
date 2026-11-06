//! Reading a prompt line, and what can follow it. Candidates come from the
//! command definitions themselves, so a new command, flag or value is offered
//! the moment it exists, with no list here to keep in step.

use clap::{Arg, Command};

struct Word {
    text: String,
    start: usize,
}

struct Lexed {
    words: Vec<Word>,
    open_quote: bool,
}

/// A line split into words as a shell would: quotes and backslashes, nothing
/// else. No pipes, globs or variables; the command never sees a shell.
fn lex(line: &str) -> Lexed {
    let mut words = Vec::new();
    let mut current: Option<Word> = None;
    let mut quote = None;
    let mut chars = line.char_indices().peekable();
    while let Some((at, c)) = chars.next() {
        let literal = match (quote, c) {
            (Some(open), c) if c == open => {
                quote = None;
                None
            }
            (Some('"'), '\\') => match chars.peek() {
                Some((_, next @ ('"' | '\\'))) => {
                    let next = *next;
                    chars.next();
                    Some(next)
                }
                _ => Some('\\'),
            },
            (Some(_), c) => Some(c),
            (None, '\'' | '"') => {
                quote = Some(c);
                current.get_or_insert_with(|| Word {
                    text: String::new(),
                    start: at,
                });
                None
            }
            (None, '\\') => chars.next().map(|(_, next)| next),
            (None, c) if c.is_whitespace() => {
                words.extend(current.take());
                None
            }
            (None, c) => Some(c),
        };
        if let Some(c) = literal {
            current
                .get_or_insert_with(|| Word {
                    text: String::new(),
                    start: at,
                })
                .text
                .push(c);
        }
    }
    words.extend(current);
    Lexed {
        words,
        open_quote: quote.is_some(),
    }
}

/// The arguments a typed line stands for. A leading `spectra` is dropped, so
/// a command copied from the docs or an error message runs as written.
pub fn arguments(line: &str) -> Result<Vec<String>, String> {
    let lexed = lex(line);
    if lexed.open_quote {
        return Err("a quote is not closed".to_string());
    }
    let home = dirs::home_dir();
    let mut arguments: Vec<String> = lexed
        .words
        .into_iter()
        .map(|word| match &home {
            // Only as typed: a quoted `~` stays a tilde, as in a shell.
            Some(home) if line[word.start..].starts_with("~/") => {
                format!("{}/{}", home.display(), &word.text[2..])
            }
            _ => word.text,
        })
        .collect();
    if arguments.first().is_some_and(|first| first == "spectra") {
        arguments.remove(0);
    }
    Ok(arguments)
}

/// Something the prompt can complete to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item {
    pub text: String,
    /// `text` with what it takes: `--chain <CHAIN>`.
    pub label: String,
    pub help: String,
    pub required: bool,
}

/// What can follow a line, up to the cursor.
#[derive(Debug, Default)]
pub struct Suggestions {
    /// Where the word being typed begins: an accepted item replaces from here.
    pub start: usize,
    /// How much of the word has been typed.
    pub typed: usize,
    pub items: Vec<Item>,
    /// What the next word means when it cannot be listed: `<WALLET> …`.
    pub hint: Option<String>,
}

impl Suggestions {
    /// What the prompt shows dimmed after the cursor: the rest of the only
    /// reading, or what is needed next.
    pub fn ghost(&self) -> Option<String> {
        if self.typed > 0 {
            return self
                .items
                .first()
                .map(|item| item.text[self.typed..].into());
        }
        if let Some(hint) = &self.hint {
            return hint.split("  ").next().map(str::to_string);
        }
        self.items
            .iter()
            .find(|item| item.required)
            .map(|item| item.label.clone())
    }
}

pub fn suggest(root: &Command, before_cursor: &str) -> Suggestions {
    let lexed = lex(before_cursor);
    let open_word = lexed.open_quote
        || before_cursor
            .chars()
            .next_back()
            .is_some_and(|c| !c.is_whitespace());
    let (finished, partial, start) = match lexed.words.split_last() {
        Some((last, rest)) if open_word => (rest, last.text.as_str(), last.start),
        _ => (&lexed.words[..], "", before_cursor.len()),
    };

    let mut command = root;
    let mut positionals = 0;
    let mut used: Vec<&str> = Vec::new();
    let mut awaiting: Option<&Arg> = None;
    for (index, word) in finished.iter().enumerate() {
        let word = word.text.as_str();
        if awaiting.take().is_some() || (index == 0 && word == "spectra") {
            continue;
        }
        if let Some(arg) = named_arg(command, word) {
            used.push(arg.get_id().as_str());
            if arg.get_action().takes_values() && !word.contains('=') {
                awaiting = Some(arg);
            }
        } else if let Some(subcommand) = command.find_subcommand(word) {
            command = subcommand;
            positionals = 0;
            used.clear();
        } else if !word.starts_with('-') {
            positionals += 1;
        }
    }

    let mut suggestions = Suggestions {
        start,
        typed: partial.len(),
        ..Suggestions::default()
    };
    let value_of = awaiting.or_else(|| {
        let positional = command.get_positionals().nth(positionals);
        positional.filter(|_| !partial.starts_with('-') && !has_subcommands(command))
    });
    if let Some(arg) = value_of {
        suggestions.items = values(arg, partial);
        if suggestions.items.is_empty() && partial.is_empty() {
            suggestions.hint = Some(value_hint(arg));
        }
        if awaiting.is_some() {
            return suggestions;
        }
    }
    if partial.starts_with('-') || (partial.is_empty() && !has_subcommands(command)) {
        suggestions.items.extend(flags(command, &used, partial));
    } else if has_subcommands(command) {
        suggestions.items = subcommands(command, partial);
    }
    suggestions
}

fn has_subcommands(command: &Command) -> bool {
    command.get_subcommands().any(|sub| !hidden_command(sub))
}

fn hidden_command(command: &Command) -> bool {
    command.is_hide_set() || command.get_name() == "help"
}

/// The flag a finished word names, in `--flag`, `--flag=value` or `-f`.
fn named_arg<'a>(command: &'a Command, word: &str) -> Option<&'a Arg> {
    let mut arguments = command.get_arguments();
    if let Some(long) = word.strip_prefix("--") {
        let name = long.split('=').next().unwrap_or(long);
        return arguments.find(|arg| arg.get_long() == Some(name));
    }
    let mut short = word.strip_prefix('-')?.chars();
    let (first, None) = (short.next()?, short.next()) else {
        return None;
    };
    arguments.find(|arg| arg.get_short() == Some(first))
}

fn subcommands(command: &Command, partial: &str) -> Vec<Item> {
    command
        .get_subcommands()
        .filter(|sub| !hidden_command(sub) && sub.get_name().starts_with(partial))
        .map(|sub| Item {
            text: sub.get_name().to_string(),
            label: sub.get_name().to_string(),
            help: sub.get_about().map(first_line).unwrap_or_default(),
            required: false,
        })
        .collect()
}

/// Flags not given yet, the required ones first and the global ones last.
fn flags(command: &Command, used: &[&str], partial: &str) -> Vec<Item> {
    let mut flags: Vec<&Arg> = command
        .get_arguments()
        .filter(|arg| {
            let repeats = matches!(arg.get_action(), clap::ArgAction::Append);
            !arg.is_hide_set() && (repeats || !used.contains(&arg.get_id().as_str()))
        })
        .filter(|arg| arg.get_long().is_some())
        .filter(|arg| format!("--{}", arg.get_long().unwrap_or_default()).starts_with(partial))
        .collect();
    flags.sort_by_key(|arg| {
        let last = arg.is_global_set() || matches!(arg.get_id().as_str(), "help" | "version");
        (!arg.is_required_set(), last)
    });
    flags
        .into_iter()
        .map(|arg| {
            let text = format!("--{}", arg.get_long().unwrap_or_default());
            let takes = arg.get_action().takes_values();
            let label = if takes {
                format!("{text} <{}>", value_name(arg))
            } else {
                text.clone()
            };
            Item {
                text,
                label,
                help: help_of(arg),
                required: arg.is_required_set(),
            }
        })
        .collect()
}

fn values(arg: &Arg, partial: &str) -> Vec<Item> {
    arg.get_possible_values()
        .into_iter()
        .filter(|value| !value.is_hide_set() && value.get_name().starts_with(partial))
        .map(|value| Item {
            text: value.get_name().to_string(),
            label: value.get_name().to_string(),
            help: value.get_help().map(first_line).unwrap_or_default(),
            required: false,
        })
        .collect()
}

fn value_hint(arg: &Arg) -> String {
    format!("<{}>  {}", value_name(arg), help_of(arg))
}

fn value_name(arg: &Arg) -> String {
    arg.get_value_names()
        .and_then(|names| names.first())
        .map_or_else(
            || arg.get_id().as_str().to_uppercase(),
            |name| name.to_string(),
        )
}

fn help_of(arg: &Arg) -> String {
    arg.get_help().map(first_line).unwrap_or_default()
}

fn first_line(help: &clap::builder::StyledStr) -> String {
    help.to_string()
        .lines()
        .next()
        .unwrap_or_default()
        .to_string()
}

#[cfg(test)]
mod tests {
    use clap::CommandFactory;

    use super::*;

    fn commands() -> Command {
        let mut command = crate::Cli::command();
        command.build();
        command
    }

    fn texts(line: &str) -> Vec<String> {
        suggest(&commands(), line)
            .items
            .into_iter()
            .map(|item| item.text)
            .collect()
    }

    #[test]
    fn words_split_as_a_shell_splits_them() {
        assert_eq!(
            arguments(r#"wallet rename "my wallet" it\'s 'a "b"' "c\"d""#).unwrap(),
            [
                "wallet",
                "rename",
                "my wallet",
                "it's",
                r#"a "b""#,
                r#"c"d"#
            ]
        );
    }

    #[test]
    fn an_empty_quoted_word_is_still_a_word() {
        assert_eq!(arguments(r#"a "" b"#).unwrap(), ["a", "", "b"]);
    }

    #[test]
    fn an_open_quote_is_refused_rather_than_guessed_at() {
        assert!(arguments(r#"wallet show "main"#).is_err());
    }

    #[test]
    fn a_copied_command_runs_as_written() {
        assert_eq!(
            arguments("spectra settings discard --yes").unwrap(),
            ["settings", "discard", "--yes"]
        );
    }

    #[test]
    fn only_a_typed_tilde_means_home() {
        let home = dirs::home_dir().unwrap();
        assert_eq!(
            arguments(r#"--file ~/key "~/kept""#).unwrap(),
            [
                "--file".to_string(),
                format!("{}/key", home.display()),
                "~/kept".to_string()
            ]
        );
    }

    #[test]
    fn an_empty_line_offers_every_command() {
        let offered = texts("");
        for command in commands().get_subcommands() {
            assert!(offered.iter().any(|text| text == command.get_name()));
        }
    }

    #[test]
    fn a_partial_word_narrows_to_what_it_starts() {
        assert_eq!(texts("st"), ["staking"]);
        let staged = suggest(&commands(), "st");
        assert_eq!(staged.start, 0);
        assert_eq!(staged.ghost().as_deref(), Some("aking"));
    }

    #[test]
    fn a_finished_noun_offers_its_subcommands() {
        let offered = texts("wallet ");
        assert!(offered.iter().any(|text| text == "list"));
        assert!(offered.iter().any(|text| text == "new"));
        assert!(!offered.iter().any(|text| text == "help"));
    }

    #[test]
    fn a_leading_spectra_does_not_change_what_follows() {
        assert_eq!(texts("spectra wallet "), texts("wallet "));
    }

    #[test]
    fn a_leaf_offers_required_flags_first_and_global_flags_last() {
        let offered = suggest(&commands(), "wallet new ");
        assert_eq!(offered.items[0].text, "--chain");
        assert!(offered.items[0].required);
        assert_eq!(offered.ghost().as_deref(), Some("--chain <CHAIN>"));
        let last = offered.items.len() - 1;
        assert!(
            offered.items[last - 2..]
                .iter()
                .any(|item| item.text == "--json")
        );
    }

    #[test]
    fn a_flag_already_given_is_not_offered_again() {
        let offered = texts("wallet new --chain Bitcoin --");
        assert!(!offered.iter().any(|text| text == "--chain"));
        assert!(offered.iter().any(|text| text == "--name"));
    }

    #[test]
    fn a_flag_awaiting_its_value_says_what_it_wants() {
        let offered = suggest(&commands(), "wallet new --chain ");
        assert!(offered.items.is_empty());
        assert!(offered.hint.unwrap().starts_with("<CHAIN>"));
    }

    #[test]
    fn the_next_positional_is_named() {
        let offered = suggest(&commands(), "wallet show ");
        assert!(offered.hint.unwrap().starts_with("<WALLET>"));
        assert_eq!(
            suggest(&commands(), "wallet show ").ghost().as_deref(),
            Some("<WALLET>")
        );
    }

    #[test]
    fn a_positional_already_given_is_not_asked_for_again() {
        assert!(suggest(&commands(), "wallet show main ").hint.is_none());
    }

    #[test]
    fn an_unknown_word_offers_nothing_rather_than_failing() {
        assert!(texts("nonsense").is_empty());
    }

    #[test]
    fn every_command_in_the_tree_is_reachable() {
        fn walk(command: &Command, line: &str) {
            let root = {
                let mut root = crate::Cli::command();
                root.build();
                root
            };
            for sub in command.get_subcommands().filter(|sub| !hidden_command(sub)) {
                let offered = suggest(&root, line);
                assert!(
                    offered.items.iter().any(|item| item.text == sub.get_name()),
                    "{line:?} does not offer {}",
                    sub.get_name()
                );
                walk(sub, &format!("{line}{} ", sub.get_name()));
            }
        }
        walk(&commands(), "");
    }
}
