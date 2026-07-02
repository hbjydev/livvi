use heck::ToSnekCase;
use proc_macro::TokenStream;
use proc_macro2::Span;
use quote::quote;
use syn::{
    Attribute, DeriveInput, Expr, ExprLit, Ident, Lit, LitStr, Meta, MetaNameValue, Type,
    parse_macro_input, spanned::Spanned,
};

#[proc_macro_derive(ToolSchema, attributes(tool))]
pub fn derive_tool_schema(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    match impl_tool_schema(input) {
        Ok(tokens) => tokens.into(),
        Err(e) => e.to_compile_error().into(),
    }
}

fn impl_tool_schema(input: DeriveInput) -> syn::Result<proc_macro2::TokenStream> {
    if !matches!(input.data, syn::Data::Struct(_)) {
        return Err(syn::Error::new(
            input.ident.span(),
            "ToolSchema can only be derived for structs",
        ));
    }

    let struct_name = &input.ident;
    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();

    let crate_path = crate_path();
    let attr = parse_tool_attribute(&input.attrs)?;

    let name = attr
        .name
        .map(|s| s.value())
        .unwrap_or_else(|| struct_name.to_string().to_snek_case());

    let description = attr
        .description
        .map(|s| s.value())
        .or_else(|| extract_doc_comment(&input.attrs))
        .unwrap_or_default();

    let input_type = attr.input.ok_or_else(|| {
        syn::Error::new(
            Span::call_site(),
            "missing required `input = Type` in #[tool { ... }]",
        )
    })?;

    let name_lit = LitStr::new(&name, Span::call_site());
    let desc_lit = LitStr::new(&description, Span::call_site());

    Ok(quote! {
        impl #impl_generics #crate_path::tool::ToolSchema for #struct_name #ty_generics #where_clause {
            fn name(&self) -> &'static str {
                #name_lit
            }

            fn description(&self) -> &'static str {
                #desc_lit
            }

            fn input_schema(&self) -> ::schemars::Schema {
                ::schemars::schema_for!(#input_type)
            }
        }
    })
}

fn crate_path() -> proc_macro2::TokenStream {
    use proc_macro_crate::{FoundCrate, crate_name};

    let current_crate = std::env::var("CARGO_CRATE_NAME").ok();
    if current_crate.as_deref() == Some("livvi_core") {
        return quote! { crate };
    }

    match crate_name("livvi-core") {
        Ok(FoundCrate::Name(name)) => {
            let ident = Ident::new(&name, Span::call_site());
            quote! { ::#ident }
        }
        Ok(FoundCrate::Itself) | Err(_) => quote! { ::livvi_core },
    }
}

struct ToolAttribute {
    name: Option<LitStr>,
    input: Option<Type>,
    description: Option<LitStr>,
}

fn parse_tool_attribute(attrs: &[Attribute]) -> syn::Result<ToolAttribute> {
    let mut result = ToolAttribute {
        name: None,
        input: None,
        description: None,
    };

    for attr in attrs {
        if attr.path().is_ident("tool") {
            let meta_list = match &attr.meta {
                Meta::List(list) => list,
                _ => return Err(syn::Error::new(attr.span(), "expected #[tool { ... }]")),
            };

            let parser =
                syn::punctuated::Punctuated::<ToolAttributeField, syn::Token![,]>::parse_terminated;
            let fields = meta_list.parse_args_with(parser)?;

            for field in fields {
                match field.key.to_string().as_str() {
                    "name" => {
                        if result.name.is_some() {
                            return Err(syn::Error::new(
                                field.key.span(),
                                "duplicate `name` in #[tool { ... }]",
                            ));
                        }
                        let lit: LitStr = syn::parse2(field.value)?;
                        result.name = Some(lit);
                    }
                    "input" => {
                        if result.input.is_some() {
                            return Err(syn::Error::new(
                                field.key.span(),
                                "duplicate `input` in #[tool { ... }]",
                            ));
                        }
                        let ty: Type = syn::parse2(field.value)?;
                        result.input = Some(ty);
                    }
                    "description" => {
                        if result.description.is_some() {
                            return Err(syn::Error::new(
                                field.key.span(),
                                "duplicate `description` in #[tool { ... }]",
                            ));
                        }
                        let lit: LitStr = syn::parse2(field.value)?;
                        result.description = Some(lit);
                    }
                    other => {
                        return Err(syn::Error::new(
                            field.key.span(),
                            format!("unknown key `{}` in #[tool {{ ... }}]", other),
                        ));
                    }
                }
            }
        }
    }

    Ok(result)
}

struct ToolAttributeField {
    key: Ident,
    value: proc_macro2::TokenStream,
}

impl syn::parse::Parse for ToolAttributeField {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let key: Ident = input.parse()?;
        input.parse::<syn::Token![=]>()?;

        let mut value = proc_macro2::TokenStream::new();
        while !input.is_empty() {
            if input.peek(syn::Token![,]) {
                break;
            }
            let tt: proc_macro2::TokenTree = input.parse()?;
            value.extend(Some(tt));
        }

        Ok(ToolAttributeField { key, value })
    }
}

fn extract_doc_comment(attrs: &[Attribute]) -> Option<String> {
    let mut docs = Vec::new();
    for attr in attrs {
        if attr.path().is_ident("doc")
            && let Meta::NameValue(MetaNameValue {
                value:
                    Expr::Lit(ExprLit {
                        lit: Lit::Str(s), ..
                    }),
                ..
            }) = &attr.meta
        {
            docs.push(s.value().trim().to_string());
        }
    }

    if docs.is_empty() {
        None
    } else {
        Some(docs.join(" "))
    }
}
