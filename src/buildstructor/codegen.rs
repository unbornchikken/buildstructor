use crate::buildstructor::utils::{
    AngleBracketedGenericArgumentsExt, ExprTupleExt, GenericsExt, IdentExt, TypeExt, TypeTupleExt,
};
use crate::lower::{FieldType, Ir};
use proc_macro2::{Ident, TokenStream};
use quote::{format_ident, quote};
use syn::punctuated::Punctuated;
use syn::{Expr, ExprCall, GenericArgument, Generics, Index, Result, Type};
extern crate inflector;
use inflector::Inflector;

pub fn codegen(ir: Ir) -> Result<TokenStream> {
    let module_name = &ir.module_name;
    let target_name = &ir.target_name;

    let (impl_generics, ty_generics, where_clause) = &ir.generics.split_for_impl();

    let param_generics = ir.param_generics();

    let builder_generics =
        Generics::combine(vec![&ir.generics, &ir.method_generics, &param_generics]);
    let (builder_impl_generics, _, builder_where_clause) = builder_generics.split_for_impl();

    let builder_generics_tuple = Generics::combine(vec![&ir.generics, &ir.method_generics]);
    let builder_tuple_ty_generics = builder_generics_tuple
        .to_generic_args()
        .insert(0, Type::Tuple(param_generics.to_tuple_type()));

    let all_generics = Generics::combine(vec![
        &ir.builder_generics,
        &ir.generics,
        &ir.method_generics,
    ]);
    let (_, all_ty_generics, _) = all_generics.split_for_impl();

    let method_generics = &ir.method_generics;
    let builder_generics = Generics::combine(vec![&ir.method_generics, &ir.generics]);
    let target_generics_raw: Vec<GenericArgument> = builder_generics
        .to_generic_args()
        .args
        .into_iter()
        .collect();

    let constructor_args = ir.constructor_args();
    let constructor_return = &ir.return_type;
    let builder_name = &ir.builder_name;
    let builder_methods = builder_methods(&ir)?;
    let builder_state_type_initial = ir.builder_state_type_initial();
    let builder_state_initial = ir.builder_state_initial();

    let async_token = ir.is_async.then(|| quote! {async});
    let await_token = ir.is_async.then(|| quote! {.await});
    let vis = &ir.vis;

    Ok(quote! {
        impl #impl_generics #target_name #ty_generics #where_clause {
            #vis fn builder #method_generics() -> #module_name::#builder_name<#builder_state_type_initial, #(#target_generics_raw) *> {
                #module_name::new()
            }
        }
        use #module_name::*;

        mod #module_name {
            use super::*;

            pub fn new #builder_generics() -> #builder_name<#builder_state_type_initial, #(#target_generics_raw), *>
            {
                #builder_name {
                    fields: (#(#builder_state_initial ,) *),
                    phantom: core::default::Default::default()
                }
            }

           pub struct __Required<T> {
                phantom: std::marker::PhantomData<T>,
            }
            pub struct __Optional<T> {
                lazy: Option<T>,
            }
            pub struct __Set<T> {
                value: T,
            }

            fn __set<T>(value: T) -> __Set<T> {
                __Set { value }
            }

            fn __required<T>() -> __Required<T> {
                __Required::<T> {
                    phantom: core::default::Default::default(),
                }
            }

            fn __optional<T>() -> __Optional<T> {
                __Optional::<T> {
                    lazy: None,
                }
            }

            impl<T: Default> From<__Optional<T>> for __Set<T> {
                fn from(o: __Optional<T>) -> Self {
                    __Set {
                        value: o.lazy.unwrap_or_default(),
                    }
                }
            }


            pub struct #builder_name #all_ty_generics {
                fields: __P,
                phantom: (#(core::marker::PhantomData<#target_generics_raw>,) *)
            }

            #(#builder_methods)*

            impl #builder_impl_generics #builder_name #builder_tuple_ty_generics #builder_where_clause {
                pub #async_token fn build(self) #constructor_return {
                    #target_name::new(#(#constructor_args),*) #await_token
                }
            }
        }
    })
}

