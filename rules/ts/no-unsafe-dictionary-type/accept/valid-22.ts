interface Owner { readonly id: string } type A = Record<string, object & Owner>;
