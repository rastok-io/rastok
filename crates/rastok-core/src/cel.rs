//! Isolated CEL compilation and evaluation domain.
//!
//! This module depends on nothing else in Rastok: the compiler hands
//! it expression text and a [`Signature`], the runtime adapts state
//! and envelopes onto [`Bindings`]. Diagnostics and errors are native
//! types; callers adapt them.
//!
//! The admitted subset covers literals, paths, member and index
//! access, arithmetic, comparisons, logic with short-circuit, the
//! ternary, list and map literals, and the `duration`, `timestamp`,
//! and `size` functions. Macros, optional types, and `as` narrowing
//! are recognized and refused explicitly. There is no `now()` and no
//! ambient clock: time enters only as data or constants.
//!
//! Map iteration and map value ordering are sorted-key — an evaluator
//! guarantee that golden-trace stability relies on.

use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;
use alloc::{format, vec};
use core::fmt;
use core::ops::Range;

/// Names refused explicitly: admitted by full CEL, outside this build.
const UNSUPPORTED_FUNCTIONS: &[&str] = &["has", "exists", "exists_one", "all", "map", "filter"];

/// The CEL type domain of this subset. `Dyn` is gradual: operations
/// on it defer their type discipline to evaluation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Type {
    /// Boolean.
    Bool,
    /// 64-bit signed integer.
    Int,
    /// 64-bit unsigned integer (CEL's `uint`).
    Uint,
    /// IEEE-754 double.
    Double,
    /// UTF-8 text.
    Text,
    /// Opaque bytes.
    Bytes,
    /// Protobuf-style duration, nanosecond resolution.
    Duration,
    /// UTC instant, nanosecond resolution.
    Timestamp,
    /// The null type.
    Null,
    /// Gradually-typed value; operations check at evaluation.
    Dyn,
    /// Homogeneous list.
    List(Box<Type>),
    /// Text-keyed map with sorted-key iteration.
    Map(Box<Type>),
}

/// A CEL runtime value, mirroring [`Type`].
///
/// Equality is STRUCTURAL and agrees with the canonical bytes: the
/// `Double` arm compares canonicalized bits (all NaNs are one value,
/// `-0.0` and `+0.0` are two), so a plan equals itself across the
/// encode/decode trip and two plans with different canonical bytes
/// never compare equal. CEL's own `==` operator does NOT use this —
/// it lives in `values_equal` and keeps IEEE numeric semantics.
#[derive(Debug, Clone)]
pub enum Value {
    /// Boolean.
    Bool(bool),
    /// 64-bit signed integer.
    Int(i64),
    /// 64-bit unsigned integer.
    Uint(u64),
    /// IEEE-754 double.
    Double(f64),
    /// UTF-8 text.
    Text(String),
    /// Opaque bytes.
    Bytes(Vec<u8>),
    /// Duration in nanoseconds.
    Duration(i64),
    /// UTC instant in nanoseconds since the Unix epoch.
    Timestamp(i64),
    /// The null value.
    Null,
    /// Homogeneous list.
    List(Vec<Value>),
    /// Text-keyed map; `BTreeMap` keeps iteration sorted by key.
    Map(BTreeMap<String, Value>),
    /// Named fields in declaration order — the LOGICAL twin of
    /// `envelope::Value::Struct` (canonical tag 17, where a map is
    /// 18): the witness that lets a struct cross the CEL bridge —
    /// bindings, locals, ternaries — without silently becoming a
    /// map. CEL cannot construct one; it can only carry, access,
    /// and re-emit what the envelope domain handed it.
    Struct(Vec<(String, Value)>),
    /// An exact decimal — `unscaled × 10^(−scale)` — the dynamic
    /// numeric beyond the integer domain (D-2026-08-10-7): at most
    /// 38 significant digits (Arrow decimal128), MINIMAL scale for
    /// dynamic values (trailing zeros stripped, so an
    /// int-representable value never hides here). No arithmetic —
    /// `variant` is narrowed before use.
    Decimal {
        /// The unscaled coefficient.
        unscaled: i128,
        /// Digits after the point (negative = trailing zeros).
        scale: i32,
    },
    /// The WHOLE `data` payload, unmaterialized — the laziness
    /// witness. Resolving the name `data` is not yet inspecting its
    /// structure: the witness carries through bindings, locals, and
    /// ternaries untouched, and materializes only where an operation
    /// demands structure (member, index, operator, call) — through
    /// [`Bindings::payload`]. It lives inside one evaluation and its
    /// host's emission builder; it never reaches a plan, a hash, or
    /// a seal (the expressible-constant gate refuses it).
    Payload,
}

impl PartialEq for Value {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Bool(a), Self::Bool(b)) => a == b,
            (Self::Int(a), Self::Int(b)) => a == b,
            (Self::Uint(a), Self::Uint(b)) => a == b,
            // Canonicalized BITS, not IEEE: structural equality must
            // agree with the canonical bytes, or a plan carrying NaN
            // stops equalling itself across the trip while the two
            // zeros collapse despite different bytes.
            (Self::Double(a), Self::Double(b)) => {
                crate::canonical_cbor::CanonicalDouble::new(*a)
                    == crate::canonical_cbor::CanonicalDouble::new(*b)
            }
            (Self::Text(a), Self::Text(b)) => a == b,
            (Self::Bytes(a), Self::Bytes(b)) => a == b,
            (Self::Duration(a), Self::Duration(b)) => a == b,
            (Self::Timestamp(a), Self::Timestamp(b)) => a == b,
            (Self::Null, Self::Null) => true,
            (Self::List(a), Self::List(b)) => a == b,
            (Self::Map(a), Self::Map(b)) => a == b,
            // Fields compare in SORTED order — by name, like the
            // canonical seal, so order is presentation. Sorting
            // (stable) instead of name-lookup keeps `Eq`'s contract
            // total: a malformed duplicate-name struct — refused at
            // every door, but representable — still equals itself.
            (Self::Struct(a), Self::Struct(b)) => struct_fields_eq(a, b, Value::eq),
            (
                Self::Decimal { unscaled, scale },
                Self::Decimal {
                    unscaled: other_unscaled,
                    scale: other_scale,
                },
            ) => unscaled == other_unscaled && scale == other_scale,
            // One payload exists per evaluation; two witnesses can
            // only name the same thing.
            (Self::Payload, Self::Payload) => true,
            _ => false,
        }
    }
}

impl Eq for Value {}

/// Sorted-field struct comparison, shared by structural equality and
/// the CEL `==` operator: stable-sort both sides by name, compare
/// positionally. For well-formed structs (unique names — the door
/// invariant) this IS by-name equality and agrees with the canonical
/// bytes; for malformed values it stays reflexive, symmetric, and
/// transitive, which `Eq` promises unconditionally.
fn struct_fields_eq(
    a: &[(String, Value)],
    b: &[(String, Value)],
    values: impl Fn(&Value, &Value) -> bool,
) -> bool {
    if a.len() != b.len() {
        return false;
    }
    fn sorted(fields: &[(String, Value)]) -> alloc::vec::Vec<&(String, Value)> {
        let mut refs: alloc::vec::Vec<&(String, Value)> = fields.iter().collect();
        refs.sort_by(|x, y| x.0.cmp(&y.0));
        refs
    }
    sorted(a)
        .into_iter()
        .zip(sorted(b))
        .all(|(x, y)| x.0 == y.0 && values(&x.1, &y.1))
}

/// Names and types visible to an expression. Keys may be dotted
/// (`data.text`) for pre-resolved leaves, or bare roots. The
/// signature never offers `now`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Signature {
    names: BTreeMap<String, Type>,
}

impl Signature {
    /// Starts an empty signature.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Declares a visible name, dotted or bare.
    #[must_use]
    pub fn with(mut self, name: impl Into<String>, ty: Type) -> Self {
        self.names.insert(name.into(), ty);
        self
    }

    /// The type of an exact name, if declared.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&Type> {
        self.names.get(name)
    }
}

/// Bridge exposing Rastok state to CEL. Lookup mirrors compilation:
/// the full dotted path is offered first, then the bare root.
pub trait Bindings {
    /// The value bound to `name`, if any.
    fn get(&self, name: &str) -> Option<Value>;

    /// The whole payload behind a [`Value::Payload`] witness,
    /// materialized. The default has no payload authority: a witness
    /// reaching an environment that cannot honor it is a fault, not
    /// a silent value.
    ///
    /// # Errors
    /// When this environment carries no payload.
    fn payload(&self) -> Result<Value, Error> {
        Err(fault(String::from(
            "`data` carries no payload in this environment",
        )))
    }

    /// The bytes behind an [`Expr::Asset`] reference. The default
    /// has no asset table.
    ///
    /// # Errors
    /// When this environment carries no assets.
    fn asset(&self, index: u32) -> Result<Value, Error> {
        Err(fault(format!("asset {index} is not available here")))
    }
}

impl Bindings for BTreeMap<String, Value> {
    fn get(&self, name: &str) -> Option<Value> {
        BTreeMap::get(self, name).cloned()
    }
}

/// What a compiler diagnostic is about, machine-readably. Messages
/// are never stable; kinds are — callers key on them, never on text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiagnosticKind {
    /// Any fault without a dedicated kind yet.
    General,
    /// A static map literal carrying the same key twice.
    DuplicateMapKey,
    /// An expression nesting beyond the format budget — refused at
    /// the gate, before any recursive pass rides the machine stack.
    /// Carries the measured depth: the STRUCTURED payload the
    /// registered diagnostic emits, never the human sentence.
    TooDeep {
        /// The nesting depth reached when the gate refused.
        depth: usize,
    },
}

