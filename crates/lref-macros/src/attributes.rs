//! Attribute parsing helpers for rust-ef-macros.
//!
//! Parses the custom attributes used in `#[derive(EntityType)]`.

#![allow(dead_code)]

use syn::{Attribute, LitStr, Meta};

pub fn parse_table_attr(attrs: &[Attribute]) -> Option<String> {
    for attr in attrs {
        if attr.path().is_ident("table") {
            if let Ok(lit) = attr.parse_args::<LitStr>() {
                return Some(lit.value());
            }
        }
    }
    None
}

pub fn parse_column_attr(attrs: &[Attribute]) -> Option<String> {
    for attr in attrs {
        if attr.path().is_ident("column") {
            if let Ok(lit) = attr.parse_args::<LitStr>() {
                return Some(lit.value());
            }
        }
    }
    None
}

pub fn parse_max_length_attr(attrs: &[Attribute]) -> Option<usize> {
    for attr in attrs {
        if attr.path().is_ident("max_length") {
            if let Ok(lit) = attr.parse_args::<syn::LitInt>() {
                return lit.base10_parse().ok();
            }
        }
    }
    None
}

pub fn has_bool_attr(attrs: &[Attribute], name: &str) -> bool {
    attrs.iter().any(|attr| attr.path().is_ident(name))
}

pub fn parse_foreign_key_attr(attrs: &[Attribute]) -> Option<String> {
    for attr in attrs {
        if attr.path().is_ident("foreign_key") {
            if let Meta::List(list) = &attr.meta {
                let tokens = list.tokens.to_string();
                return Some(tokens.trim().to_string());
            }
        }
    }
    None
}
