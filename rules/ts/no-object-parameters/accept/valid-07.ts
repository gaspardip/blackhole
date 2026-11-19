type Owner = { readonly id: string }; function f<Value extends Owner>(value: Value) {}