/// Native CEL compiler diagnostic; the caller adapts it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    /// The stable machine-readable discriminant.
    pub kind: DiagnosticKind,
    /// What is wrong.
    pub message: String,
    /// Byte range inside the compiled expression text.
    pub range: Range<usize>,
}

/// CEL evaluation error — value errors only; static type errors are
/// rejected at compilation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error {
    /// What failed.
    pub message: String,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl core::error::Error for Error {}

/// Canonical typed CEL tree — one form for hashing, planning, and
/// evaluation.
#[derive(Debug, Clone, PartialEq)]
pub struct Tree {
    /// The expression.
    pub root: Expr,
    /// The expression's static type.
    pub ty: Type,
}

/// One expression node.
#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    /// A literal value.
    Literal(Value),
    /// A build-time asset by table index (D-2026-08-10-5): evaluates
    /// to the blob's bytes through [`Bindings::asset`]; the HASH
    /// seals the content digest, never the index or the path.
    Asset(u32),
    /// A dotted access path, resolved through bindings at evaluation.
    Path(Vec<String>),
    /// Unary operation.
    Unary {
        /// The operator.
        op: UnaryOp,
        /// The operand.
        expr: Box<Expr>,
    },
    /// Binary operation.
    Binary {
        /// The operator.
        op: BinaryOp,
        /// Left operand.
        left: Box<Expr>,
        /// Right operand.
        right: Box<Expr>,
    },
    /// `cond ? then : otherwise`
    Ternary {
        /// The condition.
        cond: Box<Expr>,
        /// Value when true.
        then: Box<Expr>,
        /// Value when false.
        otherwise: Box<Expr>,
    },
    /// Index access `base[index]`.
    Index {
        /// The indexed collection.
        base: Box<Expr>,
        /// The index or key.
        index: Box<Expr>,
    },
    /// Registered function call.
    Call {
        /// The function.
        function: Function,
        /// The arguments.
        args: Vec<Expr>,
    },
    /// List literal.
    List(Vec<Expr>),
    /// Map literal with text keys.
    MapLit(Vec<(String, Expr)>),
}

/// Unary operators.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnaryOp {
    /// Logical negation.
    Not,
    /// Arithmetic negation.
    Neg,
}

/// Binary operators.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinaryOp {
    /// `*`
    Mul,
    /// `/`
    Div,
    /// `%`
    Rem,
    /// `+`
    Add,
    /// `-`
    Sub,
    /// `==`
    Eq,
    /// `!=`
    Ne,
    /// `<`
    Lt,
    /// `<=`
    Le,
    /// `>`
    Gt,
    /// `>=`
    Ge,
    /// `&&`, short-circuit.
    And,
    /// `||`, short-circuit.
    Or,
}

/// The registered function vocabulary of the subset.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Function {
    /// `duration("2h30m")` — CEL/Go format; no `min`, `d`, or `w`.
    Duration,
    /// `timestamp("2026-01-05T00:00:00Z")` — RFC 3339, UTC only.
    Timestamp,
    /// `size(text | bytes | list | map)`
    Size,
    /// `decimal("1.50")` — CEL's exact-number constructor: a
    /// standard JSON number at its WRITTEN scale (D-2026-08-13-1);
    /// the integer fold applies to the written scale.
    Decimal,
}

/// Compiles expression text against a signature into a typed tree.
///
/// # Errors
///
/// Returns every diagnostic found; an erroneous expression never
/// produces a tree.
pub fn compile(text: &str, signature: &Signature) -> Result<Tree, Vec<Diagnostic>> {
    let tokens = lex(text)?;
    let mut parser = ExprParser {
        tokens: &tokens,
        pos: 0,
        text,
        depth: 0,
    };
    let root = parser.ternary().map_err(|diagnostic| vec![diagnostic])?;
    // The depth gate, FIRST — before the trailing-input check drops
    // the tree and before the type-checker, the hasher, or the
    // lowering recurse over it on the machine stack. The parser's own
    // recursion is guarded above, but a left-fold chain (`1 + 1 + …`)
    // builds depth with no parser recursion at all; it is caught
    // here, iteratively. Structural depth beyond the reader's budget
    // can never serialize anyway — every level costs at least one
    // CBOR level — so the refusal is the same named diagnostic, and
    // the refused tree is dismantled iteratively, never dropped
    // through a recursive `Drop`.
    let depth = structural_depth(&root);
    if depth > crate::canonical_cbor::MAX_NESTING {
        let range = 0..text.len();
        dismantle(root);
        return Err(vec![Diagnostic {
            kind: DiagnosticKind::TooDeep { depth },
            message: String::from("the expression nests deeper than the format budget"),
            range,
        }]);
    }
    if let Some(token) = parser.tokens.get(parser.pos) {
        if let Tok::Ident(word) = &token.tok
            && *word == "as"
        {
            return Err(vec![Diagnostic {
                kind: DiagnosticKind::General,
                message: String::from("`as` narrowing is outside this build's subset"),
                range: token.range.clone(),
            }]);
        }
        return Err(vec![Diagnostic {
            kind: DiagnosticKind::General,
            message: String::from("unexpected trailing input"),
            range: token.range.clone(),
        }]);
    }
    let ty = check(&root, signature, text).map_err(|diagnostic| vec![diagnostic])?;
    Ok(Tree { root, ty })
}

/// Evaluates a compiled tree against runtime bindings.
///
/// # Errors
///
/// Value errors only: unknown bindings, arithmetic faults, dynamic
/// type mismatches, malformed time strings.
pub fn eval(tree: &Tree, bindings: &dyn Bindings) -> Result<Value, Error> {
    eval_expr(&tree.root, bindings)
}

/// Evaluates a compiled tree against constants only — the boot-time
/// reduction used by initializer positions.
///
/// # Errors
///
/// Same as [`eval`]; a reference to a non-constant name is an error.
pub fn const_eval(tree: &Tree, consts: &BTreeMap<String, Value>) -> Result<Value, Error> {
    eval_expr(&tree.root, consts)
}

// ===== lexing =====

#[derive(Debug, Clone, PartialEq)]
enum Tok<'a> {
    Int(i64),
    Uint(u64),
    Double(f64),
    Str(String),
    Ident(&'a str),
    True,
    False,
    Null,
    LParen,
    RParen,
    LBrack,
    RBrack,
    LBrace,
    RBrace,
    Comma,
    Colon,
    Dot,
    Question,
    Bang,
    Minus,
    Plus,
    Star,
    Slash,
    Percent,
    EqEq,
    NotEq,
    Lt,
    Le,
    Gt,
    Ge,
    AndAnd,
    OrOr,
}

#[derive(Debug, Clone, PartialEq)]
struct Spanned<'a> {
    tok: Tok<'a>,
    range: Range<usize>,
}

