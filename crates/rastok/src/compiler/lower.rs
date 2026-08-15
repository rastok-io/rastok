//! Canonicalization and lowering: HIR to `SemanticHash` and
//! `ExecutionPlan`.
//!
//! The canonical byte form follows `identity.md` §1: insensitive to
//! formatting, file order, and declaration order — nodes, vals,
//! attributes, vars, ports, and edges are sorted before hashing —
//! sensitive to topology, statement order inside a handler, every Rastok expression
//! expression as its canonical tree, and `var` defaults. Constants
//! hash as the expressions they were written as, before const-eval;
//! spans never enter the bytes. The plan then ships the reduced
//! values.

use crate::compiler::hir::{
    AttrValue, Direction, EdgeTarget, EmitValue, Hir, Node as HirNode, NodeRole,
    Statement as HirStatement,
};
use crate::datatype::{
    BinaryType, DataType, DateUnit, FloatType, IntegerType, IntegerWidth, IntervalUnit,
    TemporalType, TimeUnit, TimeZone,
};
use crate::expr;
use crate::node;
use crate::plan::{
    AttrSetting, ConnectorSpec, Edge, Emission, EndpointRef, ExecutionPlan, FieldWrite, Handler,
    Node, NodeKind, SemanticHash, Slot, Step, SymbolTable, VarCell,
};

/// Computes the semantic hash of a checked graph.
#[must_use]
pub fn semantic_hash(hir: &Hir) -> SemanticHash {
    let mut writer = Writer {
        hasher: blake3::Hasher::new(),
        assets: &hir.assets,
    };
    writer.str(&hir.app.name);
    let boundary = sorted_boundary(hir);
    writer.count(boundary.len());
    for decl in &boundary {
        writer.str(decl.port.name());
        writer.direction(decl.direction);
        // The pinned type is identity material — a public contract;
        // absence hashes as absence (the var-init idiom). NOTE: the
        // absence tag itself moved every identity with a non-empty
        // table when it landed — declared in D-2026-08-08-3, pinned
        // by the gauge fixture.
        match &decl.ty {
            None => writer.tag(0),
            Some(ty) => {
                writer.tag(1);
                writer.ty(ty);
            }
        }
    }
    let mut vals: Vec<_> = hir.app.vals.iter().collect();
    vals.sort_by(|a, b| a.name.cmp(&b.name));
    writer.count(vals.len());
    for val in vals {
        writer.str(&val.name);
        writer.expr(&val.value.tree.root);
    }
    let mut nodes: Vec<_> = sorted_nodes(hir);
    writer.count(nodes.len());
    for node in &mut nodes {
        writer.node(node);
    }
    let edges = sorted_edges(hir);
    writer.count(edges.len());
    for (text, _) in edges {
        writer.str(&text);
    }
    let mut bytes = [0u8; 32];
    writer.hasher.finalize_xof().fill(&mut bytes);
    SemanticHash::from_bytes(bytes)
}

/// Boundary ports in canonical order — by (port, direction); the
/// optional pinned type never decides an order.
fn sorted_boundary(hir: &Hir) -> Vec<crate::plan::PortDecl> {
    let mut boundary = hir.app.boundary.clone();
    boundary.sort_by(|a, b| a.key().cmp(&b.key()));
    boundary
}

/// Edges in canonical order, keyed by their endpoint rendering — the
/// one order both the hash and the shipped plan use.
fn sorted_edges(hir: &Hir) -> Vec<(String, &crate::compiler::hir::Edge)> {
    let mut edges: Vec<_> = hir
        .app
        .edges
        .iter()
        .map(|edge| {
            let to = match &edge.to {
                EdgeTarget::Port(endpoint) => endpoint_text(endpoint),
                EdgeTarget::Discard => String::from("!discard"),
            };
            (format!("{}>{to}", endpoint_text(&edge.from)), edge)
        })
        .collect();
    edges.sort_by(|a, b| a.0.cmp(&b.0));
    edges
}

