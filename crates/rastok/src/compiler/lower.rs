//! Canonicalization and lowering: HIR to `SemanticHash` and
//! `ExecutionPlan`.
//!
//! The canonical byte form follows `identity.md` §1: insensitive to
//! formatting, file order, and declaration order — nodes, vals,
//! attributes, vars, ports, and edges are sorted before hashing —
//! sensitive to topology, statement order inside a handler, every CEL
//! expression as its canonical tree, and `var` defaults. Constants
//! hash as the expressions they were written as, before const-eval;
//! spans never enter the bytes. The plan then ships the reduced
//! values.

use crate::cel;
use crate::compiler::hir::{
    AttrValue, Direction, EdgeTarget, EmitValue, Hir, Node as HirNode, NodeRole,
    Statement as HirStatement,
};
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
        let mut ports = hir_node.ports.clone();
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
                NodeRole::Connector {
                    trait_name,
                    error_set,
                    ..
                } => {
                    // The error set is a SET: it ships sorted, like
                    // the hash already reads it — the registry's
                    // declaration order is not identity material.
                    let mut error_set = error_set.clone();
                    error_set.sort();
                    NodeKind::Connector(ConnectorSpec {
                        contract: trait_name.clone(),
                        effect: typed_effect(hir_node),
                        behavior: typed_behavior(hir_node),
                        attributes,
                        error_set,
                    })
                }
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
        let mut consts: std::collections::BTreeMap<String, cel::Value> = hir
            .app
            .vals
            .iter()
            .map(|val| (val.name.clone(), val.value.value.clone()))
            .collect();
        for val in &hir_node.vals {
            consts.insert(val.name.clone(), val.value.value.clone());
        }
        let subst = |tree: &cel::Tree| substitute_tree(tree, &consts);
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

fn lower_emission(value: &EmitValue, subst: &impl Fn(&cel::Tree) -> cel::Tree) -> Emission {
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
        EmitValue::Replace(fields) => Emission::Replace(writes(fields)),
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
        crate::envelope::WriteTarget::AppMeta(field) => format!("meta.app.{field}"),
        crate::envelope::WriteTarget::CustomMeta(namespace, field) => {
            format!("meta.{namespace}.{field}")
        }
        crate::envelope::WriteTarget::MimeType => String::from("mime_type"),
    }
}

/// Rewrites every constant reference into its literal value; member
/// access on a constant becomes evaluable indexing. Sound because
/// shadowing is refused at check time.
fn substitute_tree(
    tree: &cel::Tree,
    consts: &std::collections::BTreeMap<String, cel::Value>,
) -> cel::Tree {
    cel::Tree {
        root: substitute(&tree.root, consts),
        ty: tree.ty.clone(),
    }
}