fn lex(text: &str) -> Result<Vec<Spanned<'_>>, Vec<Diagnostic>> {
    let bytes = text.as_bytes();
    let mut tokens = Vec::new();
    let mut pos = 0;
    while pos < bytes.len() {
        let start = pos;
        let byte = bytes[pos];
        let tok = match byte {
            b' ' | b'\t' | b'\r' | b'\n' => {
                pos += 1;
                continue;
            }
            b'a'..=b'z' | b'A'..=b'Z' | b'_' => {
                while pos < bytes.len()
                    && (bytes[pos].is_ascii_alphanumeric() || bytes[pos] == b'_')
                {
                    pos += 1;
                }
                match &text[start..pos] {
                    "true" => Tok::True,
                    "false" => Tok::False,
                    "null" => Tok::Null,
                    "in" => {
                        return Err(vec![Diagnostic {
                            kind: DiagnosticKind::General,
                            message: String::from("`in` is outside this build's subset"),
                            range: start..pos,
                        }]);
                    }
                    word => Tok::Ident(word),
                }
            }
            b'0'..=b'9' => {
                while pos < bytes.len() && bytes[pos].is_ascii_digit() {
                    pos += 1;
                }
                if pos < bytes.len() && bytes[pos] == b'u' {
                    let value = text[start..pos].parse().map_err(|_| {
                        vec![Diagnostic {
                            kind: DiagnosticKind::General,
                            message: String::from("uint literal out of range"),
                            range: start..pos,
                        }]
                    })?;
                    pos += 1;
                    Tok::Uint(value)
                } else if pos + 1 < bytes.len()
                    && bytes[pos] == b'.'
                    && bytes[pos + 1].is_ascii_digit()
                {
                    pos += 1;
                    while pos < bytes.len() && bytes[pos].is_ascii_digit() {
                        pos += 1;
                    }
                    let value = text[start..pos].parse().map_err(|_| {
                        vec![Diagnostic {
                            kind: DiagnosticKind::General,
                            message: String::from("malformed double literal"),
                            range: start..pos,
                        }]
                    })?;
                    Tok::Double(value)
                } else {
                    let value = text[start..pos].parse().map_err(|_| {
                        vec![Diagnostic {
                            kind: DiagnosticKind::General,
                            message: String::from("int literal out of range"),
                            range: start..pos,
                        }]
                    })?;
                    Tok::Int(value)
                }
            }
            quote @ (b'"' | b'\'') => {
                pos += 1;
                let mut value = String::new();
                loop {
                    if pos >= bytes.len() {
                        return Err(vec![Diagnostic {
                            kind: DiagnosticKind::General,
                            message: String::from("unterminated string"),
                            range: start..pos,
                        }]);
                    }
                    let current = bytes[pos];
                    if current == quote {
                        pos += 1;
                        break;
                    }
                    if current == b'\\' {
                        pos += 1;
                        let escaped = *bytes.get(pos).ok_or_else(|| {
                            vec![Diagnostic {
                                kind: DiagnosticKind::General,
                                message: String::from("unterminated escape"),
                                range: start..pos,
                            }]
                        })?;
                        value.push(match escaped {
                            b'n' => '\n',
                            b't' => '\t',
                            b'r' => '\r',
                            b'\\' => '\\',
                            b'\'' => '\'',
                            b'"' => '"',
                            other => {
                                return Err(vec![Diagnostic {
                                    kind: DiagnosticKind::General,
                                    message: format!(
                                        "unsupported escape `\\{}`",
                                        char::from(other)
                                    ),
                                    range: pos - 1..pos + 1,
                                }]);
                            }
                        });
                        pos += 1;
                    } else {
                        let character = text[pos..].chars().next().unwrap_or('\u{fffd}');
                        value.push(character);
                        pos += character.len_utf8();
                    }
                }
                Tok::Str(value)
            }
            b'(' => one(&mut pos, Tok::LParen),
            b')' => one(&mut pos, Tok::RParen),
            b'[' => one(&mut pos, Tok::LBrack),
            b']' => one(&mut pos, Tok::RBrack),
            b'{' => one(&mut pos, Tok::LBrace),
            b'}' => one(&mut pos, Tok::RBrace),
            b',' => one(&mut pos, Tok::Comma),
            b':' => one(&mut pos, Tok::Colon),
            b'.' => {
                if bytes.get(pos + 1) == Some(&b'?') {
                    return Err(vec![Diagnostic {
                        kind: DiagnosticKind::General,
                        message: String::from(
                            "optional access `.?` is outside this build's subset",
                        ),
                        range: pos..pos + 2,
                    }]);
                }
                one(&mut pos, Tok::Dot)
            }
            b'?' => one(&mut pos, Tok::Question),
            b'!' => {
                if bytes.get(pos + 1) == Some(&b'=') {
                    pos += 2;
                    Tok::NotEq
                } else {
                    one(&mut pos, Tok::Bang)
                }
            }
            b'-' => one(&mut pos, Tok::Minus),
            b'+' => one(&mut pos, Tok::Plus),
            b'*' => one(&mut pos, Tok::Star),
            b'/' => one(&mut pos, Tok::Slash),
            b'%' => one(&mut pos, Tok::Percent),
            b'=' => {
                if bytes.get(pos + 1) == Some(&b'=') {
                    pos += 2;
                    Tok::EqEq
                } else {
                    return Err(vec![Diagnostic {
                        kind: DiagnosticKind::General,
                        message: String::from("assignment is not an expression"),
                        range: pos..pos + 1,
                    }]);
                }
            }
            b'<' => {
                if bytes.get(pos + 1) == Some(&b'=') {
                    pos += 2;
                    Tok::Le
                } else {
                    one(&mut pos, Tok::Lt)
                }
            }
            b'>' => {
                if bytes.get(pos + 1) == Some(&b'=') {
                    pos += 2;
                    Tok::Ge
                } else {
                    one(&mut pos, Tok::Gt)
                }
            }
            b'&' => {
                if bytes.get(pos + 1) == Some(&b'&') {
                    pos += 2;
                    Tok::AndAnd
                } else {
                    return Err(vec![Diagnostic {
                        kind: DiagnosticKind::General,
                        message: String::from("`&` is not a CEL operator"),
                        range: pos..pos + 1,
                    }]);
                }
            }
            b'|' => {
                if bytes.get(pos + 1) == Some(&b'|') {
                    pos += 2;
                    Tok::OrOr
                } else {
                    return Err(vec![Diagnostic {
                        kind: DiagnosticKind::General,
                        message: String::from("`|` is not a CEL operator"),
                        range: pos..pos + 1,
                    }]);
                }
            }
            _ => {
                let character = text[pos..].chars().next().unwrap_or('\u{fffd}');
                return Err(vec![Diagnostic {
                    kind: DiagnosticKind::General,
                    message: format!("the character `{character}` is not part of CEL"),
                    range: pos..pos + character.len_utf8(),
                }]);
            }
        };
        tokens.push(Spanned {
            tok,
            range: start..pos,
        });
    }
    Ok(tokens)
}

fn one<'a>(pos: &mut usize, tok: Tok<'a>) -> Tok<'a> {
    *pos += 1;
    tok
}

// ===== parsing =====

struct ExprParser<'t, 'a> {
    tokens: &'t [Spanned<'a>],
    pos: usize,
    text: &'a str,
    /// Production-nesting depth — the recursion guard. Bounded at
    /// the format budget so a parenthesis tower or a unary chain
    /// refuses by name instead of overflowing the parser's stack;
    /// grouping does not create AST depth, so the bound is
    /// conservative for adversarial shapes, never for real programs.
    depth: usize,
}