/// Lowers a checked graph into the shipped plan and its symbol table.
#[must_use]
pub fn lower(hir: &Hir) -> (ExecutionPlan, SymbolTable) {
    let nodes = sorted_nodes(hir);
    let slot_of = |path: &node::Path| {
        Slot::new(
            u32::try_from(
                nodes
                    .iter()
                    .position(|candidate| &candidate.path == path)
                    .unwrap_or_else(|| unreachable!("edges resolved against these nodes")),
            )
            .unwrap_or_else(|_| unreachable!("node count fits u32")),
        )
    };
    let mut plan = ExecutionPlan {
        nodes: Vec::new(),
        boundary: sorted_boundary(hir),
        edges: Vec::new(),
        handlers: Vec::new(),
        // DERIVED from the writes the hash already seals — carried
        // for the Registry-less validator (D-2026-08-10-1), never
        // hashed twice.
        app_schema: hir.app_schema.clone(),
        ext_schema: hir.ext_schema.clone(),
        assets: hir.assets.clone(),
    };
    let mut symbols = SymbolTable { nodes: Vec::new() };
    for hir_node in &nodes {
        symbols.nodes.push(hir_node.path.to_string());
        // Declaratively unordered collections ship in canonical order,
        // so equal hashes always mean equal plans.
        let mut attributes: Vec<_> = hir_node
            .attributes()
            .iter()
            .filter(|attribute| !matches!(attribute.name.as_str(), "effect" | "behavior"))
            .map(|attribute| {
                let setting = match &attribute.value {
                    AttrValue::Word(word) => AttrSetting::Word(word.clone()),
                    AttrValue::Const(constant) => AttrSetting::Value(constant.value.clone()),
                };
                (attribute.name.clone(), setting)
            })
            .collect();
        attributes.sort_by(|a, b| a.0.cmp(&b.0));
        let mut ports = executable_ports(hir_node);
        ports.sort_by(|a, b| a.key().cmp(&b.key()));
        let mut vars: Vec<_> = hir_node
            .vars
            .iter()
            .map(|var| VarCell {
                name: var.name.clone(),
                ty: var.ty.clone(),
                init: var.init.as_ref().map(|init| init.value.clone()),
            })
            .collect();
        vars.sort_by(|a, b| a.name.cmp(&b.name));
        plan.nodes.push(Node {
            path: hir_node.path.clone(),
            kind: match &hir_node.role {
                NodeRole::Connector { trait_name, .. } => NodeKind::Connector(ConnectorSpec {
                    contract: trait_name.clone(),
                    effect: typed_effect(hir_node),
                    behavior: typed_behavior(hir_node),
                    attributes,
                }),
                NodeRole::Operator => NodeKind::Operator,
            },
            ports,
            vars,
        });
    }
    for hir_node in &nodes {
        let slot = slot_of(&hir_node.path);
        // The plan must carry its whole semantics: every constant a
        // handler references is substituted into the shipped trees, so
        // no binding is left behind in the compiler.
        let mut consts: std::collections::BTreeMap<String, crate::value::Value> = hir
            .app
            .vals
            .iter()
            .map(|val| (val.name.clone(), val.value.value.clone()))
            .collect();
        for val in &hir_node.vals {
            consts.insert(val.name.clone(), val.value.value.clone());
        }
        let subst = |tree: &expr::Tree| substitute_tree(tree, &consts);
        // Declaration order between handlers carries no semantics and
        // no identity (identity.md): they ship sorted by their
        // activation's canonical key, the same key the hash sorts by.
        let mut handlers: Vec<_> = hir_node.handlers.iter().collect();
        handlers.sort_by_key(|handler| handler.activation.canonical_key());
        for handler in handlers {
            plan.handlers.push(Handler {
                node: slot,
                activation: handler.activation.clone(),
                guard: handler.guard.as_ref().map(&subst),
                body: handler
                    .body
                    .iter()
                    .map(|statement| match statement {
                        HirStatement::Let { name, value } => Step::Let {
                            name: name.clone(),
                            expr: subst(value),
                        },
                        HirStatement::SetVar { name, value } => Step::SetVar {
                            name: name.clone(),
                            expr: subst(value),
                        },
                        HirStatement::Emit { port, value } => Step::Emit {
                            port: port.clone(),
                            value: lower_emission(value, &subst),
                        },
                    })
                    .collect(),
            });
        }
    }
    for (_, edge) in sorted_edges(hir) {
        let endpoint = |end: &crate::compiler::hir::Endpoint| EndpointRef {
            node: end.node.as_ref().map(&slot_of),
            port: end.port.clone(),
        };
        plan.edges.push(Edge {
            from: endpoint(&edge.from),
            to: match &edge.to {
                EdgeTarget::Port(end) => Some(endpoint(end)),
                EdgeTarget::Discard => None,
            },
        });
    }
    (plan, symbols)
}

fn lower_emission(value: &EmitValue, subst: &impl Fn(&expr::Tree) -> expr::Tree) -> Emission {
    let writes = |fields: &[crate::compiler::hir::FieldWrite]| {
        fields
            .iter()
            .map(|write| FieldWrite {
                target: write.target.clone(),
                expr: subst(&write.value),
            })
            .collect()
    };
    match value {
        EmitValue::Identity => Emission::Identity,
        EmitValue::Patch(fields) => Emission::Patch(writes(fields)),
        EmitValue::FreshData(fields) => Emission::Replace(writes(fields)),
        EmitValue::Data(tree) => Emission::Data(subst(tree)),
        EmitValue::Nothing => Emission::Nothing,
    }
}

/// The canonical spelling of a resolved write target — what the
/// semantic hash seals, alias-free.
fn canonical_target(target: &crate::envelope::WriteTarget) -> String {
    match target {
        crate::envelope::WriteTarget::Data(path) if path.is_empty() => String::from("data"),
        crate::envelope::WriteTarget::Data(path) => format!("data.{}", path.join(".")),
        crate::envelope::WriteTarget::App(field) => format!("meta.app.{field}"),
        crate::envelope::WriteTarget::Ext(field) => format!("meta.ext.{field}"),
    }
}

/// Rewrites every constant reference into its literal value; member
/// access on a constant becomes evaluable indexing. Sound because
/// shadowing is refused at check time.
fn substitute_tree(
    tree: &expr::Tree,
    consts: &std::collections::BTreeMap<String, crate::value::Value>,
) -> expr::Tree {
    expr::Tree {
        root: substitute(&tree.root, consts),
        ty: tree.ty.clone(),
    }
}

fn substitute(
    expr: &expr::Expr,
    consts: &std::collections::BTreeMap<String, crate::value::Value>,
) -> expr::Expr {
    let recurse = |inner: &expr::Expr| Box::new(substitute(inner, consts));
    match expr {
        expr::Expr::Asset(_) => expr.clone(),
        expr::Expr::Path(segments) => match consts.get(&segments[0]) {
            None => expr.clone(),
            Some(value) => {
                let mut result = expr::Expr::Literal(value.clone());
                for segment in &segments[1..] {
                    result = expr::Expr::Index {
                        base: Box::new(result),
                        index: Box::new(expr::Expr::Literal(crate::value::Value::Text(
                            segment.clone(),
                        ))),
                    };
                }
                result
            }
        },
        expr::Expr::Literal(_) => expr.clone(),
        expr::Expr::Unary { op, expr } => expr::Expr::Unary {
            op: *op,
            expr: recurse(expr),
        },
        expr::Expr::Binary { op, left, right } => expr::Expr::Binary {
            op: *op,
            left: recurse(left),
            right: recurse(right),
        },
        expr::Expr::Ternary {
            cond,
            then,
            otherwise,
        } => expr::Expr::Ternary {
            cond: recurse(cond),
            then: recurse(then),
            otherwise: recurse(otherwise),
        },
        expr::Expr::Index { base, index } => expr::Expr::Index {
            base: recurse(base),
            index: recurse(index),
        },
        expr::Expr::Call { function, args } => expr::Expr::Call {
            function: *function,
            args: args.iter().map(|arg| substitute(arg, consts)).collect(),
        },
        expr::Expr::List(items) => {
            expr::Expr::List(items.iter().map(|item| substitute(item, consts)).collect())
        }
        expr::Expr::MapLit(entries) => expr::Expr::MapLit(
            entries
                .iter()
                .map(|(key, value)| (key.clone(), substitute(value, consts)))
                .collect(),
        ),
        expr::Expr::Struct(fields) => expr::Expr::Struct(
            fields
                .iter()
                .map(|(name, value)| (name.clone(), substitute(value, consts)))
                .collect(),
        ),
        expr::Expr::Tagged {
            union,
            case,
            fields,
        } => expr::Expr::Tagged {
            union: union.clone(),
            case: case.clone(),
            fields: fields
                .iter()
                .map(|(name, value)| (name.clone(), substitute(value, consts)))
                .collect(),
        },
        expr::Expr::Is { expr, target } => expr::Expr::Is {
            expr: recurse(expr),
            target: target.clone(),
        },
        expr::Expr::As { expr, target } => expr::Expr::As {
            expr: recurse(expr),
            target: target.clone(),
        },
    }
}

