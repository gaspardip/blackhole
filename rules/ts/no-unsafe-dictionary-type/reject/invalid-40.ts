function outer() { type Identity<T> = T; type A = Record<string, Identity<unknown>>; }
