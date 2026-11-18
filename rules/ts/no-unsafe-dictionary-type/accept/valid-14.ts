type Readonly<T> = { value: T }; type A = Record<string, Readonly<unknown>>;
