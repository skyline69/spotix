extern crate proc_macro;

use quote::quote;

use proc_macro2::{Span, TokenStream};
use syn::{parse_macro_input, DeriveInput, Ident};

struct Idents {
    item: Ident,
    collection: Ident,
    derives: Option<Vec<Ident>>,
}

impl Idents {
    fn new(input: &DeriveInput) -> Result<Idents, String> {
        let collection_name =
            attr_string_val(input, "CollectionName").expect("Need [CollectionName=\"...\"]");
        let derives = Idents::parse_derives(input);

        Ok(Idents {
            item: input.ident.clone(),
            collection: Ident::new(&collection_name, Span::call_site()),
            derives: derives,
        })
    }

    fn parse_derives(input: &DeriveInput) -> Option<Vec<Ident>> {
        match attr_string_val(input, "CollectionDerives") {
            Some(derives_str) => {
                if derives_str.is_empty() {
                    return None;
                }

                Some(
                    derives_str
                        .split(",")
                        .map(|s| Ident::new(s.trim(), Span::call_site()))
                        .collect(),
                )
            }
            None => None,
        }
    }

    fn as_parts(&self) -> (&Ident, &Ident, &Option<Vec<Ident>>) {
        (&self.item, &self.collection, &self.derives)
    }
}

struct Docs {
    wrapper: String,
    new: String,
    is_empty: String,
    len: String,
    iter: String,
}

macro_rules! doc_attr {
    ($input:ident, $attr:expr, $default:expr) => {
        attr_string_val($input, $attr).unwrap_or($default)
    };
}

impl Docs {
    fn new(input: &DeriveInput, idents: &Idents) -> Docs {
        let wrapper = doc_attr!(
            input,
            "CollectionDoc",
            format!("A collection of {}s", idents.item)
        );
        let new = doc_attr!(
            input,
            "CollectionNewDoc",
            format!("Creates a new, empty {}", idents.collection)
        );
        let is_empty = doc_attr!(
            input,
            "CollectionIsEmptyDoc",
            format!(
                "Returns true if the {} contains no {}s",
                idents.collection, idents.item
            )
        );
        let len = doc_attr!(
            input,
            "CollectionLenDoc",
            format!(
                "Returns the number of {}s in the {}",
                idents.item, idents.collection
            )
        );
        let iter = doc_attr!(
            input,
            "CollectionIterDoc",
            format!("Returns an iterator over the {}", idents.collection)
        );

        Docs {
            wrapper: wrapper,
            new: new,
            is_empty: is_empty,
            len: len,
            iter: iter,
        }
    }

    fn as_parts(&self) -> (&String, &String, &String, &String, &String) {
        (
            &self.wrapper,
            &self.new,
            &self.is_empty,
            &self.len,
            &self.iter,
        )
    }
}

#[proc_macro_derive(
    WrappedVec,
    attributes(
        CollectionName,
        CollectionDerives,
        CollectionDoc,
        CollectionNewDoc,
        CollectionIsEmptyDoc,
        CollectionLenDoc,
        CollectionIterDoc
    )
)]
pub fn wrapped_vec(token_stream: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let input = parse_macro_input!(token_stream as DeriveInput);
    impl_wrapped_vec(&input).unwrap().into()
}

fn impl_wrapped_vec(input: &DeriveInput) -> Result<TokenStream, String> {
    let idents = Idents::new(input)?;
    let docs = Docs::new(input, &idents);
    Ok(generate_wrapped_vec(&idents, &docs))
}

fn generate_wrapped_vec(idents: &Idents, docs: &Docs) -> TokenStream {
    let (item_ident, collection_ident, collection_derive) = idents.as_parts();
    let (collection_doc, new_doc, is_empty_doc, len_doc, iter_doc) = docs.as_parts();

    let derives_toks = match collection_derive.clone() {
        Some(derives) => {
            quote! { #[derive(#(#derives),*)] }
        }
        None => {
            quote! {}
        }
    };

    quote! {
        #[doc=#collection_doc]
        #derives_toks
        pub struct #collection_ident(Vec<#item_ident>);

        impl ::std::iter::FromIterator<#item_ident> for #collection_ident {
            fn from_iter<I: IntoIterator<Item=#item_ident>>(iter: I) -> Self {
                let mut inner = vec![];
                inner.extend(iter);
                #collection_ident(inner)
            }
        }

        impl From<Vec<#item_ident>> for #collection_ident {
            fn from(ids: Vec<#item_ident>) -> #collection_ident {
                let mut new = #collection_ident::new();
                new.extend(ids);
                new
            }
        }

        impl IntoIterator for #collection_ident {
            type Item = #item_ident;
            type IntoIter = ::std::vec::IntoIter<#item_ident>;

            fn into_iter(self) -> Self::IntoIter {
                self.0.into_iter()
            }
        }

        impl<'a> IntoIterator for &'a #collection_ident {
            type Item = &'a #item_ident;
            type IntoIter = ::std::slice::Iter<'a, #item_ident>;

            fn into_iter(self) -> Self::IntoIter {
                self.0.iter()
            }
        }

        impl Extend<#item_ident> for #collection_ident {
            fn extend<T: IntoIterator<Item=#item_ident>>(&mut self, iter: T) {
                self.0.extend(iter);
            }
        }

        impl #collection_ident {

            #[doc=#new_doc]
            pub fn new() -> #collection_ident {
                #collection_ident(vec![])
            }

            #[doc=#is_empty_doc]
            pub fn is_empty(&self) -> bool {
                self.0.is_empty()
            }

            #[doc=#len_doc]
            pub fn len(&self) -> usize {
                self.0.len()
            }

            #[doc=#iter_doc]
            pub fn iter<'a>(&'a self) -> ::std::slice::Iter<'a, #item_ident> {
                self.into_iter()
            }

        }
    }
}

fn attr_string_val(input: &DeriveInput, attr_name: &'static str) -> Option<String> {
    input.attrs.iter().find_map(|input| {
        if let Ok(attribute) = input.parse_meta() {
            if let syn::Meta::NameValue(name_value) = attribute {
                if name_value.path.is_ident(attr_name) {
                    if let syn::Lit::Str(s) = &name_value.lit {
                        return Some(s.value());
                    }
                }
            }
        }
        None
    })
}
