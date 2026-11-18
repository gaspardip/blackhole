type Payload = object; function outer() { type Payload = { readonly id: string }; function consume(value: Payload) {} }