impl<'a> ExprParser<'_, 'a> {
    fn peek(&self) -> Option<&Tok<'a>> {
        self.tokens.get(self.pos).map(|spanned| &spanned.tok)
    }

    fn bump(&mut self) -> Option<&Spanned<'a>> {
        let token = self.tokens.get(self.pos);
        if token.is_some() {
            self.pos += 1;
        }
        token
    }

    fn eat(&mut self, tok: &Tok<'a>) -> bool {
        if self.peek() == Some(tok) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn fail(&self, message: impl Into<String>) -> Diagnostic {
        let range = self
            .tokens
            .get(self.pos)
            .map_or(self.text.len()..self.text.len(), |token| {
                token.range.clone()
            });
        Diagnostic {
            kind: DiagnosticKind::General,
            message: message.into(),
            range,
        }
    }

    fn expect(&mut self, tok: &Tok<'a>, what: &str) -> Result<(), Diagnostic> {
        if self.eat(tok) {
            Ok(())
        } else {
            Err(self.fail(format!("expected {what}")))
        }
    }

    /// One step deeper into the productions; refuses past the budget.
    /// Each SYNTACTIC nesting level counts exactly once — `ternary`
    /// counts (every nesting construct re-enters it through
    /// `primary`), and `unary` counts only when an operator actually
    /// nests, so the visible budget is the real one.
    fn descend(&mut self) -> Result<(), Diagnostic> {
        self.depth += 1;
        if self.depth > crate::canonical_cbor::MAX_NESTING {
            return Err(Diagnostic {
                kind: DiagnosticKind::TooDeep { depth: self.depth },
                message: String::from("the expression nests deeper than the format budget"),
                range: self
                    .tokens
                    .get(self.pos)
                    .map_or(0..self.text.len(), |token| token.range.clone()),
            });
        }
        Ok(())
    }

    fn ternary(&mut self) -> Result<Expr, Diagnostic> {
        self.descend()?;
        let result = self.ternary_inner();
        self.depth -= 1;
        result
    }

    fn ternary_inner(&mut self) -> Result<Expr, Diagnostic> {
        let cond = self.or()?;
        if self.eat(&Tok::Question) {
            let then = self.or()?;
            self.expect(&Tok::Colon, "`:`")?;
            let otherwise = self.ternary()?;
            return Ok(Expr::Ternary {
                cond: Box::new(cond),
                then: Box::new(then),
                otherwise: Box::new(otherwise),
            });
        }
        Ok(cond)
    }

    fn or(&mut self) -> Result<Expr, Diagnostic> {
        let mut left = self.and()?;
        while self.eat(&Tok::OrOr) {
            let right = self.and()?;
            left = binary(BinaryOp::Or, left, right);
        }
        Ok(left)
    }

    fn and(&mut self) -> Result<Expr, Diagnostic> {
        let mut left = self.relation()?;
        while self.eat(&Tok::AndAnd) {
            let right = self.relation()?;
            left = binary(BinaryOp::And, left, right);
        }
        Ok(left)
    }

    fn relation(&mut self) -> Result<Expr, Diagnostic> {
        let mut left = self.addition()?;
        loop {
            let op = match self.peek() {
                Some(Tok::EqEq) => BinaryOp::Eq,
                Some(Tok::NotEq) => BinaryOp::Ne,
                Some(Tok::Lt) => BinaryOp::Lt,
                Some(Tok::Le) => BinaryOp::Le,
                Some(Tok::Gt) => BinaryOp::Gt,
                Some(Tok::Ge) => BinaryOp::Ge,
                _ => return Ok(left),
            };
            self.pos += 1;
            let right = self.addition()?;
            left = binary(op, left, right);
        }
    }

    fn addition(&mut self) -> Result<Expr, Diagnostic> {
        let mut left = self.multiplication()?;
        loop {
            let op = match self.peek() {
                Some(Tok::Plus) => BinaryOp::Add,
                Some(Tok::Minus) => BinaryOp::Sub,
                _ => return Ok(left),
            };
            self.pos += 1;
            let right = self.multiplication()?;
            left = binary(op, left, right);
        }
    }

    fn multiplication(&mut self) -> Result<Expr, Diagnostic> {
        let mut left = self.unary()?;
        loop {
            let op = match self.peek() {
                Some(Tok::Star) => BinaryOp::Mul,
                Some(Tok::Slash) => BinaryOp::Div,
                Some(Tok::Percent) => BinaryOp::Rem,
                _ => return Ok(left),
            };
            self.pos += 1;
            let right = self.unary()?;
            left = binary(op, left, right);
        }
    }

    fn unary(&mut self) -> Result<Expr, Diagnostic> {
        // A unary chain recurses without ever re-entering `ternary`,
        // so an actual operator counts one level; the plain
        // fall-through to `postfix` is not a nesting and counts
        // nothing — one unit per syntactic level, everywhere.
        if matches!(self.peek(), Some(Tok::Bang | Tok::Minus)) {
            self.descend()?;
            let result = self.unary_inner();
            self.depth -= 1;
            return result;
        }
        self.postfix()
    }

    fn unary_inner(&mut self) -> Result<Expr, Diagnostic> {
        if self.eat(&Tok::Bang) {
            return Ok(Expr::Unary {
                op: UnaryOp::Not,
                expr: Box::new(self.unary()?),
            });
        }
        if self.eat(&Tok::Minus) {
            return Ok(Expr::Unary {
                op: UnaryOp::Neg,
                expr: Box::new(self.unary()?),
            });
        }
        self.postfix()
    }

    fn postfix(&mut self) -> Result<Expr, Diagnostic> {
        let mut expr = self.primary()?;
        loop {
            if self.eat(&Tok::Dot) {
                let Some(Tok::Ident(member)) = self.peek().cloned() else {
                    return Err(self.fail("expected a member name after `.`"));
                };
                self.pos += 1;
                if self.peek() == Some(&Tok::LParen) {
                    return Err(self.fail(format!(
                        "method call `.{member}()` is outside this build's subset"
                    )));
                }
                match &mut expr {
                    Expr::Path(segments) => segments.push(String::from(member)),
                    _ => {
                        return Err(self.fail(
                            "member access on a computed value is outside this build's subset",
                        ));
                    }
                }
            } else if self.eat(&Tok::LBrack) {
                let index = self.ternary()?;
                self.expect(&Tok::RBrack, "`]`")?;
                expr = Expr::Index {
                    base: Box::new(expr),
                    index: Box::new(index),
                };
            } else {
                return Ok(expr);
            }
        }
    }

    fn primary(&mut self) -> Result<Expr, Diagnostic> {
        let Some(token) = self.bump().cloned() else {
            return Err(self.fail("expected an expression"));
        };
        match token.tok {
            Tok::Int(value) => Ok(Expr::Literal(Value::Int(value))),
            Tok::Uint(value) => Ok(Expr::Literal(Value::Uint(value))),
            Tok::Double(value) => Ok(Expr::Literal(Value::Double(value))),
            Tok::Str(value) => Ok(Expr::Literal(Value::Text(value))),
            Tok::True => Ok(Expr::Literal(Value::Bool(true))),
            Tok::False => Ok(Expr::Literal(Value::Bool(false))),
            Tok::Null => Ok(Expr::Literal(Value::Null)),
            Tok::LParen => {
                let expr = self.ternary()?;
                self.expect(&Tok::RParen, "`)`")?;
                Ok(expr)
            }
            Tok::LBrack => {
                let mut items = Vec::new();
                while self.peek() != Some(&Tok::RBrack) {
                    items.push(self.ternary()?);
                    if !self.eat(&Tok::Comma) {
                        break;
                    }
                }
                self.expect(&Tok::RBrack, "`]`")?;
                Ok(Expr::List(items))
            }
            Tok::LBrace => {
                let mut entries: Vec<(String, Expr)> = Vec::new();
                while self.peek() != Some(&Tok::RBrace) {
                    let (key, range) = match self.bump().cloned() {
                        Some(Spanned {
                            tok: Tok::Str(key),
                            range,
                        }) => (key, range),
                        _ => return Err(self.fail("map keys are string literals in this subset")),
                    };
                    if entries.iter().any(|(existing, _)| existing == &key) {
                        // Stable message prefix: callers key on
                        // "duplicate map key" to assign E-DUP-KEY.
                        return Err(Diagnostic {
                            kind: DiagnosticKind::DuplicateMapKey,
                            message: format!("duplicate map key `{key}`"),
                            range,
                        });
                    }
                    self.expect(&Tok::Colon, "`:`")?;
                    entries.push((key, self.ternary()?));
                    if !self.eat(&Tok::Comma) {
                        break;
                    }
                }
                self.expect(&Tok::RBrace, "`}`")?;
                // Maps are semantically unordered: ONE canonical form
                // serves hashing, planning, and evaluation alike.
                entries.sort_by(|a, b| a.0.cmp(&b.0));
                Ok(Expr::MapLit(entries))
            }
            Tok::Ident(word) => {
                if self.peek() == Some(&Tok::LParen) {
                    self.pos += 1;
                    let function = match word {
                        "duration" => Function::Duration,
                        "timestamp" => Function::Timestamp,
                        "size" => Function::Size,
                        "decimal" => Function::Decimal,
                        "now" => {
                            return Err(Diagnostic {
                                kind: DiagnosticKind::General,
                                message: String::from(
                                    "there is no `now()`: time enters only as data or constants",
                                ),
                                range: token.range,
                            });
                        }
                        other if UNSUPPORTED_FUNCTIONS.contains(&other) => {
                            return Err(Diagnostic {
                                kind: DiagnosticKind::General,
                                message: format!(
                                    "the `{other}` macro is outside this build's subset"
                                ),
                                range: token.range,
                            });
                        }
                        other => {
                            return Err(Diagnostic {
                                kind: DiagnosticKind::General,
                                message: format!("unknown function `{other}`"),
                                range: token.range,
                            });
                        }
                    };
                    let mut args = Vec::new();
                    while self.peek() != Some(&Tok::RParen) {
                        args.push(self.ternary()?);
                        if !self.eat(&Tok::Comma) {
                            break;
                        }
                    }
                    self.expect(&Tok::RParen, "`)`")?;
                    return Ok(Expr::Call { function, args });
                }
                Ok(Expr::Path(vec![String::from(word)]))
            }
            _ => Err(Diagnostic {
                kind: DiagnosticKind::General,
                message: String::from("expected an expression"),
                range: token.range,
            }),
        }
    }
}

/// The structural nesting of a parsed expression, measured with an
/// explicit stack — the walk that bounds every later recursion over
/// the tree (type-check, hash, lowering, and the exact serialized
/// walker at validation). Parser literals are scalars, so only the
/// expression spine counts.
fn structural_depth(root: &Expr) -> usize {
    let mut max = 0;
    let mut stack = vec![(root, 1usize)];
    while let Some((expr, depth)) = stack.pop() {
        max = max.max(depth);
        match expr {
            Expr::Literal(_) | Expr::Path(_) | Expr::Asset(_) => {}
            Expr::Unary { expr, .. } => stack.push((expr, depth + 1)),
            Expr::Binary { left, right, .. } => {
                stack.push((left, depth + 1));
                stack.push((right, depth + 1));
            }
            Expr::Ternary {
                cond,
                then,
                otherwise,
            } => {
                stack.push((cond, depth + 1));
                stack.push((then, depth + 1));
                stack.push((otherwise, depth + 1));
            }
            Expr::Index { base, index } => {
                stack.push((base, depth + 1));
                stack.push((index, depth + 1));
            }
            Expr::Call { args, .. } => {
                for arg in args {
                    stack.push((arg, depth + 1));
                }
            }
            Expr::List(items) => {
                for item in items {
                    stack.push((item, depth + 1));
                }
            }
            Expr::MapLit(pairs) => {
                for (_, value) in pairs {
                    stack.push((value, depth + 1));
                }
            }
        }
    }
    max
}

/// Consumes a refused tree with an explicit stack — a hostile-depth
/// chain must not recurse through its own destruction. Moving the
/// children out leaves each node a leaf, so its ordinary drop is
/// flat.
fn dismantle(root: Expr) {
    let mut stack = vec![root];
    while let Some(expr) = stack.pop() {
        match expr {
            Expr::Literal(_) | Expr::Path(_) | Expr::Asset(_) => {}
            Expr::Unary { expr, .. } => stack.push(*expr),
            Expr::Binary { left, right, .. } => {
                stack.push(*left);
                stack.push(*right);
            }
            Expr::Ternary {
                cond,
                then,
                otherwise,
            } => {
                stack.push(*cond);
                stack.push(*then);
                stack.push(*otherwise);
            }
            Expr::Index { base, index } => {
                stack.push(*base);
                stack.push(*index);
            }
            Expr::Call { args, .. } => stack.extend(args),
            Expr::List(items) => stack.extend(items),
            Expr::MapLit(pairs) => stack.extend(pairs.into_iter().map(|(_, value)| value)),
        }
    }
}

fn binary(op: BinaryOp, left: Expr, right: Expr) -> Expr {
    Expr::Binary {
        op,
        left: Box::new(left),
        right: Box::new(right),
    }
}

// ===== type checking =====

