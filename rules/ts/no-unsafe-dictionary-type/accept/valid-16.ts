type Value<T> = T; type Index<T = Command, U = Value<T>> = Record<string, U>; type A = Index;
