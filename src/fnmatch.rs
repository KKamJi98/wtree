//! Shell-style pattern matching with Python `fnmatch` semantics.
//!
//! `wt rm --match` patterns used Python's `fnmatch.fnmatch`. The `glob` crate
//! agrees on most patterns but rejects an unclosed `[` that fnmatch treats as a
//! literal, so the translation is reproduced here:
//! - `*` matches any run of characters, `/` included; `?` matches one character.
//! - `[seq]` / `[!seq]` are character sets; a `]` right after `[` or `[!` is a
//!   member, `a-z` is a range, and an inverted range (`z-a`) matches nothing.
//! - a `[` with no closing `]` is a literal `[`.
//! - matching is case-sensitive, except on Windows where `os.path.normcase`
//!   lowercases both sides and turns `/` into `\`.

#[derive(Debug)]
enum Token {
    Literal(char),
    Any,
    Star,
    Set {
        negated: bool,
        items: Vec<(char, char)>,
    },
}

fn parse_set(stuff: &[char]) -> Vec<(char, char)> {
    let mut items = Vec::new();
    let mut i = 0;
    while i < stuff.len() {
        if i + 2 < stuff.len() && stuff[i + 1] == '-' {
            items.push((stuff[i], stuff[i + 2]));
            i += 3;
        } else {
            items.push((stuff[i], stuff[i]));
            i += 1;
        }
    }
    items
}

fn tokenize(pat: &[char]) -> Vec<Token> {
    let mut tokens = Vec::new();
    let n = pat.len();
    let mut i = 0;
    while i < n {
        let c = pat[i];
        i += 1;
        match c {
            '*' => {
                if !matches!(tokens.last(), Some(Token::Star)) {
                    tokens.push(Token::Star);
                }
            }
            '?' => tokens.push(Token::Any),
            '[' => {
                let mut j = i;
                if j < n && pat[j] == '!' {
                    j += 1;
                }
                if j < n && pat[j] == ']' {
                    j += 1;
                }
                while j < n && pat[j] != ']' {
                    j += 1;
                }
                if j >= n {
                    tokens.push(Token::Literal('['));
                    continue;
                }
                let mut stuff = &pat[i..j];
                i = j + 1;
                let negated = stuff.first() == Some(&'!');
                if negated {
                    stuff = &stuff[1..];
                }
                tokens.push(Token::Set {
                    negated,
                    items: parse_set(stuff),
                });
            }
            other => tokens.push(Token::Literal(other)),
        }
    }
    tokens
}

fn match_from(tokens: &[Token], text: &[char]) -> bool {
    let Some((first, rest)) = tokens.split_first() else {
        return text.is_empty();
    };
    match first {
        Token::Star => (0..=text.len()).any(|k| match_from(rest, &text[k..])),
        _ => {
            let Some((&ch, text_rest)) = text.split_first() else {
                return false;
            };
            let hit = match first {
                Token::Literal(l) => *l == ch,
                Token::Any => true,
                Token::Set { negated, items } => {
                    items.iter().any(|&(lo, hi)| lo <= ch && ch <= hi) != *negated
                }
                Token::Star => unreachable!(),
            };
            hit && match_from(rest, text_rest)
        }
    }
}

fn normcase(s: &str) -> String {
    if cfg!(windows) {
        s.to_lowercase().replace('/', "\\")
    } else {
        s.to_string()
    }
}

/// Python `fnmatch.fnmatch(name, pattern)`.
pub fn fnmatch(name: &str, pattern: &str) -> bool {
    let name: Vec<char> = normcase(name).chars().collect();
    let pattern: Vec<char> = normcase(pattern).chars().collect();
    match_from(&tokenize(&pattern), &name)
}

#[cfg(all(test, not(windows)))]
mod tests {
    use super::fnmatch;

    // Expected values come from CPython 3.15 `fnmatch.fnmatch` on macOS.
    const NAMES: [&str; 9] = [
        "feat/x",
        "feat-login",
        "main",
        "Main",
        "hotfix",
        "[",
        "a[",
        "]",
        "*",
    ];

    fn hits(pattern: &str) -> Vec<&'static str> {
        NAMES
            .iter()
            .copied()
            .filter(|n| fnmatch(n, pattern))
            .collect()
    }

    #[test]
    fn matches_cpython_corpus() {
        assert_eq!(hits("feat*"), ["feat/x", "feat-login"]);
        assert_eq!(hits("*/x"), ["feat/x"]);
        assert_eq!(
            hits("[!m]*"),
            [
                "feat/x",
                "feat-login",
                "Main",
                "hotfix",
                "[",
                "a[",
                "]",
                "*"
            ]
        );
        assert!(hits("MAIN").is_empty());
        assert_eq!(hits("f?at-*"), ["feat-login"]);
        assert_eq!(hits("[a-f]*"), ["feat/x", "feat-login", "a["]);
        assert_eq!(hits("["), ["["]);
        assert_eq!(hits("a["), ["a["]);
        assert_eq!(hits("**"), NAMES);
        assert!(hits("\\*").is_empty());
        assert_eq!(hits("[]]"), ["]"]);
        assert_eq!(hits("[!]]"), ["[", "*"]);
    }

    #[test]
    fn inverted_range_matches_nothing() {
        assert!(!fnmatch("m", "[z-a]"));
        assert!(fnmatch("m", "[!z-a]"));
    }
}
