use proc_macro::TokenStream;
use proc_macro2::Span;
use quote::{format_ident, quote};
use syn::{
    Attribute, Expr, ExprLit, FnArg, Ident, ItemFn, Lit, Meta, MetaNameValue, ReturnType, Type,
    parse::Parser, parse_macro_input, spanned::Spanned,
};

#[proc_macro_attribute]
pub fn tool(args: TokenStream, input: TokenStream) -> TokenStream {
    let args = parse_tool_args(&proc_macro2::TokenStream::from(args));
    let input_fn = parse_macro_input!(input as ItemFn);

    match impl_tool(args, input_fn) {
        Ok(tokens) => tokens.into(),
        Err(e) => e.to_compile_error().into(),
    }
}

fn impl_tool(args: ToolArgs, input_fn: ItemFn) -> syn::Result<proc_macro2::TokenStream> {
    let crate_path = crate_path();
    let fn_name = &input_fn.sig.ident;
    let wrapper_name = fn_name.clone();
    let impl_fn_name = format_ident!("__tool_impl_{}", fn_name);
    let vis = &input_fn.vis;

    let tool_name = args.name.unwrap_or_else(|| fn_name.to_string());
    let description = args
        .description
        .or_else(|| extract_doc_comment(&input_fn.attrs))
        .unwrap_or_default();

    let input_type = find_input_type(&input_fn.sig.inputs)?;

    let mut impl_fn = input_fn.clone();
    impl_fn.sig.ident = impl_fn_name.clone();
    impl_fn.vis = syn::Visibility::Inherited; // make the inner function private

    let extraction = generate_extraction(&input_fn.sig.inputs, &crate_path);
    let output_expr = generate_output_expr(&input_fn.sig.output, &crate_path);

    let param_names = input_fn
        .sig
        .inputs
        .iter()
        .enumerate()
        .map(|(i, _)| format_ident!("__param_{}", i))
        .collect::<Vec<_>>();

    let handler_impl = quote! {
        #[#crate_path::async_trait]
        impl<S: ::core::marker::Send + ::core::marker::Sync + 'static> #crate_path::tool::ToolHandler<S> for #wrapper_name {
            fn schema(&self) -> #crate_path::tool::ToolDefinition {
                #crate_path::tool::ToolDefinition {
                    name: #tool_name.to_string(),
                    description: #description.to_string(),
                    input_schema: #crate_path::schemars::schema_for!(#input_type),
                }
            }

            async fn call(&self, ctx: &#crate_path::tool::ToolContext<'_, S>, args: #crate_path::serde_json::Value) -> #crate_path::tool::ToolCallOutput {
                #extraction
                let __tool_result = #impl_fn_name(#(#param_names),*).await;
                #output_expr
            }
        }
    };

    let output = quote! {
        #impl_fn

        #[allow(non_camel_case_types)]
        #vis struct #wrapper_name;

        #handler_impl
    };

    Ok(output)
}

struct ToolArgs {
    name: Option<String>,
    description: Option<String>,
}

impl ToolArgs {
    fn empty() -> Self {
        ToolArgs {
            name: None,
            description: None,
        }
    }
}

