//! Flat metadata arguments are ordinary typed constants, not another value algebra.

use super::*;
use crate::datatype::IntegerType;
use crate::schema_annotation::{AnnotationApplication, AnnotationBinding};
use crate::value::Value;

impl Checker<'_, '_> {
    pub(super) fn load_standard_annotations(&mut self) -> Option<BTreeSet<String>> {
        // A separate source id keeps trusted declarations distinct from authored
        // spans. A broken embedded definition is a compiler defect, not an error
        // attributed to the user's file.
        let mut diagnostics = Bag::new();
        let tree = crate::compiler::parser::parse(
            SourceId::new(u32::MAX),
            crate::stdlib::ANNOTATION_DEFINITIONS,
            self.registry,
            &mut diagnostics,
        );
        if diagnostics.has_errors()
            || tree.decls.len() != 4
            || !tree.decls.iter().all(|declaration| matches!(
                declaration,
                TopDecl::Type(declaration)
                    if matches!(declaration.name.text.as_str(), "sensitive" | "secret" | "personal" | "measure")
                        && !declaration.ty.optional
                        && declaration.ty.annotations.is_empty()
                        && matches!(declaration.ty.alternatives.as_slice(), [BaseType::FlatStruct { .. }])
            ))
        {
            self.bag.push(Diagnostic::new(
                crate::compiler::E_INTERNAL_PLAN,
                Phase::Check,
                "the embedded standard annotation declarations are invalid",
            ));
            return None;
        }
        let mut names = BTreeSet::new();
        for declaration in tree.decls {
            let TopDecl::Type(declaration) = declaration else {
                return None;
            };
            if !names.insert(declaration.name.text.clone()) {
                self.bag.push(Diagnostic::new(
                    crate::compiler::E_INTERNAL_PLAN,
                    Phase::Check,
                    "the embedded standard annotation declarations contain a duplicate",
                ));
                return None;
            }
            self.type_decls
                .insert(declaration.name.text, declaration.ty);
        }
        Some(names)
    }

    pub(super) fn resolve_business_measure(
        &mut self,
        annotation: &SynSchemaAnnotation,
        target: &DataAnnotationTarget<'_>,
        path: &SchemaPath,
        arguments: &BTreeMap<&str, &SchemaAnnotationConstant>,
    ) -> Option<DataAnnotation> {
        if let Some(name) = ["currency", "basis", "aggregation", "subset_of"]
            .into_iter()
            .find(|name| arguments.contains_key(name))
        {
            self.annotation_refusal(
                annotation.span,
                "measure",
                path,
                &format!("`{name}` is retired from @measure; use only id and unit (for example unit = \"USD\"); valuation and aggregation are separate consumers"),
            );
            return None;
        }
        self.require_annotation_arguments(annotation, path, arguments, &["id", "unit"])?;
        if !matches!(
            target,
            DataAnnotationTarget::Type(DataType::Integer(_) | DataType::Decimal(_))
        ) {
            self.annotation_refusal(
                annotation.span,
                "measure",
                path,
                "a business measure requires an integer or exact decimal field",
            );
            return None;
        }
        self.bind_measure(annotation, path, arguments)
    }

    pub(super) fn bind_measure(
        &mut self,
        annotation: &SynSchemaAnnotation,
        path: &SchemaPath,
        arguments: &BTreeMap<&str, &SchemaAnnotationConstant>,
    ) -> Option<DataAnnotation> {
        let declaration = self.type_decls.get("measure")?.clone();
        let schema = self.resolve_plain_type_ref(&declaration)?;
        let DataType::Struct(fields) = &schema else {
            return None;
        };
        let mut values = Vec::new();
        for field in fields.fields() {
            let value = match arguments.get(field.name()) {
                Some(value) if field.name() == "id" => {
                    Value::Text(self.annotation_symbol(annotation, path, value, field.name())?)
                }
                Some(value) => match typed_constant(value, field.ty()) {
                    Ok(value) => value,
                    Err(detail) => {
                        self.annotation_refusal(
                            annotation.span,
                            "measure",
                            path,
                            &format!(
                                "argument `{}` requires {}: {detail}",
                                field.name(),
                                field.ty()
                            ),
                        );
                        return None;
                    }
                },
                None => {
                    self.annotation_refusal(
                        annotation.span,
                        "measure",
                        path,
                        &format!("missing required argument `{}`", field.name()),
                    );
                    return None;
                }
            };
            let value = match field.ty().narrow(value, None) {
                Ok(value) => value,
                Err(_) => {
                    self.annotation_refusal(
                        annotation.span,
                        "measure",
                        path,
                        &format!(
                            "argument `{}` does not inhabit its standard type",
                            field.name()
                        ),
                    );
                    return None;
                }
            };
            values.push((field.name().to_string(), value));
        }
        match AnnotationApplication::new(
            AnnotationBinding::Measure,
            "rastok/measure@3".into(),
            schema,
            Value::Struct(values),
        ) {
            Ok(application) => Some(DataAnnotation::Declared(Box::new(application))),
            Err(error) => {
                self.annotation_refusal(annotation.span, "measure", path, &error.to_string());
                None
            }
        }
    }

