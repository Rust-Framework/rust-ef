//! Compilation context and field extraction for the `linq!` macro.
//!
//! `LinqCtx` binds a closure parameter to its entity type so that field
//! references like `b.rating` can be resolved to entity column constants
//! (`Blog::COLUMN_RATING`). `FieldKind` distinguishes scalar columns from
//! navigation fields. `FieldRef` carries the owning entity type alongside
//! the bare field name for multi-entity (join) scenarios.

use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::{Expr, ExprField, ExprLit, ExprPath, Ident, Lit, Member, Type};

// ---------------------------------------------------------------------------
// LinqCtx — compilation context
// ---------------------------------------------------------------------------

/// Compilation context binding a closure parameter to its entity type.
///
/// Carries enough information to resolve `b.field` references to the correct
/// entity's `COLUMN_*` / `FIELD_*` constants, including multi-param join
/// scenarios where multiple entities are in scope.
pub(crate) struct LinqCtx<'a> {
    pub entity: &'a Type,
    pub param: Option<&'a Ident>,
    /// Multi-param closure context for join scenarios, e.g. `|a: Blog, b: Post| ...`.
    /// Empty for single-entity forms (A/B/C non-join).
    pub params: Vec<(Ident, Type)>,
}

impl<'a> LinqCtx<'a> {
    pub(crate) fn single(entity: &'a Type, param: Option<&'a Ident>) -> Self {
        Self {
            entity,
            param,
            params: Vec::new(),
        }
    }

    /// Multi-param context for join clauses. `primary` is the param being matched;
    /// `all_params` is the full list for cross-param resolution.
    pub(crate) fn multi(
        entity: &'a Type,
        primary: &'a Ident,
        all_params: &[(Ident, Type)],
    ) -> Self {
        Self {
            entity,
            param: Some(primary),
            params: all_params
                .iter()
                .map(|(i, t)| (i.clone(), t.clone()))
                .collect(),
        }
    }
}

/// Whether a field reference should resolve to a `COLUMN_*` (scalar) or `FIELD_*`
/// (navigation) constant on the entity type.
pub(crate) enum FieldKind {
    Column,
    Navigation,
}

/// A resolved field reference: the owning entity type and the field's bare name.
pub(crate) struct FieldRef {
    pub entity: Type,
    pub field_name: String,
}

// ---------------------------------------------------------------------------
// Field extraction
// ---------------------------------------------------------------------------

/// Extracts a `COLUMN_*` constant reference for a scalar field.
pub(crate) fn extract_field(ctx: &LinqCtx<'_>, expr: &Expr) -> syn::Result<TokenStream2> {
    let field_ref = extract_field_ref(ctx, expr)?;
    Ok(field_const(
        &field_ref.entity,
        &field_ref.field_name,
        FieldKind::Column,
    ))
}

/// Extracts a `FIELD_*` constant reference for a navigation field.
pub(crate) fn extract_field_nav(ctx: &LinqCtx<'_>, expr: &Expr) -> syn::Result<TokenStream2> {
    let field_ref = extract_field_ref(ctx, expr)?;
    Ok(field_const(
        &field_ref.entity,
        &field_ref.field_name,
        FieldKind::Navigation,
    ))
}

/// Resolves a field-access expression to its owning entity type + bare field name.
/// Recognizes `b.field` (closure param), `Blog::field` (entity path), and bare `field`.
/// For multi-param (join) contexts, resolves the owning entity from the param binding.
pub(crate) fn extract_field_ref(ctx: &LinqCtx<'_>, expr: &Expr) -> syn::Result<FieldRef> {
    match expr {
        Expr::Path(ExprPath { path, .. }) if path.segments.len() == 1 => {
            let field = path.segments[0].ident.to_string();
            Ok(FieldRef {
                entity: ctx.entity.clone(),
                field_name: field,
            })
        }
        Expr::Field(ExprField { base, member, .. }) => {
            let field_name = match member {
                Member::Named(name) => name.to_string(),
                _ => return Err(syn::Error::new_spanned(expr, "expected field name")),
            };
            if let Expr::Path(ExprPath { path, .. }) = &**base {
                // `b.field` where `b` is a closure param (single or multi).
                if path.segments.len() == 1 {
                    let base_ident = &path.segments[0].ident;
                    // Single-param form: matches ctx.param.
                    if let Some(param) = ctx.param {
                        if param == base_ident {
                            return Ok(FieldRef {
                                entity: ctx.entity.clone(),
                                field_name,
                            });
                        }
                    }
                    // Multi-param form: look up in ctx.params (join scenario).
                    if let Some((_, entity)) = ctx.params.iter().find(|(p, _)| p == base_ident) {
                        return Ok(FieldRef {
                            entity: entity.clone(),
                            field_name,
                        });
                    }
                }
                // `Blog::field` form.
                if type_path_matches(path, ctx.entity) {
                    return Ok(FieldRef {
                        entity: ctx.entity.clone(),
                        field_name,
                    });
                }
                // Multi-param: `Post::field` form matching one of the join entities.
                for (_, entity) in &ctx.params {
                    if type_path_matches(path, entity) {
                        return Ok(FieldRef {
                            entity: entity.clone(),
                            field_name,
                        });
                    }
                }
            }
            // Fallback: attribute to the primary entity.
            Ok(FieldRef {
                entity: ctx.entity.clone(),
                field_name,
            })
        }
        _ => Err(syn::Error::new_spanned(
            expr,
            "expected field access like `b.rating`",
        )),
    }
}

pub(crate) fn type_path_matches(path: &syn::Path, entity: &Type) -> bool {
    let entity_name = if let Type::Path(p) = entity {
        p.path
            .segments
            .last()
            .map(|s| s.ident.to_string())
            .unwrap_or_default()
    } else {
        String::new()
    };
    path.segments.last().is_some_and(|s| s.ident == entity_name)
}

/// Generates `Entity::COLUMN_<UPPER>` or `Entity::FIELD_<UPPER>` depending on `kind`.
pub(crate) fn field_const(entity: &Type, field: &str, kind: FieldKind) -> TokenStream2 {
    let prefix = match kind {
        FieldKind::Column => "COLUMN_",
        FieldKind::Navigation => "FIELD_",
    };
    let const_name = Ident::new(
        &format!("{}{}", prefix, field.to_uppercase()),
        proc_macro2::Span::call_site(),
    );
    quote! { #entity::#const_name }
}

pub(crate) fn extract_value(expr: &Expr) -> syn::Result<TokenStream2> {
    match expr {
        Expr::Lit(ExprLit { lit, .. }) => match &lit {
            Lit::Int(i) => Ok(quote! { #i }),
            Lit::Float(f) => Ok(quote! { #f }),
            Lit::Str(s) => Ok(quote! { #s }),
            Lit::Bool(b) => Ok(quote! { #b }),
            _ => Err(syn::Error::new_spanned(lit, "unsupported literal type")),
        },
        Expr::Path(p) if p.path.is_ident("None") => Ok(quote! { None::<String> }),
        other => Ok(quote! { #other }),
    }
}