/// Nodes in canonical order: (kind, path), per `identity.md` §1's
/// sorted top-level declarations. Slots index this order.
fn sorted_nodes(hir: &Hir) -> Vec<&HirNode> {
    let mut nodes: Vec<_> = hir.app.nodes.iter().collect();
    nodes.sort_by_key(|node| (kind_rank(node.category()), node.path.to_string()));
    nodes
}

/// The checker guarantees the mandatory axes exist with valid words
/// before any plan is built; the plan makes them irrevocable types.
fn typed_effect(node: &HirNode) -> node::Effect {
    let word = attribute_word(node, "effect");
    match word {
        "idempotent" => node::Effect::Class(node::EffectClass::Idempotent),
        "compensable" => node::Effect::Class(node::EffectClass::Compensable),
        "irreversible" => node::Effect::Class(node::EffectClass::Irreversible),
        other => unreachable!("the contract sheet admitted `{other}`"),
    }
}

fn typed_behavior(node: &HirNode) -> node::Behavior {
    match attribute_word(node, "behavior") {
        "deterministic" => node::Behavior::Deterministic,
        "probabilistic" => node::Behavior::Probabilistic,
        other => unreachable!("the contract sheet admitted `{other}`"),
    }
}

fn attribute_word<'hir>(node: &'hir HirNode, name: &str) -> &'hir str {
    node.attributes()
        .iter()
        .find_map(|attribute| match (&attribute.name, &attribute.value) {
            (attr, AttrValue::Word(word)) if attr == name => Some(word.as_str()),
            _ => None,
        })
        .unwrap_or_else(|| unreachable!("the contract sheet requires `{name}`"))
}

fn kind_rank(kind: node::Kind) -> u8 {
    match kind {
        node::Kind::Operator => 0,
        node::Kind::Connector => 1,
        node::Kind::Graph => 2,
    }
}

fn endpoint_text(endpoint: &crate::compiler::hir::Endpoint) -> String {
    match &endpoint.node {
        Some(path) => format!("{path}.{}", endpoint.port),
        None => format!(".{}", endpoint.port),
    }
}

/// Deterministic canonical byte writer feeding blake3. Every variable
/// datum is length- or count-prefixed; every variant carries a tag —
/// the same injectivity discipline as the id framing.
struct Writer<'assets> {
    hasher: blake3::Hasher,
    /// The asset table — `Expr::Asset` hashes by content digest.
    assets: &'assets [crate::plan::AssetBlob],
}