fn check(expr: &Expr, signature: &Signature, text: &str) -> Result<Type, Diagnostic> {
    let whole = 0..text.len();
    let fail = |message: String| Diagnostic {
        kind: DiagnosticKind::General,
        message,
        range: whole.clone(),
    };
    match expr {
        Expr::Literal(value) => Ok(literal_type(value)),
        // Compiler-constructed only — CEL text never spells one.
        Expr::Asset(_) => Ok(Type::Bytes),
        Expr::Path(segments) => {
            let dotted = segments.join(".");
            if let Some(ty) = signature.get(&dotted) {
                return Ok(ty.clone());
            }
            match signature.get(&segments[0]) {
                Some(Type::Dyn) => Ok(Type::Dyn),
                Some(Type::Map(values)) if segments.len() > 1 => Ok((**values).clone()),
                Some(other) if segments.len() == 1 => Ok(other.clone()),
                Some(_) => Err(fail(format!("`{dotted}` does not resolve to a member"))),
                None => Err(fail(format!("`{}` is not a visible name", segments[0]))),
            }
        }
        Expr::Unary { op, expr } => {
            let inner = check(expr, signature, text)?;
            match (op, &inner) {
                (_, Type::Dyn) => Ok(Type::Dyn),
                (UnaryOp::Not, Type::Bool) => Ok(Type::Bool),
                (UnaryOp::Neg, Type::Int | Type::Double | Type::Duration) => Ok(inner),
                _ => Err(fail(format!("operator does not accept a {inner:?}"))),
            }
        }
        Expr::Binary { op, left, right } => {
            let lhs = check(left, signature, text)?;
            let rhs = check(right, signature, text)?;
            binary_type(*op, &lhs, &rhs).ok_or_else(|| {
                fail(format!(
                    "operator {op:?} does not accept {lhs:?} and {rhs:?}"
                ))
            })
        }
        Expr::Ternary {
            cond,
            then,
            otherwise,
        } => {
            let cond_ty = check(cond, signature, text)?;
            if !matches!(cond_ty, Type::Bool | Type::Dyn) {
                return Err(fail(String::from("the condition must be a bool")));
            }
            let then_ty = check(then, signature, text)?;
            let else_ty = check(otherwise, signature, text)?;
            Ok(join_types(&then_ty, &else_ty))
        }
        Expr::Index { base, index } => {
            let base_ty = check(base, signature, text)?;
            let index_ty = check(index, signature, text)?;
            match base_ty {
                Type::Dyn => Ok(Type::Dyn),
                Type::List(element) => {
                    if matches!(index_ty, Type::Int | Type::Uint | Type::Dyn) {
                        Ok(*element)
                    } else {
                        Err(fail(String::from("a list index must be an integer")))
                    }
                }
                Type::Map(values) => {
                    if matches!(index_ty, Type::Text | Type::Dyn) {
                        Ok(*values)
                    } else {
                        Err(fail(String::from("a map key must be text")))
                    }
                }
                other => Err(fail(format!("a {other:?} cannot be indexed"))),
            }
        }
        Expr::Call { function, args } => match function {
            Function::Duration | Function::Timestamp | Function::Decimal => {
                if args.len() != 1 {
                    return Err(fail(String::from("this function takes one argument")));
                }
                let arg = check(&args[0], signature, text)?;
                if !matches!(arg, Type::Text | Type::Dyn) {
                    return Err(fail(String::from("the argument must be text")));
                }
                Ok(match function {
                    Function::Duration => Type::Duration,
                    // A dynamic decimal conforms to no precise M0
                    // type — `variant`, narrowed before use
                    // (D-2026-08-10-7).
                    Function::Decimal => Type::Dyn,
                    _ => Type::Timestamp,
                })
            }
            Function::Size => {
                if args.len() != 1 {
                    return Err(fail(String::from("`size` takes one argument")));
                }
                let arg = check(&args[0], signature, text)?;
                match arg {
                    Type::Text | Type::Bytes | Type::List(_) | Type::Map(_) | Type::Dyn => {
                        Ok(Type::Int)
                    }
                    other => Err(fail(format!("`size` does not accept a {other:?}"))),
                }
            }
        },
        Expr::List(items) => {
            let mut element = Type::Dyn;
            for (index, item) in items.iter().enumerate() {
                let ty = check(item, signature, text)?;
                if index == 0 {
                    element = ty;
                } else {
                    element = join_types(&element, &ty);
                }
            }
            Ok(Type::List(Box::new(element)))
        }
        Expr::MapLit(entries) => {
            let mut values = Type::Dyn;
            for (index, (_, value)) in entries.iter().enumerate() {
                let ty = check(value, signature, text)?;
                if index == 0 {
                    values = ty;
                } else {
                    values = join_types(&values, &ty);
                }
            }
            Ok(Type::Map(Box::new(values)))
        }
    }
}

fn literal_type(value: &Value) -> Type {
    match value {
        Value::Bool(_) => Type::Bool,
        Value::Int(_) => Type::Int,
        Value::Uint(_) => Type::Uint,
        Value::Double(_) => Type::Double,
        Value::Text(_) => Type::Text,
        Value::Bytes(_) => Type::Bytes,
        Value::Duration(_) => Type::Duration,
        Value::Timestamp(_) => Type::Timestamp,
        Value::Null => Type::Null,
        Value::List(_) => Type::List(Box::new(Type::Dyn)),
        Value::Map(_) => Type::Map(Box::new(Type::Dyn)),
        // No struct type exists in the annotation subset; the value
        // carries its own witness and types as dynamic.
        Value::Struct(_) => Type::Dyn,
        // The dynamic decimal types as Dyn until narrowing exists.
        Value::Decimal { .. } => Type::Dyn,
        // A literal is never a witness; total anyway.
        Value::Payload => Type::Dyn,
    }
}

fn join_types(left: &Type, right: &Type) -> Type {
    if left == right {
        left.clone()
    } else {
        Type::Dyn
    }
}

fn binary_type(op: BinaryOp, lhs: &Type, rhs: &Type) -> Option<Type> {
    use BinaryOp::{Add, And, Div, Eq, Ge, Gt, Le, Lt, Mul, Ne, Or, Rem, Sub};
    if matches!(lhs, Type::Dyn) || matches!(rhs, Type::Dyn) {
        return Some(match op {
            Eq | Ne | Lt | Le | Gt | Ge | And | Or => Type::Bool,
            _ => Type::Dyn,
        });
    }
    match op {
        And | Or => (lhs == &Type::Bool && rhs == &Type::Bool).then_some(Type::Bool),
        Eq | Ne => Some(Type::Bool),
        Lt | Le | Gt | Ge => {
            let comparable = matches!(
                lhs,
                Type::Int
                    | Type::Uint
                    | Type::Double
                    | Type::Text
                    | Type::Duration
                    | Type::Timestamp
            );
            (comparable && lhs == rhs).then_some(Type::Bool)
        }
        Add => match (lhs, rhs) {
            (Type::Text, Type::Text) => Some(Type::Text),
            (Type::Duration, Type::Duration) => Some(Type::Duration),
            (Type::Timestamp, Type::Duration) | (Type::Duration, Type::Timestamp) => {
                Some(Type::Timestamp)
            }
            _ => numeric(lhs, rhs),
        },
        Sub => match (lhs, rhs) {
            (Type::Duration, Type::Duration) => Some(Type::Duration),
            (Type::Timestamp, Type::Duration) => Some(Type::Timestamp),
            (Type::Timestamp, Type::Timestamp) => Some(Type::Duration),
            _ => numeric(lhs, rhs),
        },
        Mul | Div | Rem => numeric(lhs, rhs),
    }
}

fn numeric(lhs: &Type, rhs: &Type) -> Option<Type> {
    (lhs == rhs && matches!(lhs, Type::Int | Type::Uint | Type::Double)).then(|| lhs.clone())
}

// ===== evaluation =====

/// Upgrades a payload witness at a point of STRUCTURAL consumption;
/// every other value passes through. Carrying positions (a `val`
/// binding, a ternary result, a list element) never call this — that
/// is what keeps `val copy = data` as lazy as `data` itself.
fn concrete(value: Value, bindings: &dyn Bindings) -> Result<Value, Error> {
    match value {
        Value::Payload => bindings.payload(),
        other => Ok(other),
    }
}

/// The deep variant for comparisons: a witness inside a composite
/// materializes too, so `[data] == [x]` compares values, never
/// markers.
fn concrete_deep(value: Value, bindings: &dyn Bindings) -> Result<Value, Error> {
    Ok(match value {
        Value::Payload => bindings.payload()?,
        Value::List(items) => Value::List(
            items
                .into_iter()
                .map(|item| concrete_deep(item, bindings))
                .collect::<Result<_, _>>()?,
        ),
        Value::Map(map) => {
            let mut deep = BTreeMap::new();
            for (key, value) in map {
                deep.insert(key, concrete_deep(value, bindings)?);
            }
            Value::Map(deep)
        }
        Value::Struct(fields) => Value::Struct(
            fields
                .into_iter()
                .map(|(name, value)| Ok((name, concrete_deep(value, bindings)?)))
                .collect::<Result<_, _>>()?,
        ),
        other => other,
    })
}

