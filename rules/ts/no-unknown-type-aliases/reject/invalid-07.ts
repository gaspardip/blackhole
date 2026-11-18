type Identity<T> = T; type Wrapped<T> = Identity<T>; type Payload = Wrapped<unknown>;