pub fn builder_methods(ir: &Ir) -> Result<Vec<TokenStream>> {
    let builder_generics = Generics::combine(vec![&ir.method_generics, &ir.generics]);
    Ok(ir.builder_fields
        .iter()
        .enumerate()
        .map(|(idx, f)| {
            let builder_name = &ir.builder_name;
            let field_name = &f.name;
            let method_name = format_ident!("{}", f.name);
            let ty = &f.ty;
            let builder_type_generics = ir.builder_type_generics();
            let params_before = match f.field_type {
                FieldType::Regular =>
                    builder_type_generics
                        .to_tuple_type()
                        .with_type(idx, f.ty.clone().wrap_in_generic(format_ident!("__Required"))),
                _ => builder_type_generics
                    .to_tuple_type()
                    .with_type(idx, f.ty.clone().wrap_in_generic(format_ident!("__Optional"))),

            };
            let params_after = match f.field_type {
                FieldType::Regular | FieldType::Optional =>
                    builder_type_generics
                        .to_tuple_type()
                        .with_type(idx, f.ty.clone().wrap_in_generic(format_ident!("__Set"))),
                _ => builder_type_generics
                    .to_tuple_type()
                    .with_type(idx, f.ty.clone().wrap_in_generic(format_ident!("__Optional"))),
            };
            let before = builder_generics
                .to_generic_args()
                .insert(0, Type::Tuple(params_before));
            let after = builder_generics
                .to_generic_args()
                .insert(0, Type::Tuple(params_after));

            let set = call(format_ident!("__set"), vec![Expr::Path(field_name.to_expr_path())]);
            let new_state = params(ir, idx, field_name, &builder_type_generics, set);
            let set_some = call(format_ident!("__set"), vec![call(format_ident!("Some"), vec![Expr::Path(field_name.to_expr_path())])]);
            let new_state_option = params(ir, idx, field_name, &builder_type_generics, set_some);
            let builder_type_generics = Generics::combine(vec![&builder_type_generics.without(idx), &builder_generics]);
            let generic_param = ty.generic_args();

            match f.field_type {
                FieldType::Optional => {
                    let and_method_name = format_ident!("and_{}", f.name);
                    quote! {
                        impl #builder_type_generics #builder_name #before {
                            pub fn #method_name (self, #field_name: #generic_param) -> #builder_name #after {
                                #builder_name {
                                    fields: #new_state_option,
                                    phantom: core::default::Default::default()
                                }
                            }
                            pub fn #and_method_name (self, #field_name: #ty) -> #builder_name #after {
                                #builder_name {
                                    fields: #new_state,
                                    phantom: core::default::Default::default()
                                }
                            }
                        }
                    }
                },
                FieldType::Set => {
                    let (singular, plural) = single_plural_names(field_name);
                    let field_collection_type = &f.collection_type;
                    let index = Index::from(idx);
                    quote! {
                        impl #builder_type_generics #builder_name #before {

                            pub fn #plural (mut self, #field_name: #ty) -> #builder_name #before {
                                self.fields.#index.lazy.get_or_insert_with(||core::default::Default::default()).extend(#field_name.into_iter());
                                self
                            }

                            pub fn #singular (mut self, value: #field_collection_type) -> #builder_name #before {
                                self.fields.#index.lazy.get_or_insert_with(||core::default::Default::default()).insert(value);
                                self
                            }

                        }
                    }
                },
                FieldType::Vec => {
                    let (singular, plural) = single_plural_names(field_name);
                    let field_collection_type = &f.collection_type;
                    let index = Index::from(idx);
                    quote! {
                        impl #builder_type_generics #builder_name #before {

                            pub fn #plural (mut self, #field_name: #ty) -> #builder_name #before {
                                self.fields.#index.lazy.get_or_insert_with(||core::default::Default::default()).extend(#field_name.into_iter());
                                self
                            }

                            pub fn #singular (mut self, value: #field_collection_type) -> #builder_name #before {
                                self.fields.#index.lazy.get_or_insert_with(||core::default::Default::default()).push(value);
                                self
                            }

                        }
                    }
                },
                FieldType::Map => {
                    let (singular, plural) = single_plural_names(field_name);
                    let field_key_type = &f.key_type;
                    let field_value_type = &f.value_type;
                    let index = Index::from(idx);
                    quote! {
                        impl #builder_type_generics #builder_name #before {

                            pub fn #plural (mut self, #field_name: #ty) -> #builder_name #before {
                                self.fields.#index.lazy.get_or_insert_with(||core::default::Default::default()).extend(#field_name.into_iter());
                                self
                            }

                            pub fn #singular (mut self, key: #field_key_type, value: #field_value_type) -> #builder_name #before {
                                self.fields.#index.lazy.get_or_insert_with(||core::default::Default::default()).insert(key, value);
                                self
                            }
                        }
                    }
                },
                _ => quote! {
                    impl #builder_type_generics #builder_name #before {
                        pub fn #method_name (self, #field_name: #ty) -> #builder_name #after {
                            #builder_name {
                                fields: #new_state,
                                phantom: core::default::Default::default()
                            }
                        }
                    }
                },
            }

        })
        .collect())
}

