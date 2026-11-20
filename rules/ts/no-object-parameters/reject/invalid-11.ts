type Identity<T> = T; type Wrapped<T> = Identity<T>; function consume(value: Wrapped<object>) {}
