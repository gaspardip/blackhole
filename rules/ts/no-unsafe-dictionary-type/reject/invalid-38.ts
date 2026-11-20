type Value<T> = T; type Index<T, U = Value<T>> = Record<string, U>; type A = Index<unknown>;
