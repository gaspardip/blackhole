interface Owner { readonly id: string } type A = Record<string, unknown & Owner>;