fn substitute(
    expr: &cel::Expr,
    consts: &std::collections::BTreeMap<String, cel::Value>,
) -> cel::Expr {
    let recurse = |inner: &cel::Expr| Box::new(substitute(inner, consts));
    match expr {
        cel::Expr::Path(segments) => match consts.get(&segments[0]) {
            None => expr.clone(),
            Some(value) => {
                let mut result = cel::Expr::Literal(value.clone());
                for segment in &segments[1..] {
                    result = cel::Expr::Index {
                        base: Box::new(result),
                        index: Box::new(cel::Expr::Literal(cel::Value::Text(segment.clone()))),
                    };
                }
                result
            }
        },
        cel::Expr::Literal(_) => expr.clone(),
        cel::Expr::Unary { op, expr } => cel::Expr::Unary {
            op: *op,
            expr: recurse(expr),
        },
        cel::Expr::Binary { op, left, right } => cel::Expr::Binary {
            op: *op,
            left: recurse(left),
            right: recurse(right),
        },
        cel::Expr::Ternary {
            cond,
            then,
            otherwise,
        } => cel::Expr::Ternary {
            cond: recurse(cond),
            then: recurse(then),
            otherwise: recurse(otherwise),
        },
        cel::Expr::Index { base, index } => cel::Expr::Index {
            base: recurse(base),
            index: recurse(index),
        },
        cel::Expr::Call { function, args } => cel::Expr::Call {
            function: *function,
            args: args.iter().map(|arg| substitute(arg, consts)).collect(),
        },
        cel::Expr::List(items) => {
            cel::Expr::List(items.iter().map(|item| substitute(item, consts)).collect())
        }
        cel::Expr::MapLit(entries) => cel::Expr::MapLit(
            entries
                .iter()
                .map(|(key, value)| (key.clone(), substitute(value, consts)))
                .collect(),
        ),
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
struct Writer {
    hasher: blake3::Hasher,
}

impl Writer {
    fn tag(&mut self, tag: u8) {
        self.hasher.update(&[tag]);
    }

    fn count(&mut self, count: usize) {
        let count = u32::try_from(count).unwrap_or_else(|_| unreachable!("canonical counts fit"));
        self.hasher.update(&count.to_le_bytes());
    }

    fn str(&mut self, text: &str) {
        self.count(text.len());
        self.hasher.update(text.as_bytes());
    }

    fn direction(&mut self, direction: Direction) {
        self.tag(match direction {
            Direction::Input => 1,
            Direction::Output => 2,
        });
    }

    /// Stable domain tags for the CEL type vocabulary — never a Rust
    /// `Debug` rendering, which renames could silently change.
    fn ty(&mut self, ty: &cel::Type) {
        match ty {
            cel::Type::Bool => self.tag(1),
            cel::Type::Int => self.tag(2),
            cel::Type::Uint => self.tag(3),
            cel::Type::Double => self.tag(4),
            cel::Type::Text => self.tag(5),
            cel::Type::Bytes => self.tag(6),
            cel::Type::Duration => self.tag(7),
            cel::Type::Timestamp => self.tag(8),
            cel::Type::Null => self.tag(9),
            cel::Type::Dyn => self.tag(10),
            cel::Type::List(element) => {
                self.tag(11);
                self.ty(element);
            }
            cel::Type::Map(values) => {
                self.tag(12);
                self.ty(values);
            }
        }
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
            crate::compiler::hir::NodeRole::Connector {
                trait_name,
                error_set,
                ..
            } => {
                self.tag(1);
                self.str(trait_name);
                // The resolved contract is identity material: two
                // sheet revisions with different error sets must not
                // share a hash.
                let mut kinds: Vec<&String> = error_set.iter().collect();
                kinds.sort();
                self.count(kinds.len());
                for kind in kinds {
                    self.str(kind);
                }
            }
        }
        let mut vals: Vec<_> = node.vals.iter().collect();
        vals.sort_by(|a, b| a.name.cmp(&b.name));
        self.count(vals.len());
        for val in vals {
            self.str(&val.name);
            self.expr(&val.value.tree.root);
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
                    self.expr(&constant.tree.root);
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
                    self.expr(&init.tree.root);
                }
            }
        }
        let mut ports: Vec<_> = node.ports.iter().collect();
        ports.sort_by_key(|decl| (decl.port.name(), decl.direction));
        self.count(ports.len());
        for decl in ports {
            self.str(decl.port.name());
            self.tag(match decl.direction {
                Direction::Input => 1,
                Direction::Output => 2,
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
                    self.expr(&guard.root);
                }
            }
            self.count(handler.body.len());
            for statement in &handler.body {
                match statement {
                    HirStatement::Let { name, value } => {
                        self.tag(1);
                        self.str(name);
                        self.expr(&value.root);
                    }
                    HirStatement::Emit { port, value } => {
                        self.tag(2);
                        self.str(port.name());
                        self.emission(value);
                    }
                    HirStatement::SetVar { name, value } => {
                        self.tag(3);
                        self.str(name);
                        self.expr(&value.root);
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
            EmitValue::Replace(writes) => {
                self.tag(3);
                self.field_writes(writes);
            }
            EmitValue::Data(tree) => {
                self.tag(4);
                self.expr(&tree.root);
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
            self.expr(&write.value.root);
        }
    }

    fn expr(&mut self, expr: &cel::Expr) {
        match expr {
            cel::Expr::Literal(value) => {
                self.tag(1);
                self.value(value);
            }
            cel::Expr::Path(segments) => {
                self.tag(2);
                self.str(&segments.join("."));
            }
            cel::Expr::Unary { op, expr } => {
                self.tag(3);
                self.tag(match op {
                    cel::UnaryOp::Not => 1,
                    cel::UnaryOp::Neg => 2,
                });
                self.expr(expr);
            }
            cel::Expr::Binary { op, left, right } => {
                self.tag(4);
                self.tag(match op {
                    cel::BinaryOp::Mul => 1,
                    cel::BinaryOp::Div => 2,
                    cel::BinaryOp::Rem => 3,
                    cel::BinaryOp::Add => 4,
                    cel::BinaryOp::Sub => 5,
                    cel::BinaryOp::Eq => 6,
                    cel::BinaryOp::Ne => 7,
                    cel::BinaryOp::Lt => 8,
                    cel::BinaryOp::Le => 9,
                    cel::BinaryOp::Gt => 10,
                    cel::BinaryOp::Ge => 11,
                    cel::BinaryOp::And => 12,
                    cel::BinaryOp::Or => 13,
                });
                self.expr(left);
                self.expr(right);
            }
            cel::Expr::Ternary {
                cond,
                then,
                otherwise,
            } => {
                self.tag(5);
                self.expr(cond);
                self.expr(then);
                self.expr(otherwise);
            }
            cel::Expr::Index { base, index } => {
                self.tag(6);
                self.expr(base);
                self.expr(index);
            }
            cel::Expr::Call { function, args } => {
                self.tag(7);
                self.tag(match function {
                    cel::Function::Duration => 1,
                    cel::Function::Timestamp => 2,
                    cel::Function::Size => 3,
                });
                self.count(args.len());
                for arg in args {
                    self.expr(arg);
                }
            }
            cel::Expr::List(items) => {
                self.tag(8);
                self.count(items.len());
                for item in items {
                    self.expr(item);
                }
            }
            cel::Expr::MapLit(entries) => {
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
        }
    }

    fn value(&mut self, value: &cel::Value) {
        match value {
            cel::Value::Bool(b) => {
                self.tag(1);
                self.tag(u8::from(*b));
            }
            cel::Value::Int(i) => {
                self.tag(2);
                self.hasher.update(&i.to_le_bytes());
            }
            cel::Value::Uint(u) => {
                self.tag(3);
                self.hasher.update(&u.to_le_bytes());
            }
            cel::Value::Double(d) => {
                self.tag(4);
                self.hasher.update(&d.to_bits().to_le_bytes());
            }
            cel::Value::Text(text) => {
                self.tag(5);
                self.str(text);
            }
            cel::Value::Bytes(bytes) => {
                self.tag(6);
                self.count(bytes.len());
                self.hasher.update(bytes);
            }
            cel::Value::Duration(nanos) => {
                self.tag(7);
                self.hasher.update(&nanos.to_le_bytes());
            }
            cel::Value::Timestamp(nanos) => {
                self.tag(8);
                self.hasher.update(&nanos.to_le_bytes());
            }
            cel::Value::Null => self.tag(9),
            cel::Value::List(items) => {
                self.tag(10);
                self.count(items.len());
                for item in items {
                    self.value(item);
                }
            }
            cel::Value::Map(map) => {
                self.tag(11);
                self.count(map.len());
                for (key, item) in map {
                    self.str(key);
                    self.value(item);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::compiler::compile;
    use crate::plan::{AttrSetting, Emission, GraphVersion, Manifest, Step};
    use crate::stdlib::Registry;
    use crate::{cel, node};

    const M0_PROBE: &str = r#"
graph m0_probe : App {
  connector echo : Process {
    mode = job ; command = "cat"
    codec = lines
    effect = idempotent ; behavior = deterministic
  }

  operator shout {
    on in where data.text != "" {
      out = envelope { data.text = data.text }
    }
  }

  . -> shout -> echo -> .
}
"#;

    #[test]
    fn the_m0_probe_compiles_to_a_plan() {
        let compilation = compile(&[M0_PROBE], &Registry::m0()).expect("a valid plan");
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
            Some(&AttrSetting::Value(cel::Value::Text(String::from("cat"))))
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
                value: Emission::Patch(_),
                ..
            }
        ));
        assert_eq!(plan.edges.len(), 3);
        assert_eq!(compilation.symbols.nodes, ["shout", "echo"]);
    }

    #[test]
    fn the_mandatory_axes_ship_typed() {
        let compilation = compile(&[M0_PROBE], &Registry::m0()).unwrap();
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
        let original = compile(&[M0_PROBE], &Registry::m0()).unwrap().hash;
        let same = compile(&[M0_PROBE], &Registry::m0()).unwrap().hash;
        assert_eq!(original, same);
        // Same program: comments added, attributes spaced differently,
        // operator declared before the connector.
        let reordered = r#"
// a comment that must not enter the identity
graph m0_probe : App {
  operator shout {
    on in where data.text != "" {
      out = envelope { data.text = data.text }
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
            compile(&[reordered], &Registry::m0()).unwrap().hash,
            original
        );
    }

    #[test]
    fn the_hash_is_sensitive_to_meaning() {
        let original = compile(&[M0_PROBE], &Registry::m0()).unwrap().hash;
        let renamed = M0_PROBE.replace("shout", "whisper");
        assert_ne!(
            compile(&[renamed.as_str()], &Registry::m0()).unwrap().hash,
            original,
            "renaming a node is an identity change - no alpha-equivalence"
        );
        let regexed = M0_PROBE.replace("data.text != \"\"", "data.text != \"x\"");
        assert_ne!(
            compile(&[regexed.as_str()], &Registry::m0()).unwrap().hash,
            original,
            "a CEL expression is identity material"
        );
    }

    #[test]
    fn the_semantic_hash_is_pinned_byte_for_byte() {
        // The frozen identity of the M0 probe. If this test fails, the
        // canonical form changed: that is an identity-contract change
        // and must be deliberate, reviewed, and re-blessed.
        let hash = compile(&[M0_PROBE], &Registry::m0()).unwrap().hash;
        assert_eq!(
            hash.to_string(),
            "60d911720258efd717e66c86b87e55aba8ff94f97ca9bc90f078a1587bd9cee3"
        );
    }

    #[test]
    fn boundary_ports_are_identity_material() {
        let bare = "graph g : App {\n  operator o { on in { out = envelope } }\n  . -> o -> .\n}\n";
        let with_port = "graph g : App {\n  input feed\n  operator o { on in { out = envelope } }\n  . -> o -> .\n}\n";
        let bare_hash = compile(&[bare], &Registry::m0()).unwrap().hash;
        let ported = compile(&[with_port], &Registry::m0()).unwrap();
        assert_ne!(
            ported.hash, bare_hash,
            "a declared boundary port is identity"
        );
        assert_eq!(ported.plan.boundary.len(), 1);
        assert_eq!(ported.plan.boundary[0].port.name(), "feed");
    }

    #[test]
    fn the_plan_and_the_hash_share_one_edge_order() {
        let forward = "graph g : App {\n  operator a { on in { out = envelope } }\n  operator b { on in { out = envelope } }\n  . -> a -> b -> .\n}\n";
        let reordered = "graph g : App {\n  operator a { on in { out = envelope } }\n  operator b { on in { out = envelope } }\n  b -> .\n  a -> b\n  . -> a\n}\n";
        let one = compile(&[forward], &Registry::m0()).unwrap();
        let two = compile(&[reordered], &Registry::m0()).unwrap();
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
            &Registry::m0(),
        )
        .unwrap();
        let two = compile(
            &[with_attrs("    behavior = deterministic ; effect = idempotent\n    codec = lines ; command = \"cat\"\n").as_str()],
            &Registry::m0(),
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
                "graph g : App {{\n  operator o {{\n    on in where size({map}) > 0 {{ out = envelope }}\n  }}\n  . -> o -> .\n}}\n"
            )
        };
        let forward = compile(&[with_map("{'a': 1, 'b': 2}").as_str()], &Registry::m0()).unwrap();
        let reversed = compile(&[with_map("{'b': 2, 'a': 1}").as_str()], &Registry::m0()).unwrap();
        assert_eq!(forward.hash, reversed.hash);
        assert_eq!(forward.plan, reversed.plan);
    }

    #[test]
    fn inferred_ports_travel_in_the_plan() {
        let source = "graph g : App {\n  input feed\n  output sink\n  operator o {\n    on event { result = envelope }\n  }\n  .feed -> o.event\n  o.result -> .sink\n}\n";
        let compilation = compile(&[source], &Registry::m0()).unwrap();
        let ports = &compilation.plan.nodes[0].ports;
        assert_eq!(ports.len(), 2);
        assert!(ports.iter().any(|decl| decl.port.name() == "event"));
        assert!(ports.iter().any(|decl| decl.port.name() == "result"));
    }

    #[test]
    fn constants_reach_both_the_hash_and_the_plan() {
        let with_val = |n: &str| {
            format!(
                "graph g : App {{\n  operator o {{\n    val threshold = {n}\n    on in where data.n > threshold {{ out = envelope }}\n  }}\n  . -> o -> .\n}}\n"
            )
        };
        let five = compile(&[with_val("5").as_str()], &Registry::m0()).unwrap();
        let six = compile(&[with_val("6").as_str()], &Registry::m0()).unwrap();
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
            &Registry::m0(),
        )
        .expect_err("missing mandatory attributes must refuse");
        assert!(bag.has_errors());
    }

    #[test]
    fn versions_alias_the_hash_through_the_manifest() {
        let hash = compile(&[M0_PROBE], &Registry::m0()).unwrap().hash;
        let mut manifest = Manifest::embedded();
        assert_eq!(manifest.allocate(hash), Some(GraphVersion(1)));
        assert_eq!(manifest.hash_of(GraphVersion(1)), Some(hash));
    }
}
