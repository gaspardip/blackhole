type NonNullable<T> = { value: T }; type A = Record<string, NonNullable<unknown>>;