fn parse_tool_args(args: &proc_macro2::TokenStream) -> ToolArgs {
    let mut result = ToolArgs::empty();

    if args.is_empty() {
        return result;
    }

    let parser = syn::punctuated::Punctuated::<Meta, syn::Token![,]>::parse_terminated;
    let metas = match parser.parse2(args.clone()) {
        Ok(metas) => metas,
        Err(_) => return result,
    };

    for meta in metas {
        if let Meta::NameValue(MetaNameValue { path, value, .. }) = meta
            && let Some(key) = path.get_ident()
        {
            let value = match value {
                Expr::Lit(ExprLit {
                    lit: Lit::Str(s), ..
                }) => s.value(),
                _ => continue,
            };
            match key.to_string().as_str() {
                "name" => result.name = Some(value),
                "description" => result.description = Some(value),
                _ => {}
            }
        }
    }

    result
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

fn find_input_type(
    inputs: &syn::punctuated::Punctuated<FnArg, syn::Token![,]>,
) -> syn::Result<Type> {
    let mut found = None;

    for input in inputs {
        let typed = match input {
            FnArg::Typed(t) => t,
            FnArg::Receiver(_) => {
                return Err(syn::Error::new(input.span(), "tools cannot be methods"));
            }
        };

        if let Some(ty) = extract_input_type(&typed.ty) {
            if found.is_some() {
                return Err(syn::Error::new(
                    typed.ty.span(),
                    "only one `Input<T>` extractor is allowed per tool",
                ));
            }
            found = Some(ty.clone());
        }
    }

    Ok(found.unwrap_or_else(|| syn::parse_quote! { () }))
}

fn extract_input_type(ty: &Type) -> Option<&Type> {
    if let Type::Path(type_path) = ty {
        let segment = type_path.path.segments.last()?;
        if segment.ident != "Input" {
            return None;
        }
        if let syn::PathArguments::AngleBracketed(args) = &segment.arguments {
            return args.args.iter().find_map(|arg| {
                if let syn::GenericArgument::Type(ty) = arg {
                    Some(ty)
                } else {
                    None
                }
            });
        }
    }
    None
}

fn generate_extraction(
    inputs: &syn::punctuated::Punctuated<FnArg, syn::Token![,]>,
    crate_path: &proc_macro2::TokenStream,
) -> proc_macro2::TokenStream {
    let mut tokens = proc_macro2::TokenStream::new();

    for (i, input) in inputs.iter().enumerate() {
        let typed = match input {
            FnArg::Typed(t) => t,
            FnArg::Receiver(_) => continue,
        };
        let param_name = format_ident!("__param_{}", i);
        let ty = &typed.ty;

        tokens.extend(quote! {
            let #param_name: #ty = match #crate_path::tool::FromToolContext::from_tool_context(ctx, &args) {
                Ok(v) => v,
                Err(e) => return #crate_path::tool::ToolCallOutput::Error(e.to_string()),
            };
        });
    }

    tokens
}

fn generate_output_expr(
    output: &ReturnType,
    crate_path: &proc_macro2::TokenStream,
) -> proc_macro2::TokenStream {
    match output {
        ReturnType::Default => {
            quote! {
                #crate_path::tool::ToolCallOutput::Success(
                    #crate_path::serde_json::to_string(&()).unwrap_or_default()
                )
            }
        }
        ReturnType::Type(_, ty) => {
            if let Some((ok_ty, err_ty)) = extract_result_types(ty) {
                let ok_serialize = serialize_expr(&ok_ty, &syn::parse_quote! { v }, crate_path);
                let err_serialize = serialize_expr(&err_ty, &syn::parse_quote! { e }, crate_path);
                quote! {
                    match __tool_result {
                        Ok(v) => #crate_path::tool::ToolCallOutput::Success(#ok_serialize),
                        Err(e) => #crate_path::tool::ToolCallOutput::Error(#err_serialize),
                    }
                }
            } else {
                let serialized =
                    serialize_expr(ty, &syn::parse_quote! { __tool_result }, crate_path);
                quote! {
                    #crate_path::tool::ToolCallOutput::Success(#serialized)
                }
            }
        }
    }
}

fn serialize_expr(
    ty: &Type,
    expr: &syn::Expr,
    crate_path: &proc_macro2::TokenStream,
) -> proc_macro2::TokenStream {
    if is_primitive_type(ty) {
        quote! { (#expr).to_string() }
    } else {
        quote! { #crate_path::serde_json::to_string(&(#expr)).unwrap_or_default() }
    }
}

fn is_primitive_type(ty: &Type) -> bool {
    let inner = match ty {
        Type::Reference(r) => &*r.elem,
        _ => ty,
    };

    if let Type::Path(type_path) = inner
        && let Some(ident) = type_path.path.get_ident()
    {
        const PRIMITIVES: &[&str] = &[
            "i8", "i16", "i32", "i64", "i128", "isize", "u8", "u16", "u32", "u64", "u128", "usize",
            "f32", "f64", "bool", "char", "String", "str",
        ];
        return PRIMITIVES.iter().any(|p| ident == p);
    }

    false
}

fn extract_result_types(ty: &Type) -> Option<(Type, Type)> {
    if let Type::Path(type_path) = ty {
        let segment = type_path.path.segments.last()?;
        if segment.ident != "Result" {
            return None;
        }
        if let syn::PathArguments::AngleBracketed(args) = &segment.arguments {
            let mut types = args.args.iter().filter_map(|arg| {
                if let syn::GenericArgument::Type(ty) = arg {
                    Some(ty.clone())
                } else {
                    None
                }
            });
            let ok = types.next()?;
            let err = types.next()?;
            return Some((ok, err));
        }
    }
    None
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