impl Writer<'_> {
    fn tag(&mut self, tag: u8) {
        self.hasher.update(&[tag]);
    }

    fn count(&mut self, count: usize) {
        let count = u32::try_from(count).unwrap_or_else(|_| unreachable!("canonical counts fit"));
        self.u32(count);
    }

    fn str(&mut self, text: &str) {
        self.count(text.len());
        self.hasher.update(text.as_bytes());
    }

    fn i32(&mut self, value: i32) {
        self.hasher.update(&value.to_le_bytes());
    }

    fn u32(&mut self, value: u32) {
        self.hasher.update(&value.to_le_bytes());
    }

    fn direction(&mut self, direction: Direction) {
        self.tag(match direction {
            Direction::Input => 1,
            Direction::Output => 2,
        });
    }

    /// Stable domain tags for the logical data-type vocabulary — never a Rust
    /// `Debug` rendering, which renames could silently change.
    fn ty(&mut self, ty: &DataType) {
        // Preserve every pre-M1.3 hash exactly. The parameterized forms below
        // extend the HIR domain; these legacy aliases keep their historical
        // one-byte spelling until the categorical identity move in B8.
        if ty == &DataType::INT {
            self.tag(2);
            return;
        }
        if ty == &DataType::UINT {
            self.tag(3);
            return;
        }
        if ty == &DataType::DOUBLE {
            self.tag(4);
            return;
        }
        if ty == &DataType::BYTES {
            self.tag(6);
            return;
        }
        if ty == &DataType::DURATION {
            self.tag(7);
            return;
        }
        if ty == &DataType::TIMESTAMP {
            self.tag(8);
            return;
        }
        if let DataType::Array(array) = ty
            && array.length().is_none()
        {
            self.tag(11);
            self.ty(array.element());
            return;
        }
        if let DataType::Map(map) = ty
            && map.key() == &DataType::Text
        {
            self.tag(12);
            self.ty(map.value());
            return;
        }

        match ty {
            DataType::Bool => self.tag(1),
            DataType::Text => self.tag(5),
            DataType::Null => self.tag(9),
            DataType::Variant => self.tag(10),
            DataType::Integer(integer) => {
                self.tag(13);
                let (signedness, width) = match integer {
                    IntegerType::Signed(width) => (1, *width),
                    IntegerType::Unsigned(width) => (2, *width),
                };
                self.tag(signedness);
                self.tag(match width {
                    IntegerWidth::Bits8 => 8,
                    IntegerWidth::Bits16 => 16,
                    IntegerWidth::Bits32 => 32,
                    IntegerWidth::Bits64 => 64,
                });
            }
            DataType::Float(float) => {
                self.tag(14);
                self.tag(match float {
                    FloatType::Binary16 => 16,
                    FloatType::Binary32 => 32,
                    FloatType::Binary64 => 64,
                });
            }
            DataType::Decimal(decimal) => {
                self.tag(15);
                self.tag(decimal.precision());
                self.i32(i32::from(decimal.scale()));
            }
            DataType::Number => self.tag(16),
            DataType::Bytes(BinaryType::Variable) => self.tag(6),
            DataType::Bytes(BinaryType::Fixed(length)) => {
                self.tag(17);
                self.u32(length.get());
            }
            DataType::Temporal(temporal) => {
                self.tag(18);
                match temporal {
                    TemporalType::Date(unit) => {
                        self.tag(1);
                        self.tag(match unit {
                            DateUnit::Day => 1,
                            DateUnit::Millisecond => 2,
                        });
                    }
                    TemporalType::Time(unit) => {
                        self.tag(2);
                        self.time_unit(*unit);
                    }
                    TemporalType::Timestamp { unit, zone } => {
                        self.tag(3);
                        self.time_unit(*unit);
                        match zone {
                            TimeZone::Utc => self.tag(1),
                            TimeZone::Named(name) => {
                                self.tag(2);
                                self.str(name.as_str());
                            }
                        }
                    }
                    TemporalType::TimestampNtz(unit) => {
                        self.tag(4);
                        self.time_unit(*unit);
                    }
                    TemporalType::Duration(unit) => {
                        self.tag(5);
                        self.time_unit(*unit);
                    }
                    TemporalType::Interval(unit) => {
                        self.tag(6);
                        self.tag(match unit {
                            IntervalUnit::YearMonth => 1,
                            IntervalUnit::DayTime => 2,
                            IntervalUnit::MonthDayNano => 3,
                            IntervalUnit::MonthDayMillisUnsigned => 4,
                        });
                    }
                }
            }
            DataType::Uuid => self.tag(19),
            DataType::Struct(structure) => {
                self.tag(20);
                self.count(structure.fields().len());
                for field in structure.fields() {
                    self.str(field.name());
                    self.ty(field.ty());
                }
            }
            DataType::Map(map) => {
                self.tag(21);
                self.ty(map.key());
                self.ty(map.value());
            }
            DataType::Array(array) => {
                self.tag(22);
                self.ty(array.element());
                match array.length() {
                    None => self.tag(0),
                    Some(length) => {
                        self.tag(1);
                        self.u32(length.get());
                    }
                }
            }
            DataType::Enum(enumeration) => {
                self.tag(23);
                self.count(enumeration.symbols().len());
                for symbol in enumeration.symbols() {
                    self.str(symbol);
                }
            }
            DataType::StructuralUnion(union) => {
                self.tag(24);
                self.count(union.branches().len());
                for branch in union.branches() {
                    self.ty(branch);
                }
            }
            DataType::TaggedUnion(union) => {
                self.tag(25);
                self.count(union.cases().len());
                for case in union.cases() {
                    self.str(case.name());
                    self.count(case.payload().fields().len());
                    for field in case.payload().fields() {
                        self.str(field.name());
                        self.ty(field.ty());
                    }
                }
            }
        }
    }

    fn time_unit(&mut self, unit: TimeUnit) {
        self.tag(match unit {
            TimeUnit::Second => 1,
            TimeUnit::Millisecond => 2,
            TimeUnit::Microsecond => 3,
            TimeUnit::Nanosecond => 4,
        });
    }

    fn node(&mut self, node: &HirNode) {
        self.tag(match node.category() {
            node::Kind::Operator => 1,
            node::Kind::Connector => 2,
            node::Kind::Graph => 3,
        });
        self.str(&node.path.to_string());
        match &node.role {
            crate::compiler::hir::NodeRole::Operator => self.tag(0),
            crate::compiler::hir::NodeRole::Connector { trait_name, .. } => {
                self.tag(1);
                self.str(trait_name);
            }
        }
        let mut vals: Vec<_> = node.vals.iter().collect();
        vals.sort_by(|a, b| a.name.cmp(&b.name));
        self.count(vals.len());
        for val in vals {
            self.str(&val.name);
            self.tree(&val.value.tree);
        }
        let mut attributes: Vec<_> = node.attributes().iter().collect();
        attributes.sort_by(|a, b| a.name.cmp(&b.name));
        self.count(attributes.len());
        for attribute in attributes {
            self.str(&attribute.name);
            match &attribute.value {
                AttrValue::Word(word) => {
                    self.tag(1);
                    self.str(word);
                }
                AttrValue::Const(constant) => {
                    self.tag(2);
                    self.tree(&constant.tree);
                }
            }
        }
        let mut vars: Vec<_> = node.vars.iter().collect();
        vars.sort_by(|a, b| a.name.cmp(&b.name));
        self.count(vars.len());
        for var in vars {
            self.str(&var.name);
            self.ty(&var.ty);
            match &var.init {
                None => self.tag(0),
                Some(init) => {
                    self.tag(1);
                    self.tree(&init.tree);
                }
            }
        }
        let mut ports: Vec<_> = node.ports.iter().collect();
        ports.sort_by_key(|decl| (decl.port.name(), decl.direction));
        self.count(ports.len());
        for decl in ports {
            self.str(decl.port.name());
            self.tag(match (decl.direction, decl.unwired) {
                (Direction::Input, crate::plan::UnwiredPolicy::RequireWire) => 1,
                (Direction::Output, crate::plan::UnwiredPolicy::RequireWire) => 2,
                (Direction::Output, crate::plan::UnwiredPolicy::RootSink) => 3,
                (Direction::Input, crate::plan::UnwiredPolicy::RootSink) => 4,
            });
            // The pinned type, present-or-absent — same idiom as the
            // boundary and the var initializers.
            match &decl.ty {
                None => self.tag(0),
                Some(ty) => {
                    self.tag(1);
                    self.ty(ty);
                }
            }
        }
        // Sorted by activation key: the identity is insensitive to
        // declaration order between handlers (identity.md) — only
        // statements INSIDE a handler are order-sensitive.
        let mut handlers: Vec<_> = node.handlers.iter().collect();
        handlers.sort_by_key(|handler| handler.activation.canonical_key());
        self.count(handlers.len());
        for handler in handlers {
            // Hash material: the canonical key (`p:<port>`) — the
            // pinned identity form, byte-identical to the
            // pre-activation encoding; Display is presentation and
            // free to move.
            self.str(&handler.activation.canonical_key());
            match &handler.guard {
                None => self.tag(0),
                Some(guard) => {
                    self.tag(1);
                    self.tree(guard);
                }
            }
            self.count(handler.body.len());
            for statement in &handler.body {
                match statement {
                    HirStatement::Let { name, value } => {
                        self.tag(1);
                        self.str(name);
                        self.tree(value);
                    }
                    HirStatement::Emit { port, value } => {
                        self.tag(2);
                        self.str(port.name());
                        self.emission(value);
                    }
                    HirStatement::SetVar { name, value } => {
                        self.tag(3);
                        self.str(name);
                        self.tree(value);
                    }
                }
            }
        }
    }

    fn emission(&mut self, value: &EmitValue) {
        match value {
            EmitValue::Identity => self.tag(1),
            EmitValue::Patch(writes) => {
                self.tag(2);
                self.field_writes(writes);
            }
            EmitValue::FreshData(writes) => {
                self.tag(3);
                self.field_writes(writes);
            }
            EmitValue::Data(tree) => {
                self.tag(4);
                self.tree(tree);
            }
            EmitValue::Nothing => self.tag(5),
        }
    }

    fn field_writes(&mut self, writes: &[crate::compiler::hir::FieldWrite]) {
        self.count(writes.len());
        for write in writes {
            // The RESOLVED target is identity material — identity.md
            // makes the hash sugar-spelling-insensitive, so
            // `meta.flag` and `meta.app.flag` seal identically.
            self.str(&canonical_target(&write.target));
            self.tree(&write.value);
        }
    }

    /// A compiled tree seals its STATIC TYPE with its expression —
    /// `json! [1]` (variant) and `[1]` (list) are different programs
    /// (D-2026-08-10-7), exactly as a pinned port differs from an
    /// unpinned one.
    fn tree(&mut self, tree: &expr::Tree) {
        self.ty(&tree.ty);
        self.expr(&tree.root);
    }

    fn expr(&mut self, expr: &expr::Expr) {
        match expr {
            expr::Expr::Literal(value) => {
                self.tag(1);
                self.value(value);
            }
            expr::Expr::Asset(index) => {
                // The CONTENT digest is the identity — the path is
                // spelling and the index is layout (D-2026-08-10-5).
                self.tag(9);
                let digest = self
                    .assets
                    .get(*index as usize)
                    .map(|blob| blob.digest)
                    .unwrap_or_else(|| unreachable!("the checker minted the index"));
                self.hasher.update(&digest);
            }
            expr::Expr::Path(segments) => {
                self.tag(2);
                self.str(&segments.join("."));
            }
            expr::Expr::Unary { op, expr } => {
                self.tag(3);
                self.tag(match op {
                    expr::UnaryOp::Not => 1,
                    expr::UnaryOp::Neg => 2,
                });
                self.expr(expr);
            }
            expr::Expr::Binary { op, left, right } => {
                self.tag(4);
                self.tag(match op {
                    expr::BinaryOp::Mul => 1,
                    expr::BinaryOp::Div => 2,
                    expr::BinaryOp::Rem => 3,
                    expr::BinaryOp::Add => 4,
                    expr::BinaryOp::Sub => 5,
                    expr::BinaryOp::Eq => 6,
                    expr::BinaryOp::Ne => 7,
                    expr::BinaryOp::Lt => 8,
                    expr::BinaryOp::Le => 9,
                    expr::BinaryOp::Gt => 10,
                    expr::BinaryOp::Ge => 11,
                    expr::BinaryOp::And => 12,
                    expr::BinaryOp::Or => 13,
                });
                self.expr(left);
                self.expr(right);
            }
            expr::Expr::Ternary {
                cond,
                then,
                otherwise,
            } => {
                self.tag(5);
                self.expr(cond);
                self.expr(then);
                self.expr(otherwise);
            }
            expr::Expr::Index { base, index } => {
                self.tag(6);
                self.expr(base);
                self.expr(index);
            }
            expr::Expr::Call { function, args } => {
                self.tag(7);
                self.tag(match function {
                    expr::Function::Asset => 5,
                    expr::Function::Duration => 1,
                    expr::Function::Timestamp => 2,
                    expr::Function::Size => 3,
                    expr::Function::Decimal => 4,
                });
                self.count(args.len());
                for arg in args {
                    self.expr(arg);
                }
            }
            expr::Expr::List(items) => {
                self.tag(8);
                self.count(items.len());
                for item in items {
                    self.expr(item);
                }
            }
            expr::Expr::MapLit(entries) => {
                self.tag(9);
                // Maps are semantically unordered; the canonical form
                // sorts keys (duplicates are rejected at compilation).
                let mut sorted: Vec<_> = entries.iter().collect();
                sorted.sort_by(|a, b| a.0.cmp(&b.0));
                self.count(sorted.len());
                for (key, value) in sorted {
                    self.str(key);
                    self.expr(value);
                }
            }
            expr::Expr::Struct(fields) => {
                self.tag(12);
                let mut sorted: Vec<_> = fields.iter().collect();
                sorted.sort_by(|a, b| a.0.cmp(&b.0));
                self.count(sorted.len());
                for (name, value) in sorted {
                    self.str(name);
                    self.expr(value);
                }
            }
            expr::Expr::Tagged {
                union,
                case,
                fields,
            } => {
                self.tag(13);
                self.ty(&DataType::TaggedUnion((**union).clone()));
                self.str(case);
                let mut sorted: Vec<_> = fields.iter().collect();
                sorted.sort_by(|a, b| a.0.cmp(&b.0));
                self.count(sorted.len());
                for (name, value) in sorted {
                    self.str(name);
                    self.expr(value);
                }
            }
            expr::Expr::Is { expr, target } => {
                self.tag(10);
                self.expr(expr);
                self.narrow_target(target);
            }
            expr::Expr::As { expr, target } => {
                self.tag(11);
                self.expr(expr);
                self.narrow_target(target);
            }
        }
    }

    fn narrow_target(&mut self, target: &expr::NarrowTarget) {
        match target {
            expr::NarrowTarget::Type(target) => {
                self.tag(1);
                self.ty(target.ty());
                match target.discriminator() {
                    None => self.tag(0),
                    Some(discriminator) => {
                        self.tag(1);
                        self.str(discriminator.field());
                        self.count(discriminator.mapping().len());
                        for (external, case) in discriminator.mapping() {
                            self.str(external);
                            self.str(case);
                        }
                    }
                }
            }
            expr::NarrowTarget::Case { name, payload } => {
                self.tag(2);
                self.str(name);
                self.ty(payload);
            }
        }
    }

    fn value(&mut self, value: &crate::value::Value) {
        match value {
            crate::value::Value::Decimal { unscaled, scale } => {
                // Tag 13 in the HASH register: the canonical pair —
                // reached by `json` literals (D-2026-08-10-7).
                self.tag(13);
                self.hasher.update(&unscaled.to_be_bytes());
                self.hasher.update(&scale.to_be_bytes());
            }
            crate::value::Value::Bool(b) => {
                self.tag(1);
                self.tag(u8::from(*b));
            }
            crate::value::Value::Int(i) => {
                self.tag(2);
                self.hasher.update(&i.to_le_bytes());
            }
            crate::value::Value::Uint(u) => {
                self.tag(3);
                self.hasher.update(&u.to_le_bytes());
            }
            crate::value::Value::Double(d) => {
                self.tag(4);
                self.hasher.update(&d.to_bits().to_le_bytes());
            }
            crate::value::Value::Text(text) => {
                self.tag(5);
                self.str(text);
            }
            crate::value::Value::Bytes(bytes) => {
                self.tag(6);
                self.count(bytes.len());
                self.hasher.update(bytes);
            }
            crate::value::Value::Duration(nanos) => {
                self.tag(7);
                self.hasher.update(&nanos.to_le_bytes());
            }
            crate::value::Value::Timestamp(nanos) => {
                self.tag(8);
                self.hasher.update(&nanos.to_le_bytes());
            }
            crate::value::Value::Date(days) => {
                self.tag(17);
                self.hasher.update(&days.to_le_bytes());
            }
            crate::value::Value::Time(nanos) => {
                self.tag(18);
                self.hasher.update(&nanos.to_le_bytes());
            }
            crate::value::Value::TimestampNtz(nanos) => {
                self.tag(19);
                self.hasher.update(&nanos.to_le_bytes());
            }
            crate::value::Value::Interval {
                months,
                days,
                nanoseconds,
            } => {
                self.tag(20);
                self.hasher.update(&months.to_le_bytes());
                self.hasher.update(&days.to_le_bytes());
                self.hasher.update(&nanoseconds.to_le_bytes());
            }
            crate::value::Value::Uuid(bytes) => {
                self.tag(21);
                self.hasher.update(bytes);
            }
            crate::value::Value::Null => self.tag(9),
            crate::value::Value::Array(items) => {
                self.tag(10);
                self.count(items.len());
                for item in items {
                    self.value(item);
                }
            }
            crate::value::Value::Map(map) => {
                self.tag(11);
                self.count(map.len());
                for (key, item) in map {
                    self.str(key);
                    self.value(item);
                }
            }
            crate::value::Value::KeyedMap(entries) => {
                self.tag(22);
                let mut sorted: Vec<_> = entries.iter().collect();
                sorted.sort_by_cached_key(|(key, _)| {
                    crate::canonical_cbor::encode(&crate::identity::encoding::value_tagged(key))
                });
                self.count(sorted.len());
                for (key, item) in sorted {
                    self.value(key);
                    self.value(item);
                }
            }
            crate::value::Value::Struct(fields) => {
                // A literal or reduced host constructor reaches this form.
                // Structs are semantically unordered, so identity uses the
                // same canonical field-name order as the value registry.
                self.tag(12);
                let mut sorted: Vec<_> = fields.iter().collect();
                sorted.sort_by(|a, b| a.0.cmp(&b.0));
                self.count(sorted.len());
                for (name, item) in sorted {
                    self.str(name);
                    self.value(item);
                }
            }
            crate::value::Value::Enum(symbol) => {
                self.tag(14);
                self.str(symbol);
            }
            crate::value::Value::StructuralUnion { branch, value } => {
                self.tag(15);
                self.hasher.update(&branch.to_le_bytes());
                self.value(value);
            }
            crate::value::Value::TaggedUnion(tagged) => {
                self.tag(16);
                self.str(&tagged.case);
                let mut sorted: Vec<_> = tagged.payload.iter().collect();
                sorted.sort_by(|a, b| a.0.cmp(&b.0));
                self.count(sorted.len());
                for (name, item) in sorted {
                    self.str(name);
                    self.value(item);
                }
            }
        }
    }
}

