/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */

mod codegen;

use proc_macro2::{Ident, Literal, TokenStream};
use quote::{format_ident, quote, ToTokens};
use syn::{DataEnum, DataStruct, Generics};

use crate::{FieldProps, TypeProps};

use self::codegen::{
    generate_serialize_impl_for, with_enum_variants, with_struct_fields, with_text_variants, Field,
    FieldKind, Variant, VariantKind,
};

/// Generates an implementation of the `XmlSerialize` trait for a Rust struct
/// and its fields.
pub(crate) fn write_serialize_impl_for_struct(
    ident: Ident,
    generics: Generics,
    input: DataStruct,
    props: TypeProps,
) -> TokenStream {
    // Process the struct's fields in order to determine how to represent them,
    // based on struct type and any consumer-applied attributes.
    let fields = match input.fields {
        // Fields in a regular struct, i.e. declared with a name and type.
        syn::Fields::Named(fields) => {
            let map_result: Result<Vec<Field>, syn::Error> = fields
                .named
                .into_iter()
                .map(|field| {
                    // We should be able to unwrap without panicking, since we
                    // know these are named fields.
                    let ident = field.ident.unwrap();
                    let accessor = quote!(self.#ident);

                    let field = Field {
                        kind: FieldKind::Named(ident),
                        ty: field.ty.into_token_stream(),
                        accessor,
                        props: FieldProps::try_from_attrs(field.attrs, true)?,
                    };

                    Ok(field)
                })
                .collect();

            match map_result {
                Ok(fields) => fields,
                Err(err) => return err.into_compile_error(),
            }
        }

        // Fields in a tuple struct, i.e. declared by type and position only.
        syn::Fields::Unnamed(fields) => {
            let map_result: Result<Vec<Field>, syn::Error> = fields
                .unnamed
                .into_iter()
                .enumerate()
                .map(|(idx, field)| {
                    let idx_literal = Literal::usize_unsuffixed(idx);
                    let accessor = quote!(self.#idx_literal);

                    let field = Field {
                        kind: FieldKind::Unnamed,
                        ty: field.ty.into_token_stream(),
                        accessor,
                        props: FieldProps::try_from_attrs(field.attrs, false)?,
                    };

                    Ok(field)
                })
                .collect();

            match map_result {
                Ok(fields) => fields,
                Err(err) => return err.into_compile_error(),
            }
        }

        // A unit struct, i.e. one which has no fields.
        syn::Fields::Unit => vec![],
    };

    generate_serialize_impl_for(ident, generics, props, with_struct_fields(fields))
}

/// Generates an implementation of the `XmlSerialize` trait (and the
/// `XmlSerializeAttr` trait if appropriate) for a Rust enum, its variants, and
/// their fields.
pub(crate) fn write_serialize_impl_for_enum(
    ident: Ident,
    generics: Generics,
    input: DataEnum,
    props: TypeProps,
) -> TokenStream {
    if props.should_serialize_as_text {
        // We depend on the code which generates `TypeProps` to handle verifying
        // that this enum consists solely of unit variants when setting this
        // property, so we just collect variant identifiers.
        let variants = input
            .variants
            .into_iter()
            .map(|variant| variant.ident)
            .collect();

        return generate_serialize_impl_for(ident, generics, props, with_text_variants(variants));
    }

    // We build a list of errors so that we can combine them later and emit
    // them all instead of quitting at the first we encounter.
    let mut errors = Vec::new();

    // Process the enum's variants in order to determine how to represent them,
    // based on variant type and any consumer-applied attributes.
    let variants = input
        .variants
        .into_iter()
        .map(|variant| {
            let kind = match variant.fields {
                syn::Fields::Named(fields) => {
                    let fields = fields
                        .named
                        .into_iter()
                        .map(|field| {
                            // We should be able to unwrap without panicking, since we
                            // know these are named fields.
                            let ident = field.ident.unwrap();
                            let accessor = quote!(#ident);

                            let props = FieldProps::try_from_attrs(field.attrs, true)
                                .unwrap_or_else(|err| {
                                    errors.push(err);

                                    FieldProps::default()
                                });

                            Field {
                                kind: FieldKind::Named(ident),
                                ty: field.ty.into_token_stream(),
                                accessor,
                                props,
                            }
                        })
                        .collect();

                    VariantKind::Struct(fields)
                }
                syn::Fields::Unnamed(fields) => {
                    let fields = fields
                        .unnamed
                        .into_iter()
                        .enumerate()
                        .map(|(idx, field)| {
                            let idx = Literal::usize_unsuffixed(idx);
                            let accessor = format_ident!("field{idx}").into_token_stream();

                            let props = FieldProps::try_from_attrs(field.attrs, false)
                                .unwrap_or_else(|err| {
                                    errors.push(err);

                                    FieldProps::default()
                                });

                            Field {
                                kind: FieldKind::Unnamed,
                                ty: field.ty.into_token_stream(),
                                accessor,
                                props,
                            }
                        })
                        .collect();

                    VariantKind::Tuple(fields)
                }
                syn::Fields::Unit => VariantKind::Unit,
            };

            Variant {
                ident: variant.ident,
                kind,
            }
        })
        .collect();

    // Combine and return errors if there are any. If none, we've successfully
    // handled all fields and can generate the final implementation.
    let err = errors.into_iter().reduce(|mut acc, err| {
        acc.combine(err);

        acc
    });

    if let Some(err) = err {
        return err.into_compile_error();
    }

    let ns_prefix = props.ns_prefix_for_variants.clone();

    generate_serialize_impl_for(
        ident,
        generics,
        props,
        with_enum_variants(variants, ns_prefix),
    )
}