fn eval_expr(expr: &Expr, bindings: &dyn Bindings) -> Result<Value, Error> {
    match expr {
        Expr::Literal(value) => Ok(value.clone()),
        Expr::Asset(index) => bindings.asset(*index),
        Expr::Path(segments) => {
            let dotted = segments.join(".");
            if let Some(value) = bindings.get(&dotted) {
                return Ok(value);
            }
            let mut value = bindings
                .get(&segments[0])
                .ok_or_else(|| fault(format!("`{}` is not bound", segments[0])))?;
            for segment in &segments[1..] {
                // A member access IS the structural demand.
                value = concrete(value, bindings)?;
                match value {
                    Value::Map(mut map) => {
                        value = map.remove(segment.as_str()).ok_or_else(|| {
                            fault(format!("`{dotted}` has no member `{segment}`"))
                        })?;
                    }
                    Value::Struct(fields) => {
                        value = fields
                            .into_iter()
                            .find(|(name, _)| name == segment)
                            .map(|(_, value)| value)
                            .ok_or_else(|| {
                                fault(format!("`{dotted}` has no member `{segment}`"))
                            })?;
                    }
                    _ => return Err(fault(format!("`{dotted}` is not a map"))),
                }
            }
            Ok(value)
        }
        Expr::Unary { op, expr } => {
            let value = concrete(eval_expr(expr, bindings)?, bindings)?;
            match (op, value) {
                (UnaryOp::Not, Value::Bool(b)) => Ok(Value::Bool(!b)),
                (UnaryOp::Neg, Value::Int(i)) => i
                    .checked_neg()
                    .map(Value::Int)
                    .ok_or_else(|| fault(String::from("integer overflow"))),
                (UnaryOp::Neg, Value::Double(d)) => Ok(Value::Double(-d)),
                (UnaryOp::Neg, Value::Duration(d)) => d
                    .checked_neg()
                    .map(Value::Duration)
                    .ok_or_else(|| fault(String::from("duration overflow"))),
                (_, other) => Err(fault(format!("operator does not accept {other:?}"))),
            }
        }
        Expr::Binary { op, left, right } => eval_binary(*op, left, right, bindings),
        Expr::Ternary {
            cond,
            then,
            otherwise,
        } => match concrete(eval_expr(cond, bindings)?, bindings)? {
            Value::Bool(true) => eval_expr(then, bindings),
            Value::Bool(false) => eval_expr(otherwise, bindings),
            other => Err(fault(format!("the condition evaluated to {other:?}"))),
        },
        Expr::Index { base, index } => {
            let base = concrete(eval_expr(base, bindings)?, bindings)?;
            let index = concrete(eval_expr(index, bindings)?, bindings)?;
            match (base, index) {
                (Value::List(items), Value::Int(i)) => usize::try_from(i)
                    .ok()
                    .and_then(|i| items.into_iter().nth(i))
                    .ok_or_else(|| fault(String::from("list index out of range"))),
                (Value::List(items), Value::Uint(i)) => usize::try_from(i)
                    .ok()
                    .and_then(|i| items.into_iter().nth(i))
                    .ok_or_else(|| fault(String::from("list index out of range"))),
                (Value::Struct(fields), Value::Text(key)) => fields
                    .into_iter()
                    .find(|(name, _)| name == &key)
                    .map(|(_, value)| value)
                    .ok_or_else(|| fault(format!("no member `{key}`"))),
                (Value::Map(mut map), Value::Text(key)) => map
                    .remove(&key)
                    .ok_or_else(|| fault(format!("no such key `{key}`"))),
                (base, index) => Err(fault(format!("cannot index {base:?} with {index:?}"))),
            }
        }
        Expr::Call { function, args } => {
            let mut values = Vec::with_capacity(args.len());
            for arg in args {
                // A function consumes its argument's structure.
                values.push(concrete(eval_expr(arg, bindings)?, bindings)?);
            }
            match (function, values.as_slice()) {
                (Function::Duration, [Value::Text(text)]) => {
                    parse_duration(text).map(Value::Duration)
                }
                (Function::Timestamp, [Value::Text(text)]) => {
                    parse_timestamp(text).map(Value::Timestamp)
                }
                (Function::Decimal, [Value::Text(text)]) => {
                    match crate::json::parse_declared_number(text) {
                        Ok(crate::json::DeclaredNumber::Int(value)) => Ok(Value::Int(value)),
                        Ok(crate::json::DeclaredNumber::Decimal { unscaled, scale }) => {
                            Ok(Value::Decimal { unscaled, scale })
                        }
                        Err(reason) => Err(fault(alloc::format!(
                            "`{text}` is not an exact decimal: {reason}"
                        ))),
                    }
                }
                (Function::Size, [Value::Text(text)]) => {
                    Ok(Value::Int(text.chars().count() as i64))
                }
                (Function::Size, [Value::Bytes(bytes)]) => Ok(Value::Int(bytes.len() as i64)),
                (Function::Size, [Value::List(items)]) => Ok(Value::Int(items.len() as i64)),
                (Function::Size, [Value::Map(map)]) => Ok(Value::Int(map.len() as i64)),
                (Function::Size, [Value::Struct(fields)]) => Ok(Value::Int(fields.len() as i64)),
                (function, values) => {
                    Err(fault(format!("{function:?} does not accept {values:?}")))
                }
            }
        }
        Expr::List(items) => {
            let mut values = Vec::with_capacity(items.len());
            for item in items {
                values.push(eval_expr(item, bindings)?);
            }
            Ok(Value::List(values))
        }
        Expr::MapLit(entries) => {
            let mut map = BTreeMap::new();
            for (key, value) in entries {
                map.insert(key.clone(), eval_expr(value, bindings)?);
            }
            Ok(Value::Map(map))
        }
    }
}

fn eval_binary(
    op: BinaryOp,
    left: &Expr,
    right: &Expr,
    bindings: &dyn Bindings,
) -> Result<Value, Error> {
    use BinaryOp::{Add, And, Div, Eq, Ge, Gt, Le, Lt, Mul, Ne, Or, Rem, Sub};
    if matches!(op, And | Or) {
        let Value::Bool(lhs) = concrete(eval_expr(left, bindings)?, bindings)? else {
            return Err(fault(String::from("logic needs booleans")));
        };
        return match (op, lhs) {
            (And, false) => Ok(Value::Bool(false)),
            (Or, true) => Ok(Value::Bool(true)),
            _ => match concrete(eval_expr(right, bindings)?, bindings)? {
                Value::Bool(rhs) => Ok(Value::Bool(rhs)),
                _ => Err(fault(String::from("logic needs booleans"))),
            },
        };
    }
    let lhs = concrete_deep(eval_expr(left, bindings)?, bindings)?;
    let rhs = concrete_deep(eval_expr(right, bindings)?, bindings)?;
    match op {
        Eq => Ok(Value::Bool(values_equal(&lhs, &rhs))),
        Ne => Ok(Value::Bool(!values_equal(&lhs, &rhs))),
        Lt | Le | Gt | Ge => {
            let ordering = compare(&lhs, &rhs)?;
            Ok(Value::Bool(match op {
                Lt => ordering.is_lt(),
                Le => ordering.is_le(),
                Gt => ordering.is_gt(),
                _ => ordering.is_ge(),
            }))
        }
        Add | Sub | Mul | Div | Rem => arithmetic(op, lhs, rhs),
        And | Or => unreachable!("handled above"),
    }
}

fn values_equal(lhs: &Value, rhs: &Value) -> bool {
    match (lhs, rhs) {
        (Value::Bool(a), Value::Bool(b)) => a == b,
        // Structural on the canonical pair — a dynamic decimal is
        // minimal-scale by construction, so it never numerically
        // equals an int it could have been (D-2026-08-10-7).
        (
            Value::Decimal { unscaled, scale },
            Value::Decimal {
                unscaled: other_unscaled,
                scale: other_scale,
            },
        ) => unscaled == other_unscaled && scale == other_scale,
        (Value::Int(a), Value::Int(b)) => a == b,
        (Value::Uint(a), Value::Uint(b)) => a == b,
        (Value::Double(a), Value::Double(b)) => a == b,
        (Value::Text(a), Value::Text(b)) => a == b,
        (Value::Bytes(a), Value::Bytes(b)) => a == b,
        (Value::Duration(a), Value::Duration(b)) => a == b,
        (Value::Timestamp(a), Value::Timestamp(b)) => a == b,
        (Value::Null, Value::Null) => true,
        (Value::List(a), Value::List(b)) => {
            a.len() == b.len() && a.iter().zip(b).all(|(x, y)| values_equal(x, y))
        }
        (Value::Struct(a), Value::Struct(b)) => {
            // Sorted like the canonical seal — CEL `==` must not
            // split two values one identity owns, and stays total on
            // malformed duplicates (see `struct_fields_eq`).
            struct_fields_eq(a, b, values_equal)
        }
        (Value::Map(a), Value::Map(b)) => {
            a.len() == b.len()
                && a.iter()
                    .zip(b)
                    .all(|((ka, va), (kb, vb))| ka == kb && values_equal(va, vb))
        }
        _ => false,
    }
}

fn compare(lhs: &Value, rhs: &Value) -> Result<core::cmp::Ordering, Error> {
    match (lhs, rhs) {
        (Value::Int(a), Value::Int(b)) => Ok(a.cmp(b)),
        (Value::Uint(a), Value::Uint(b)) => Ok(a.cmp(b)),
        (Value::Double(a), Value::Double(b)) => a
            .partial_cmp(b)
            .ok_or_else(|| fault(String::from("NaN is not ordered"))),
        (Value::Text(a), Value::Text(b)) => Ok(a.cmp(b)),
        (Value::Duration(a), Value::Duration(b)) => Ok(a.cmp(b)),
        (Value::Timestamp(a), Value::Timestamp(b)) => Ok(a.cmp(b)),
        (lhs, rhs) => Err(fault(format!("cannot order {lhs:?} against {rhs:?}"))),
    }
}