/// Adds the one runtime-produced Operator port that has no source emission.
/// Connector rows already come resolved from their contract without a second
/// lowering interpretation.
fn executable_ports(node: &HirNode) -> Vec<crate::plan::PortDecl> {
    let mut ports = node.ports.clone();
    match &node.role {
        NodeRole::Operator => {
            if let Some(err) = ports.iter_mut().find(|port| port.port.name() == "err") {
                err.unwired = crate::plan::UnwiredPolicy::RootSink;
                if err.ty.is_none() {
                    err.ty = Some(DataType::text_map(DataType::Text));
                }
            } else if let Ok(err) = node::Port::new("err") {
                ports.push(crate::plan::PortDecl::root_sink(
                    err,
                    Some(DataType::text_map(DataType::Text)),
                ));
            }
        }
        NodeRole::Connector { .. } => {}
    }
    ports
}

#[cfg(test)]
mod tests {
    use super::DataType;
    use crate::compiler::compile;
    use crate::contract::Registry;
    use crate::node;
    use crate::plan::{AttrSetting, Emission, GraphVersion, Manifest, Step};

    const M0_PROBE: &str = r#"
graph m0_probe : App {
  connector echo : Process {
    mode = job ; command = "cat"
    codec = lines
    effect = idempotent ; behavior = deterministic
  }

  operator shout {
    on in where data.text != "" {
      out = data.text
    }
  }

  . -> shout -> echo -> .
}
"#;

