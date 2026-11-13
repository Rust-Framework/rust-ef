//! Resolve the real dependency name of the `rust-ef` crate.
//!
//! Before MACRO-1 every expansion hardcoded `rust_ef::…`, so a downstream crate
//! that renamed the dependency (`ef = { package = "rust-ef" }`) had to add
//! `extern crate ef as rust_ef;` shims to its crate roots. Expansions now
//! address the crate through [`ef_crate_path`].

use proc_macro2::{Span, TokenStream};
use quote::quote;

/// Path to the `rust-ef` crate as seen from the crate the macro expands into.
///
/// - Dependency found (renamed or not) → `::<real name>`, e.g. `::ef`.
/// - Expanding inside `rust-ef` itself (unit tests) → `crate`.
/// - Doctests are compiled as a separate crate linked against the library, so
///   `crate` would point at the doctest wrapper; rustdoc flags them with
///   `UNSTABLE_RUSTDOC_TEST_PATH`, and the library is `::rust_ef` there.
/// - Manifest unreadable or dependency absent → keep the historical
///   `::rust_ef` so `rust-ef-macros` used directly behaves as before.
pub(crate) fn ef_crate_path() -> TokenStream {
    match proc_macro_crate::crate_name("rust-ef") {
        Ok(proc_macro_crate::FoundCrate::Itself) => {
            if std::env::var_os("UNSTABLE_RUSTDOC_TEST_PATH").is_some() {
                quote!(::rust_ef)
            } else {
                quote!(crate)
            }
        }
        Ok(proc_macro_crate::FoundCrate::Name(name)) => {
            let ident = syn::Ident::new(&name, Span::call_site());
            quote!(::#ident)
        }
        Err(_) => quote!(::rust_ef),
    }
}