fn arithmetic(op: BinaryOp, lhs: Value, rhs: Value) -> Result<Value, Error> {
    use BinaryOp::{Add, Div, Mul, Rem, Sub};
    let overflow = || fault(String::from("integer overflow"));
    match (op, lhs, rhs) {
        (Add, Value::Int(a), Value::Int(b)) => {
            a.checked_add(b).map(Value::Int).ok_or_else(overflow)
        }
        (Sub, Value::Int(a), Value::Int(b)) => {
            a.checked_sub(b).map(Value::Int).ok_or_else(overflow)
        }
        (Mul, Value::Int(a), Value::Int(b)) => {
            a.checked_mul(b).map(Value::Int).ok_or_else(overflow)
        }
        (Div, Value::Int(a), Value::Int(b)) => a
            .checked_div(b)
            .map(Value::Int)
            .ok_or_else(|| fault(String::from("division by zero"))),
        (Rem, Value::Int(a), Value::Int(b)) => a
            .checked_rem(b)
            .map(Value::Int)
            .ok_or_else(|| fault(String::from("division by zero"))),
        (Add, Value::Uint(a), Value::Uint(b)) => {
            a.checked_add(b).map(Value::Uint).ok_or_else(overflow)
        }
        (Sub, Value::Uint(a), Value::Uint(b)) => {
            a.checked_sub(b).map(Value::Uint).ok_or_else(overflow)
        }
        (Mul, Value::Uint(a), Value::Uint(b)) => {
            a.checked_mul(b).map(Value::Uint).ok_or_else(overflow)
        }
        (Div, Value::Uint(a), Value::Uint(b)) => a
            .checked_div(b)
            .map(Value::Uint)
            .ok_or_else(|| fault(String::from("division by zero"))),
        (Rem, Value::Uint(a), Value::Uint(b)) => a
            .checked_rem(b)
            .map(Value::Uint)
            .ok_or_else(|| fault(String::from("division by zero"))),
        (Add, Value::Double(a), Value::Double(b)) => Ok(Value::Double(a + b)),
        (Sub, Value::Double(a), Value::Double(b)) => Ok(Value::Double(a - b)),
        (Mul, Value::Double(a), Value::Double(b)) => Ok(Value::Double(a * b)),
        (Div, Value::Double(a), Value::Double(b)) => Ok(Value::Double(a / b)),
        (Add, Value::Text(a), Value::Text(b)) => Ok(Value::Text(a + &b)),
        (Add, Value::Duration(a), Value::Duration(b)) => {
            a.checked_add(b).map(Value::Duration).ok_or_else(overflow)
        }
        (Sub, Value::Duration(a), Value::Duration(b)) => {
            a.checked_sub(b).map(Value::Duration).ok_or_else(overflow)
        }
        (Add, Value::Timestamp(a), Value::Duration(b))
        | (Add, Value::Duration(b), Value::Timestamp(a)) => {
            a.checked_add(b).map(Value::Timestamp).ok_or_else(overflow)
        }
        (Sub, Value::Timestamp(a), Value::Duration(b)) => {
            a.checked_sub(b).map(Value::Timestamp).ok_or_else(overflow)
        }
        (Sub, Value::Timestamp(a), Value::Timestamp(b)) => {
            a.checked_sub(b).map(Value::Duration).ok_or_else(overflow)
        }
        (op, lhs, rhs) => Err(fault(format!("{op:?} does not accept {lhs:?} and {rhs:?}"))),
    }
}

fn fault(message: String) -> Error {
    Error { message }
}

// ===== time parsing =====

/// Parses the CEL/Go duration format: `ns`/`us`/`ms`/`s`/`m`/`h`,
/// compound and fractional segments, optional leading sign. `min`,
/// `d`, and `w` are not accepted.
fn parse_duration(text: &str) -> Result<i64, Error> {
    let bad = || fault(format!("`{text}` is not a valid duration"));
    let (negative, rest) = match text.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, text),
    };
    if rest.is_empty() {
        return Err(bad());
    }
    let bytes = rest.as_bytes();
    let mut pos = 0;
    let mut total: i64 = 0;
    while pos < bytes.len() {
        let start = pos;
        while pos < bytes.len() && (bytes[pos].is_ascii_digit() || bytes[pos] == b'.') {
            pos += 1;
        }
        let number: f64 = rest[start..pos].parse().map_err(|_| bad())?;
        let unit_start = pos;
        while pos < bytes.len() && bytes[pos].is_ascii_alphabetic() {
            pos += 1;
        }
        let scale: i64 = match &rest[unit_start..pos] {
            "ns" => 1,
            "us" => 1_000,
            "ms" => 1_000_000,
            "s" => 1_000_000_000,
            "m" => 60_000_000_000,
            "h" => 3_600_000_000_000,
            _ => return Err(bad()),
        };
        let nanos = number * scale as f64;
        if !nanos.is_finite() || nanos > i64::MAX as f64 {
            return Err(bad());
        }
        total = total.checked_add(nanos as i64).ok_or_else(bad)?;
    }
    Ok(if negative { -total } else { total })
}

/// Parses an RFC 3339 UTC timestamp (`Z` suffix only) to nanoseconds
/// since the Unix epoch.
fn parse_timestamp(text: &str) -> Result<i64, Error> {
    let bad = || fault(format!("`{text}` is not a valid UTC timestamp"));
    // RFC 3339 has fixed widths; checking them first also bounds every
    // component before any arithmetic, so nothing below can overflow.
    let field = |part: Option<&str>, width: usize| -> Result<i64, Error> {
        let part = part.ok_or_else(bad)?;
        if part.len() != width || !part.bytes().all(|byte| byte.is_ascii_digit()) {
            return Err(bad());
        }
        part.parse().map_err(|_| bad())
    };
    let rest = text.strip_suffix('Z').ok_or_else(bad)?;
    let (date, time) = rest.split_once('T').ok_or_else(bad)?;
    let mut date_parts = date.splitn(3, '-');
    let year = field(date_parts.next(), 4)?;
    let month = field(date_parts.next(), 2)?;
    let day = field(date_parts.next(), 2)?;
    let (clock, fraction) = match time.split_once('.') {
        Some((clock, fraction)) => (clock, Some(fraction)),
        None => (time, None),
    };
    let mut clock_parts = clock.splitn(3, ':');
    let hour = field(clock_parts.next(), 2)?;
    let minute = field(clock_parts.next(), 2)?;
    let second = field(clock_parts.next(), 2)?;
    if !(1..=12).contains(&month)
        || !(0..24).contains(&hour)
        || !(0..60).contains(&minute)
        || !(0..60).contains(&second)
    {
        return Err(bad());
    }
    // A timestamp-shaped string is not yet a timestamp: the calendar
    // must agree (no February 31st).
    let leap = (year % 4 == 0 && year % 100 != 0) || year % 400 == 0;
    let days_in_month = match month {
        4 | 6 | 9 | 11 => 30,
        2 => {
            if leap {
                29
            } else {
                28
            }
        }
        _ => 31,
    };
    if !(1..=days_in_month).contains(&day) {
        return Err(bad());
    }
    let nanos_fraction: i64 = match fraction {
        None => 0,
        Some(digits) => {
            if digits.is_empty() || digits.len() > 9 || !digits.bytes().all(|b| b.is_ascii_digit())
            {
                return Err(bad());
            }
            let value: i64 = digits.parse().map_err(|_| bad())?;
            value * 10_i64.pow(9 - digits.len() as u32)
        }
    };
    let days = days_from_civil(year, month, day);
    let seconds = days * 86_400 + hour * 3_600 + minute * 60 + second;
    seconds
        .checked_mul(1_000_000_000)
        .and_then(|nanos| nanos.checked_add(nanos_fraction))
        .ok_or_else(bad)
}

