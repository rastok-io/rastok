//! The ONE lexical fact of a comment — recognized here, policed by
//! each consumer. The primitive answers "is there a comment at this
//! offset, and where does it end"; it decides NOTHING about
//! expression termination, trivia consumption, or diagnostics —
//! those are the consumers' policies (host trivia consumes, the CEL
//! region scan applies its depth rule, probes stay read-only)
//! (D-2026-08-13-1, review round: divergent hand-rolled copies of
//! this fact were the root of a real bug).

/// One recognized comment at a byte offset.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Comment {
    /// One past the comment's final byte. A `//` comment ends AT its
    /// newline (the newline belongs to the source); an unterminated
    /// `/* …` ends at the source's end.
    pub end: usize,
    /// Whether a `/* … */` found its closer (`//` always is).
    pub terminated: bool,
    /// Whether the comment's bytes contain a line break.
    pub multiline: bool,
}

/// Recognizes a comment starting EXACTLY at `pos`, or `None`.
#[must_use]
pub fn comment_at(source: &str, pos: usize) -> Option<Comment> {
    let bytes = source.as_bytes();
    match (bytes.get(pos), bytes.get(pos + 1)) {
        (Some(b'/'), Some(b'/')) => {
            let end = source[pos..].find('\n').map_or(source.len(), |at| pos + at);
            Some(Comment {
                end,
                terminated: true,
                multiline: false,
            })
        }
        (Some(b'/'), Some(b'*')) => match source[pos + 2..].find("*/") {
            Some(at) => {
                let end = pos + 2 + at + 2;
                Some(Comment {
                    end,
                    terminated: true,
                    multiline: source[pos..end].contains('\n'),
                })
            }
            None => Some(Comment {
                end: source.len(),
                terminated: false,
                multiline: source[pos..].contains('\n'),
            }),
        },
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_primitive_states_the_fact_alone() {
        // Not a comment.
        assert_eq!(comment_at("a / b", 2), None);
        assert_eq!(comment_at("", 0), None);
        // Line comment: ends AT the newline, never multiline.
        assert_eq!(
            comment_at("x // note\ny", 2),
            Some(Comment {
                end: 9,
                terminated: true,
                multiline: false
            })
        );
        // At the source's end, no newline.
        assert_eq!(
            comment_at("// tail", 0),
            Some(Comment {
                end: 7,
                terminated: true,
                multiline: false
            })
        );
        // Block comment, same line.
        assert_eq!(
            comment_at("/* c */ x", 0),
            Some(Comment {
                end: 7,
                terminated: true,
                multiline: false
            })
        );
        // Block comment crossing a line.
        assert_eq!(
            comment_at("/* a\nb */", 0),
            Some(Comment {
                end: 9,
                terminated: true,
                multiline: true
            })
        );
        // Unterminated: ends at the source's end, says so.
        assert_eq!(
            comment_at("/* never", 0),
            Some(Comment {
                end: 8,
                terminated: false,
                multiline: false
            })
        );
        // An empty block comment and the tight star cases.
        assert_eq!(
            comment_at("/**/", 0),
            Some(Comment {
                end: 4,
                terminated: true,
                multiline: false
            })
        );
        assert_eq!(
            comment_at("/*/", 0),
            Some(Comment {
                end: 3,
                terminated: false,
                multiline: false
            })
        );
    }
}
