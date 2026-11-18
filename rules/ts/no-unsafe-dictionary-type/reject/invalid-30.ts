type Wrapped<T> = Readonly<T>; type A = Record<string, Wrapped<unknown>>;