/// Days since 1970-01-01 for a proleptic-Gregorian civil date
/// (Howard Hinnant's `days_from_civil`).
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let adjusted_year = if month <= 2 { year - 1 } else { year };
    let era = adjusted_year.div_euclid(400);
    let year_of_era = adjusted_year - era * 400;
    let month_shifted = if month > 2 { month - 3 } else { month + 9 };
    let day_of_year = (153 * month_shifted + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn struct_equality_is_reflexive_even_on_malformed_values() {
        // `Eq`'s contract is unconditional: a duplicate-name struct
        // is refused at every door, but it is REPRESENTABLE — and a
        // representable value must equal itself. Sorted comparison
        // keeps the door refusals AND the contract.
        let duplicated = Value::Struct(alloc::vec![
            (String::from("x"), Value::Int(1)),
            (String::from("x"), Value::Int(2)),
        ]);
        assert_eq!(duplicated, duplicated.clone(), "reflexive");
        assert!(
            values_equal(&duplicated, &duplicated),
            "the CEL `==` agrees"
        );
        // Symmetry, on two DIFFERENT malformed spellings.
        let reversed = Value::Struct(alloc::vec![
            (String::from("x"), Value::Int(2)),
            (String::from("x"), Value::Int(1)),
        ]);
        assert_eq!(duplicated == reversed, reversed == duplicated, "symmetric");
        // And the well-formed case still compares by name.
        let ab = Value::Struct(alloc::vec![
            (String::from("a"), Value::Int(1)),
            (String::from("b"), Value::Int(2)),
        ]);
        let ba = Value::Struct(alloc::vec![
            (String::from("b"), Value::Int(2)),
            (String::from("a"), Value::Int(1)),
        ]);
        assert_eq!(ab, ba, "order is presentation");
        assert!(values_equal(&ab, &ba));
    }

    fn compile_ok(text: &str, signature: &Signature) -> Tree {
        compile(text, signature).unwrap_or_else(|diagnostics| {
            panic!("expected `{text}` to compile, got {diagnostics:?}")
        })
    }

    fn eval_const(text: &str) -> Value {
        let tree = compile_ok(text, &Signature::new());
        eval(&tree, &BTreeMap::new()).unwrap()
    }

    #[test]
    fn literals_and_arithmetic_evaluate() {
        assert_eq!(eval_const("1 + 2 * 3"), Value::Int(7));
        assert_eq!(eval_const("(1 + 2) * 3"), Value::Int(9));
        assert_eq!(eval_const("7 % 3"), Value::Int(1));
        assert_eq!(eval_const("1.5 + 0.25"), Value::Double(1.75));
        assert_eq!(eval_const("\"a\" + 'b'"), Value::Text(String::from("ab")));
        assert_eq!(eval_const("-3"), Value::Int(-3));
        assert_eq!(eval_const("3u + 4u"), Value::Uint(7));
    }

    #[test]
    fn comparisons_ternary_and_logic_evaluate() {
        assert_eq!(eval_const("1 < 2 && 2 <= 2"), Value::Bool(true));
        assert_eq!(eval_const("\"a\" != \"b\""), Value::Bool(true));
        assert_eq!(eval_const("true ? 1 : 2"), Value::Int(1));
        assert_eq!(eval_const("null == null"), Value::Bool(true));
        assert_eq!(eval_const("1 == \"1\""), Value::Bool(false));
        assert_eq!(eval_const("!false || false"), Value::Bool(true));
    }

    /// The CEL `==` OPERATOR keeps IEEE semantics — `values_equal`,
    /// not the structural `PartialEq` (which compares canonical bits
    /// so plans stay equal to themselves across the byte trip). The
    /// two equalities answer different questions and must not bleed
    /// into each other.
    #[test]
    fn the_cel_operator_keeps_ieee_while_structure_keeps_bits() {
        // IEEE, through the operator: NaN never equals itself, and
        // the two zeros are numerically one.
        assert_eq!(eval_const("0.0 / 0.0 == 0.0 / 0.0"), Value::Bool(false));
        assert_eq!(eval_const("-0.0 == 0.0"), Value::Bool(true));
        // Bits, through the structure: NaN is ONE value, the zeros
        // are TWO.
        assert_eq!(eval_const("0.0 / 0.0"), eval_const("0.0 / 0.0"));
        assert_ne!(eval_const("-0.0"), eval_const("0.0"));
    }

    #[test]
    fn logic_short_circuits() {
        // The right side would divide by zero; short-circuit skips it.
        assert_eq!(eval_const("false && 1 / 0 == 0"), Value::Bool(false));
        assert_eq!(eval_const("true || 1 / 0 == 0"), Value::Bool(true));
    }

    #[test]
    fn value_errors_are_values_not_panics() {
        let tree = compile_ok("1 / 0", &Signature::new());
        assert_eq!(
            eval(&tree, &BTreeMap::new()).unwrap_err().message,
            "division by zero"
        );
        let overflow = compile_ok("9223372036854775807 + 1", &Signature::new());
        assert_eq!(
            eval(&overflow, &BTreeMap::new()).unwrap_err().message,
            "integer overflow"
        );
    }

    #[test]
    fn paths_resolve_through_bindings() {
        let signature = Signature::new()
            .with("data", Type::Dyn)
            .with("session.key", Type::Text);
        let tree = compile_ok("data.text != \"\" && session.key == \"s-1\"", &signature);
        assert_eq!(tree.ty, Type::Bool);
        let mut data = BTreeMap::new();
        data.insert(String::from("text"), Value::Text(String::from("hello")));
        let mut bindings = BTreeMap::new();
        bindings.insert(String::from("data"), Value::Map(data));
        bindings.insert(
            String::from("session.key"),
            Value::Text(String::from("s-1")),
        );
        assert_eq!(eval(&tree, &bindings).unwrap(), Value::Bool(true));
    }

    #[test]
    fn unknown_names_fail_at_compile_time() {
        let diagnostics = compile("mystery + 1", &Signature::new()).unwrap_err();
        assert!(diagnostics[0].message.contains("not a visible name"));
        let typed = compile(
            "session.key + 1",
            &Signature::new().with("session.key", Type::Text),
        );
        assert!(typed.is_err(), "text + int must not type-check");
    }

    #[test]
    fn collections_index_and_size() {
        assert_eq!(eval_const("[1, 2, 3][1]"), Value::Int(2));
        assert_eq!(eval_const("{'a': 1, 'b': 2}['b']"), Value::Int(2));
        assert_eq!(eval_const("size([1, 2, 3])"), Value::Int(3));
        assert_eq!(eval_const("size(\"héllo\")"), Value::Int(5));
        assert_eq!(eval_const("size({'a': 1})"), Value::Int(1));
    }

    #[test]
    fn durations_follow_the_go_format() {
        assert_eq!(
            eval_const("duration(\"2h\")"),
            Value::Duration(7_200_000_000_000)
        );
        assert_eq!(
            eval_const("duration(\"2h30m\")"),
            Value::Duration(9_000_000_000_000)
        );
        assert_eq!(
            eval_const("duration(\"1.5s\")"),
            Value::Duration(1_500_000_000)
        );
        assert_eq!(
            eval_const("duration(\"-10ms\")"),
            Value::Duration(-10_000_000)
        );
        assert_eq!(
            eval_const("duration(\"1h\") + duration(\"30m\")"),
            Value::Duration(5_400_000_000_000)
        );
        let tree = compile_ok("duration(\"2d\")", &Signature::new());
        assert!(eval(&tree, &BTreeMap::new()).is_err(), "`d` is not a unit");
        let minutes = compile_ok("duration(\"5min\")", &Signature::new());
        assert!(
            eval(&minutes, &BTreeMap::new()).is_err(),
            "`min` is not a unit"
        );
    }

    #[test]
    fn timestamps_are_utc_rfc3339() {
        assert_eq!(
            eval_const("timestamp(\"1970-01-01T00:00:00Z\")"),
            Value::Timestamp(0)
        );
        assert_eq!(
            eval_const("timestamp(\"2026-01-05T00:00:00Z\")"),
            Value::Timestamp(1_767_571_200_000_000_000)
        );
        assert_eq!(
            eval_const("timestamp(\"2026-01-05T01:00:00Z\") - timestamp(\"2026-01-05T00:00:00Z\")"),
            Value::Duration(3_600_000_000_000)
        );
        let offset = compile_ok(
            "timestamp(\"2026-01-05T00:00:00+01:00\")",
            &Signature::new(),
        );
        assert!(eval(&offset, &BTreeMap::new()).is_err(), "UTC only");
    }

    #[test]
    fn the_guard_rails_refuse_explicitly() {
        let cases: &[(&str, &str)] = &[
            ("now()", "no `now()`"),
            ("data.?text", "outside this build's subset"),
            ("items.map(i, i * 2)", "outside this build's subset"),
            ("has(data.text)", "macro is outside"),
            ("1 in [1]", "outside this build's subset"),
        ];
        let signature = Signature::new()
            .with("data", Type::Dyn)
            .with("items", Type::Dyn);
        for (text, needle) in cases {
            let diagnostics = compile(text, &signature).unwrap_err();
            assert!(
                diagnostics[0].message.contains(needle),
                "`{text}` → {diagnostics:?}"
            );
        }
    }

    #[test]
    fn duplicate_map_keys_are_refused_statically() {
        let diagnostics = compile("{'a': 1, 'a': 2}", &Signature::new()).unwrap_err();
        assert!(diagnostics[0].message.starts_with("duplicate map key"));
    }

    #[test]
    fn timestamps_have_fixed_widths_and_never_panic() {
        // RFC 3339 widths are fixed; and no user string may panic the
        // evaluator — the extreme year is a value error, not an abort.
        for bad in [
            "9223372036854775807-01-01T00:00:00Z",
            "2026-2-3T4:5:6Z",
            "26-01-01T00:00:00Z",
            "2026-01-01T00:00:00",
        ] {
            let tree = compile_ok(&format!("timestamp(\"{bad}\")"), &Signature::new());
            assert!(eval(&tree, &BTreeMap::new()).is_err(), "{bad} must refuse");
        }
    }

    #[test]
    fn timestamps_respect_the_calendar() {
        for bad in [
            "2026-02-31T00:00:00Z",
            "2026-04-31T00:00:00Z",
            "2025-02-29T00:00:00Z",
        ] {
            let tree = compile_ok(&format!("timestamp(\"{bad}\")"), &Signature::new());
            assert!(eval(&tree, &BTreeMap::new()).is_err(), "{bad} must refuse");
        }
        assert_eq!(
            eval_const("timestamp(\"2024-02-29T00:00:00Z\")"),
            eval_const("timestamp(\"2024-02-28T00:00:00Z\") + duration(\"24h\")")
        );
    }

    #[test]
    fn const_eval_reduces_initializers() {
        let tree = compile_ok("retries * 2", &Signature::new().with("retries", Type::Int));
        let mut consts = BTreeMap::new();
        consts.insert(String::from("retries"), Value::Int(3));
        assert_eq!(const_eval(&tree, &consts).unwrap(), Value::Int(6));
        assert!(const_eval(&tree, &BTreeMap::new()).is_err());
    }
}