fn single_plural_names(ident: &Ident) -> (Ident, Ident) {
    let plural = format_ident!("{}", ident);
    let mut singular = format_ident!("{}", ident.to_string().to_singular());
    if plural == singular {
        singular = format_ident!("{}_entry", ident);
    }
    (singular, plural)
}

fn call(name: Ident, params: Vec<Expr>) -> Expr {
    Expr::Call(ExprCall {
        attrs: Default::default(),
        func: Box::new(Expr::Path(name.to_expr_path())),
        paren_token: Default::default(),
        args: Punctuated::from_iter(params),
    })
}

fn params(
    ir: &Ir,
    idx: usize,
    field_name: &Ident,
    builder_type_generics: &Generics,
    set: Expr,
) -> Expr {
    Expr::Tuple(
        builder_type_generics
            .to_expr_tuple(|idxp, _| {
                if idx == idxp {
                    Expr::Path(field_name.to_expr_path())
                } else {
                    ir.tuple_field(idxp)
                }
            })
            .with_expr(idx, set),
    )
}

#[cfg(test)]
mod tests {
    use crate::analyze::analyze;
    use crate::buildstructor::tests::*;
    use crate::codegen::codegen;
    use crate::lower::lower;

    macro_rules! assert_codegen {
        ($input:expr) => {
            let model = analyze($input).expect("Analysis failed");
            let ir = lower(model).expect("Ir failed");
            if let Ok(codegen) = codegen(ir) {
                if let Ok(new_ast) = syn::parse2(codegen.clone()) {
                    let output = prettyplease::unparse(&new_ast);
                    insta::assert_snapshot!(output);
                } else {
                    panic!("Failed to generate valid code:\n{}", codegen);
                }
            } else {
                panic!("Failed generate code");
            }
        };
    }

    #[test]
    fn single_field_test() {
        assert_codegen!(single_field_test_case());
    }

    #[test]
    fn pub_test() {
        assert_codegen!(pub_test_case());
    }

    #[test]
    fn multi_field_test() {
        assert_codegen!(multi_field_test_case());
    }

    #[test]
    fn generic_test() {
        assert_codegen!(generic_test_case());
    }

    #[test]
    fn async_test() {
        assert_codegen!(async_test_case());
    }

    #[test]
    fn fallible_test() {
        assert_codegen!(fallible_test_case());
    }

    #[test]
    fn into_test() {
        assert_codegen!(into_test_case());
    }

    #[test]
    fn into_where_test() {
        assert_codegen!(into_where_test_case());
    }

    #[test]
    fn option_test() {
        assert_codegen!(option_test_case());
    }

    #[test]
    fn collection_test() {
        assert_codegen!(collections_test_case());
    }
}