    #[test]
    fn the_m0_probe_compiles_to_a_plan() {
        let compilation = compile(
            &[M0_PROBE],
            &Registry::try_m0().expect("the standard catalogue holds"),
        )
        .expect("a valid plan");
        assert!(compilation.warnings.is_empty());
        let plan = &compilation.plan;
        assert_eq!(plan.nodes.len(), 2);
        // Canonical order: operators before connectors.
        assert_eq!(plan.nodes[0].path.to_string(), "shout");
        assert_eq!(plan.nodes[1].path.to_string(), "echo");
        let spec = plan.nodes[1].connector().expect("a connector spec");
        assert_eq!(spec.contract, "Process");
        assert_eq!(
            spec.attribute("command"),
            Some(&AttrSetting::Value(crate::value::Value::Text(
                String::from("cat")
            )))
        );
        assert_eq!(plan.handlers.len(), 1);
        let handler = &plan.handlers[0];
        assert_eq!(handler.node.index(), 0);
        assert_eq!(
            handler.activation,
            node::HandlerActivation::Port(node::Port::new("in").unwrap())
        );
        assert!(matches!(
            &handler.body[0],
            Step::Emit {
                value: Emission::Data(_),
                ..
            }
        ));
        assert_eq!(plan.edges.len(), 3);
        assert_eq!(compilation.symbols.nodes, ["shout", "echo"]);
        let resolved = plan.resolved_ports();
        let boundary_in = resolved
            .boundary()
            .iter()
            .find(|port| port.port().name() == "in")
            .expect("the used boundary input resolves");
        assert!(matches!(boundary_in.ty(), DataType::Struct(_)));
        assert_eq!(boundary_in.origin(), crate::plan::TypeOrigin::Inferred);
        assert_eq!(
            resolved
                .node(crate::plan::Slot::new(0))
                .expect("the operator row")
                .iter()
                .find(|port| port.port().name() == "out")
                .expect("the operator output")
                .ty(),
            &DataType::Text
        );
    }