    pub(super) fn resolve_declared_annotation(
        &mut self,
        annotation: &SynSchemaAnnotation,
        declaration: &TypeRef,
        path: &SchemaPath,
    ) -> Option<DataAnnotation> {
        let DataType::Struct(schema) = self.resolve_plain_type_ref(declaration)? else {
            return None;
        };
        let mut pending = vec![declaration];
        while let Some(ty) = pending.pop() {
            if !ty.annotations.is_empty() {
                self.bag.push(
                    Diagnostic::new(
                        E_CHECK_DEFERRED,
                        Phase::Check,
                        "policies on annotation arguments require nested argument-policy transport",
                    )
                    .with_span(annotation.span)
                    .with_field("annotation", annotation.name.text.clone())
                    .with_field("path", path.to_string()),
                );
                return None;
            }
            for base in &ty.alternatives {
                match base {
                    BaseType::Named(name) => {
                        if let Some(aliased) = self.type_decls.get(&name.text) {
                            pending.push(aliased);
                        }
                    }
                    BaseType::FlatStruct { fields, .. } => {
                        pending.extend(fields.iter().map(|field| &field.ty));
                    }
                    _ => {}
                }
            }
        }
        let mut values = BTreeMap::new();
        for argument in &annotation.arguments {
            let Some(field) = schema
                .fields()
                .iter()
                .find(|field| field.name() == argument.name.text)
            else {
                self.annotation_refusal(
                    argument.name.span,
                    &annotation.name.text,
                    path,
                    &format!("unexpected argument `{}`", argument.name.text),
                );
                return None;
            };
            match typed_constant(&argument.value, field.ty()) {
                Ok(value) => {
                    values.insert(field.name().to_string(), value);
                }
                Err(detail) => {
                    self.annotation_refusal(
                        argument.name.span,
                        &annotation.name.text,
                        path,
                        &format!(
                            "argument `{}` requires {}: {detail}",
                            argument.name.text,
                            field.ty()
                        ),
                    );
                    return None;
                }
            }
        }
        for field in schema.fields() {
            let nullable = matches!(field.ty(), DataType::StructuralUnion(union)
                if union.branches().contains(&DataType::Null));
            if !values.contains_key(field.name()) {
                if nullable {
                    let value = field.ty().narrow(Value::Null, None).ok()?;
                    values.insert(field.name().to_string(), value);
                    continue;
                }
                self.annotation_refusal(
                    annotation.span,
                    &annotation.name.text,
                    path,
                    &format!("missing required argument `{}`", field.name()),
                );
                return None;
            }
        }
        let origin = self
            .annotation_origins
            .get(&annotation.name.text)
            .cloned()
            .unwrap_or_else(|| annotation.name.text.clone());
        match AnnotationApplication::new(
            AnnotationBinding::Metadata,
            origin,
            DataType::Struct(schema),
            Value::Struct(values.into_iter().collect()),
        ) {
            Ok(application) => Some(DataAnnotation::Declared(Box::new(application))),
            Err(error) => {
                self.annotation_refusal(
                    annotation.span,
                    &annotation.name.text,
                    path,
                    &error.to_string(),
                );
                None
            }
        }
    }
}

fn typed_constant(
    source: &SchemaAnnotationConstant,
    target: &DataType,
) -> Result<Value, &'static str> {
    let scalar = match target {
        DataType::StructuralUnion(union) => union
            .branches()
            .iter()
            .find(|branch| **branch != DataType::Null)
            .ok_or("the argument type has no scalar branch")?,
        _ => target,
    };
    let value = match source {
        SchemaAnnotationConstant::Symbol(names)
            if matches!(scalar, DataType::Enum(_))
                && names.len() == 1
                && names[0].text != "null" =>
        {
            Value::Enum(names[0].text.clone())
        }
        _ => {
            let text = match source {
                SchemaAnnotationConstant::Symbol(names) => names
                    .iter()
                    .map(|name| name.text.as_str())
                    .collect::<Vec<_>>()
                    .join("."),
                SchemaAnnotationConstant::Text { text, .. } => format!("\"{text}\""),
                SchemaAnnotationConstant::Integer(literal) => literal.text.clone(),
                SchemaAnnotationConstant::Dynamic(expression) => expression.text.clone(),
            };
            let tree = expr::compile(&text, &expr::Signature::new())
                .map_err(|_| "a typed plan constant is required")?;
            expr::const_eval(&tree, &BTreeMap::new())
                .map_err(|_| "the plan constant cannot be evaluated")?
        }
    };
    // Literal integers select the declared signedness; all range/precision
    // checks and decimal rescaling still belong to DataType.
    let value = match (scalar, value) {
        (DataType::Integer(IntegerType::Unsigned(_)), Value::Int(number)) => {
            Value::Uint(u64::try_from(number).map_err(|_| "a negative quantity is not unsigned")?)
        }
        (DataType::Integer(IntegerType::Signed(_)), Value::Uint(number)) => {
            Value::Int(i64::try_from(number).map_err(|_| "the quantity exceeds the signed range")?)
        }
        (_, Value::Null) => Value::Null,
        (_, value) => scalar
            .narrow(value, None)
            .map_err(|_| "the constant does not conform to the declared type")?,
    };
    target
        .narrow(value, None)
        .map_err(|_| "the constant does not conform to the declared type")
}
