use quote::quote;

#[proc_macro_derive(Message, attributes(reply))]
pub fn message_derive(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let input = syn::parse_macro_input!(input as syn::DeriveInput);

    let ty_name = &input.ident;
    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();
    let (reply_type, message_trait) = input
        .attrs
        .iter()
        .find(|attr| attr.path().is_ident("reply"))
        .map(|attr| {
            let tokens = attr.parse_args::<proc_macro2::TokenStream>().unwrap();
            if tokens.is_empty() {
                (quote!(()), quote!(actlet::Action))
            } else {
                (tokens, quote!(actlet::Action))
            }
        })
        .unwrap_or((quote!(()), quote!(actlet::Signal)));

    let gen = quote! {
        impl #impl_generics actlet::Message for #ty_name #ty_generics #where_clause {
            type Reply = #reply_type;
        }

        impl #impl_generics #message_trait for #ty_name #ty_generics #where_clause {}
    };

    gen.into()
}