    #[test]
    fn the_mandatory_axes_ship_typed() {
        let compilation = compile(
            &[M0_PROBE],
            &Registry::try_m0().expect("the standard catalogue holds"),
        )
        .unwrap();
        let spec = compilation.plan.nodes[1].connector().unwrap();
        assert_eq!(
            spec.effect,
            crate::node::Effect::Class(crate::node::EffectClass::Idempotent)
        );
        assert_eq!(spec.behavior, crate::node::Behavior::Deterministic);
        // The typed axes are not repeated in the attribute bag.
        assert!(spec.attribute("effect").is_none());
        assert!(spec.attribute("behavior").is_none());
    }

    #[test]
    fn the_hash_ignores_formatting_and_declaration_order() {
        let original = compile(
            &[M0_PROBE],
            &Registry::try_m0().expect("the standard catalogue holds"),
        )
        .unwrap()
        .hash;
        let same = compile(
            &[M0_PROBE],
            &Registry::try_m0().expect("the standard catalogue holds"),
        )
        .unwrap()
        .hash;
        assert_eq!(original, same);
        // Same program: comments added, attributes spaced differently,
        // operator declared before the connector.
        let reordered = r#"
// a comment that must not enter the identity
graph m0_probe : App {
  operator shout {
    on in where data.text != "" {
      out = data.text
    }
  }

  connector echo : Process {
    mode  =  job
    command = "cat"
    codec = lines
    effect = idempotent
    behavior = deterministic
  }

  . -> shout -> echo -> .
}
"#;
        assert_eq!(
            compile(
                &[reordered],
                &Registry::try_m0().expect("the standard catalogue holds")
            )
            .unwrap()
            .hash,
            original
        );
    }

    #[test]
    fn the_hash_is_sensitive_to_meaning() {
        let original = compile(
            &[M0_PROBE],
            &Registry::try_m0().expect("the standard catalogue holds"),
        )
        .unwrap()
        .hash;
        let renamed = M0_PROBE.replace("shout", "whisper");
        assert_ne!(
            compile(
                &[renamed.as_str()],
                &Registry::try_m0().expect("the standard catalogue holds")
            )
            .unwrap()
            .hash,
            original,
            "renaming a node is an identity change - no alpha-equivalence"
        );
        let regexed = M0_PROBE.replace("data.text != \"\"", "data.text != \"x\"");
        assert_ne!(
            compile(
                &[regexed.as_str()],
                &Registry::try_m0().expect("the standard catalogue holds")
            )
            .unwrap()
            .hash,
            original,
            "a Rastok expression is identity material"
        );
    }

    #[test]
    fn the_semantic_hash_is_pinned_byte_for_byte() {
        // The frozen identity of the M0 probe. If this test fails, the
        // canonical form changed: that is an identity-contract change
        // and must be deliberate, reviewed, and re-blessed.
        // DELIBERATE MOVE 2026-08-10 (D-2026-08-10-7): a compiled
        // tree seals its STATIC TYPE with its root, so a `json`
        // document and its Rastok expression twin are distinct programs — every
        // tree-carrying graph moved once.
        // DELIBERATE MOVE 2026-08-12 (D-2026-08-12-1): contract
        // defaults are injected into the effective HIR before the
        // hash (`args = []`, `max_output = "8MiB"`,
        // `on_timeout = kill` on this probe) — omission and the
        // exact spelling of a default are ONE identity, so every
        // Process-carrying graph moved once.
        // DELIBERATE MOVE 2026-08-12, same day (review round): the
        // former Process error_set gained `CaptureFailed` (a pipe READ
        // fault is its own kind).
        // DELIBERATE FIXTURE MIGRATION 2026-08-14 (M1.3 B2): the
        // probe now emits `data.text`, not a one-field record, because
        // the Process `lines` input is text. The identity recipe did
        // not move; this previously ill-typed program did.
        // DELIBERATE MOVE 2026-08-15 (M1.3 B5): complete resolved
        // Connector port rows and their routing policies replace the
        // travelling error_set. The Process sheet also materializes
        // `err_codec = lines` before hashing.
        let hash = compile(
            &[M0_PROBE],
            &Registry::try_m0().expect("the standard catalogue holds"),
        )
        .unwrap()
        .hash;
        assert_eq!(
            hash.to_string(),
            "f068bcd1d7b2f0756fceb59b160e8e033bbb7791f39840c4456643fadca97a1a"
        );
    }

