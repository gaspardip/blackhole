interface Owner { readonly id: string } interface Child extends Owner {} type A = Record<string, Child>;