    #[test]
    fn boundary_ports_are_identity_material() {
        let bare = "graph g : App {\n  operator o { on in where data.anchor == \"\" { out = . } }\n  . -> o -> .\n}\n";
        let with_port = "graph g : App {\n  input feed: text\n  operator o { on in where data.anchor == \"\" { out = . } }\n  . -> o -> .\n}\n";
        let bare_hash = compile(
            &[bare],
            &Registry::try_m0().expect("the standard catalogue holds"),
        )
        .unwrap()
        .hash;
        let ported = compile(
            &[with_port],
            &Registry::try_m0().expect("the standard catalogue holds"),
        )
        .unwrap();
        assert_ne!(
            ported.hash, bare_hash,
            "a declared boundary port is identity"
        );
        assert_eq!(ported.plan.boundary.len(), 3);
        assert!(
            ported
                .plan
                .boundary
                .iter()
                .any(|port| port.port.name() == "feed")
        );
    }

    #[test]
    fn the_plan_and_the_hash_share_one_edge_order() {
        let forward = "graph g : App {\n  operator a { on in where data.anchor == \"\" { out = . } }\n  operator b { on in { out = . } }\n  . -> a -> b -> .\n}\n";
        let reordered = "graph g : App {\n  operator a { on in where data.anchor == \"\" { out = . } }\n  operator b { on in { out = . } }\n  b -> .\n  a -> b\n  . -> a\n}\n";
        let one = compile(
            &[forward],
            &Registry::try_m0().expect("the standard catalogue holds"),
        )
        .unwrap();
        let two = compile(
            &[reordered],
            &Registry::try_m0().expect("the standard catalogue holds"),
        )
        .unwrap();
        assert_eq!(one.hash, two.hash);
        assert_eq!(one.plan.edges, two.plan.edges);
    }

    #[test]
    fn unordered_collections_ship_canonically() {
        let with_attrs = |attrs: &str| {
            format!(
                "graph g : App {{\n  connector c : Process {{\n{attrs}  }}\n  . -> c -> .\n}}\n"
            )
        };
        let one = compile(
            &[with_attrs("    command = \"cat\" ; codec = lines\n    effect = idempotent ; behavior = deterministic\n").as_str()],
            &Registry::try_m0().expect("the standard catalogue holds"),
        )
        .unwrap();
        let two = compile(
            &[with_attrs("    behavior = deterministic ; effect = idempotent\n    codec = lines ; command = \"cat\"\n").as_str()],
            &Registry::try_m0().expect("the standard catalogue holds"),
        )
        .unwrap();
        assert_eq!(one.hash, two.hash);
        assert_eq!(
            one.plan, two.plan,
            "attribute order must not leak into the plan"
        );
        // Map literals: one canonical form for hash, plan, and eval.
        let with_map = |map: &str| {
            format!(
                "graph g : App {{\n  operator o {{\n    on in where data.anchor == \"\" && size({map}) > 0 {{ out = . }}\n  }}\n  . -> o -> .\n}}\n"
            )
        };
        let forward = compile(
            &[with_map("{'a': 1, 'b': 2}").as_str()],
            &Registry::try_m0().expect("the standard catalogue holds"),
        )
        .unwrap();
        let reversed = compile(
            &[with_map("{'b': 2, 'a': 1}").as_str()],
            &Registry::try_m0().expect("the standard catalogue holds"),
        )
        .unwrap();
        assert_eq!(forward.hash, reversed.hash);
        assert_eq!(forward.plan, reversed.plan);
    }

    #[test]
    fn inferred_ports_travel_in_the_plan() {
        let source = "graph g : App {\n  input feed: text\n  output sink\n  operator o {\n    on event { result = . }\n  }\n  .feed -> o.event\n  o.result -> .sink\n}\n";
        let compilation = compile(
            &[source],
            &Registry::try_m0().expect("the standard catalogue holds"),
        )
        .unwrap();
        let ports = &compilation.plan.nodes[0].ports;
        assert_eq!(ports.len(), 3, "the VM-produced err row travels too");
        assert!(ports.iter().any(|decl| decl.port.name() == "event"));
        assert!(ports.iter().any(|decl| decl.port.name() == "result"));
    }

    #[test]
    fn constants_reach_both_the_hash_and_the_plan() {
        let with_val = |n: &str| {
            format!(
                "graph g : App {{\n  operator o {{\n    val threshold = {n}\n    on in where data.n > threshold {{ out = . }}\n  }}\n  . -> o -> .\n}}\n"
            )
        };
        let five = compile(
            &[with_val("5").as_str()],
            &Registry::try_m0().expect("the standard catalogue holds"),
        )
        .unwrap();
        let six = compile(
            &[with_val("6").as_str()],
            &Registry::try_m0().expect("the standard catalogue holds"),
        )
        .unwrap();
        // A local constant is identity material...
        assert_ne!(five.hash, six.hash);
        // ...and the shipped guard carries its value: no free
        // `threshold` binding survives in the plan.
        let guard = five.plan.handlers[0].guard.as_ref().unwrap();
        let rendered = format!("{guard:?}");
        assert!(
            !rendered.contains("threshold"),
            "the constant must be substituted: {rendered}"
        );
        assert!(rendered.contains("Int(5)"));
    }

    #[test]
    fn a_plan_with_errors_is_unrepresentable() {
        let bag = compile(
            &["graph g : App {\n  connector c : Process { }\n  . -> c -> .\n}\n"],
            &Registry::try_m0().expect("the standard catalogue holds"),
        )
        .expect_err("missing mandatory attributes must refuse");
        assert!(bag.has_errors());
    }

    #[test]
    fn versions_alias_the_hash_through_the_manifest() {
        let hash = compile(
            &[M0_PROBE],
            &Registry::try_m0().expect("the standard catalogue holds"),
        )
        .unwrap()
        .hash;
        let mut manifest = Manifest::embedded();
        assert_eq!(manifest.allocate(hash), Some(GraphVersion(1)));
        assert_eq!(manifest.hash_of(GraphVersion(1)), Some(hash));
    }
}
